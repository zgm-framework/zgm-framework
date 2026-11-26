//! What the module does within one tenant.

use std::collections::HashMap;

use uuid::Uuid;

use crate::error::SettingsError;
use crate::repository::{self, StoredValue};
use crate::setting::Setting;
use crate::settings::Settings;
use crate::value::Value;

/// The module within one tenant: every function of a tenant hangs from here, so none can be
/// called without the tenant, and none reaches past it.
///
/// ```ignore
/// let in_nowak = settings.tenant_id(nowak.id);
/// let window = in_nowak.get(IamSetting::UserDeletionWindow).await?;
/// ```
///
/// [`Settings::tenant_id`] gives it without asking the database, and the module does not know
/// which tenants exist: a tenant it never heard of has the defaults. It holds its own copy of
/// [`Settings`], so an application with one tenant can make it once, when it starts, and
/// clone it where it needs it.
#[derive(Debug, Clone)]
pub struct TenantScope {
    settings: Settings,
    tenant_id: Uuid,
}

impl TenantScope {
    pub(crate) fn new(settings: Settings, tenant_id: Uuid) -> Self {
        Self {
            settings,
            tenant_id,
        }
    }

    /// The value of a setting in the tenant: the one the tenant set, or, when it set none, the
    /// default. Both are read from the database every time, in one statement, so a default
    /// changed there is seen at once, by every copy of the program.
    ///
    /// The value of the tenant is given as it is, also when it lies outside the bounds of the
    /// default: the bounds hold for the values set after they were.
    ///
    /// A name without a default in the database is [`SettingsError::UnknownSetting`]. A row the
    /// module cannot read, which only SQL written by hand can write, is an error, not the
    /// default: [`SettingsError::UnreadableDefault`] for the row of the default, and
    /// [`SettingsError::WrongKind`] for a value of the tenant of another kind than the default.
    pub async fn get(&self, name: impl AsRef<str>) -> Result<Value, SettingsError> {
        let name = name.as_ref();
        let mut connection = self.settings.pool().acquire().await?;
        let (kind, default, of_the_tenant) =
            repository::select_setting(&mut connection, self.tenant_id, name)
                .await?
                .ok_or_else(|| SettingsError::UnknownSetting(name.to_owned()))?;
        let default =
            Value::read_as(&kind, &default).ok_or_else(|| SettingsError::UnreadableDefault {
                name: name.to_owned(),
                kind,
                found: default.json.clone(),
            })?;
        match of_the_tenant {
            None => Ok(default),
            Some(stored) => default
                .read_like(&stored)
                .ok_or_else(|| SettingsError::WrongKind {
                    name: name.to_owned(),
                    found: stored.json,
                }),
        }
    }

    /// Sets the value of a setting in the tenant, and says whether anything changed: `false`
    /// when the tenant had that value already, and then not even `updated_at` moves.
    ///
    /// A value equal to the default is kept as a row of the tenant too: when a program
    /// changes the default later, the tenant keeps its own. Going back to the default is
    /// another function.
    ///
    /// The kind and the bounds are those of the row of the default in the database, read at
    /// every call. A bound holds as it is now, also for the value the tenant has already: the
    /// same value again, set while it lies outside the bounds, is refused.
    ///
    /// Nothing is written for a name without a default in the database
    /// ([`SettingsError::UnknownSetting`]), for a row of the default the module cannot read
    /// ([`SettingsError::UnreadableDefault`]), for a value of another kind than the default
    /// ([`SettingsError::WrongKind`]), for a duration with a fraction of a second
    /// ([`SettingsError::FractionOfASecond`]), or for a duration or a number below the least
    /// or above the most ([`SettingsError::BelowTheLeast`], [`SettingsError::AboveTheMost`]);
    /// the bounds themselves are allowed. A flag and a text have no bounds.
    pub async fn set(&self, name: impl AsRef<str>, value: Value) -> Result<bool, SettingsError> {
        let name = name.as_ref();
        let mut connection = self.settings.pool().acquire().await?;
        let row = repository::select_default(&mut connection, name)
            .await?
            .ok_or_else(|| SettingsError::UnknownSetting(name.to_owned()))?;
        let default = row.read(name)?;
        if !value.is_of_the_kind_of(&default) {
            return Err(SettingsError::WrongKind {
                name: name.to_owned(),
                found: format!("{value:?}"),
            });
        }
        let json = value
            .to_json()
            .ok_or_else(|| SettingsError::FractionOfASecond(name.to_owned()))?;
        within_the_bounds(name, &value, row.min, row.max)?;
        repository::upsert_value(&mut connection, self.tenant_id, name, &json).await
    }

    /// Brings the tenant back to the default of a setting, by deleting its row, and says
    /// whether there was a row: `false` when the tenant had the default already.
    ///
    /// The value of the row is not read, so a row of another kind than the default, which
    /// only SQL written by hand can write and [`get`](Self::get) refuses, goes too.
    ///
    /// The setting is known when its default is in the database, also when this program did
    /// not give it, and the default is not read either: a row of a tenant goes also when the
    /// default cannot be read. One statement deletes the row and finds the default. A name
    /// without a default in the database is [`SettingsError::UnknownSetting`].
    pub async fn restore_default(&self, name: impl AsRef<str>) -> Result<bool, SettingsError> {
        let name = name.as_ref();
        let mut connection = self.settings.pool().acquire().await?;
        let (known, deleted) =
            repository::delete_value(&mut connection, self.tenant_id, name).await?;
        if !known {
            return Err(SettingsError::UnknownSetting(name.to_owned()));
        }
        Ok(deleted)
    }

    /// All the settings of the tenant, in the order of their names: every setting a module
    /// gave a default for, with its value in the tenant, the default, and whether the tenant
    /// has a value of its own. One query reads all the rows of the tenant.
    ///
    /// The row of a name no module gives a default for is left out. A row with a value of
    /// another kind than the default, which only SQL written by hand can write, makes the
    /// whole list [`SettingsError::WrongKind`], as in [`get`](Self::get).
    pub async fn list(&self) -> Result<Vec<Setting>, SettingsError> {
        let defaults = self.settings.defaults();
        let mut connection = self.settings.pool().acquire().await?;
        let mut rows: HashMap<String, StoredValue> =
            repository::select_values(&mut connection, self.tenant_id)
                .await?
                .into_iter()
                .collect();
        defaults
            .into_iter()
            .map(|(name, default)| match rows.remove(&name) {
                None => Ok(Setting {
                    name,
                    value: default.clone(),
                    default,
                    overridden: false,
                }),
                Some(stored) => match default.read_like(&stored) {
                    Some(value) => Ok(Setting {
                        name,
                        value,
                        default,
                        overridden: true,
                    }),
                    None => Err(SettingsError::WrongKind {
                        name,
                        found: stored.json,
                    }),
                },
            })
            .collect()
    }
}

/// Refuses a duration or a number below `min` or above `max`, a duration in whole seconds;
/// `None` is no bound, and a flag or a text has none.
fn within_the_bounds(
    name: &str,
    value: &Value,
    min: Option<i64>,
    max: Option<i64>,
) -> Result<(), SettingsError> {
    let Some(number) = value.as_bounded() else {
        return Ok(());
    };
    if let Some(min) = min
        && number < min
    {
        return Err(SettingsError::BelowTheLeast {
            name: name.to_owned(),
            min: value.bound_like(min),
        });
    }
    if let Some(max) = max
        && number > max
    {
        return Err(SettingsError::AboveTheMost {
            name: name.to_owned(),
            max: value.bound_like(max),
        });
    }
    Ok(())
}
