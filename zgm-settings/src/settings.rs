//! The entry point of the module.

use std::collections::BTreeMap;

use sqlx::PgPool;
use uuid::Uuid;

use crate::bounds;
use crate::default_setting::DefaultSetting;
use crate::error::SettingsError;
use crate::repository;
use crate::tenant_scope::TenantScope;
use crate::value::Value;

/// The module: the settings of the tenants, with the defaults the modules gave, all of them
/// in the database.
///
/// ```ignore
/// let settings = Settings::builder().pool(pool).build();
/// // A module, e.g. IAM, does this itself when it starts:
/// let window = DefaultSetting::Duration { value: thirty_days, min: Some(a_day), max: None };
/// settings.add_default(IamSetting::UserDeletionWindow, window).await?;
/// settings.defaults().await?; // {"iam.user_deletion_window": Duration(30 days)}
/// ```
///
/// The application builds it once, when it starts, after [`migrate`](crate::migrate), and
/// gives clones of it to the modules that have settings. It keeps nothing but the pool: every
/// function reads the database, so every clone and every copy of the program sees the same.
#[derive(Debug, Clone)]
pub struct Settings {
    pool: PgPool,
}

#[bon::bon]
impl Settings {
    /// Builds the module on the application's pool of connections.
    #[builder]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Gives the default of a setting, with its bounds: the value of every tenant that has
    /// not changed it. A module gives its defaults when it starts.
    ///
    /// The first time, the default is written to the database. After that it is left as it
    /// is, whatever is given: a later start, also of a version of the module with other
    /// numbers, leaves the default the database has. `true` means it was written now. One
    /// statement; two copies of a program giving a default at once write one row.
    ///
    /// The name is the prefix of the module and the name of the setting, e.g.
    /// `iam.user_deletion_window`. A module names its settings with an enum that derives
    /// `strum::AsRefStr` with `serialize_all = "snake_case"` and its `prefix`; plain text
    /// works too. Nothing given is checked ([`DefaultSetting`]), and a row that is there is
    /// not read.
    ///
    /// An error when the database fails, e.g. for a text with a NUL, which PostgreSQL does not
    /// keep; nothing is written then.
    pub async fn add_default(
        &self,
        name: impl AsRef<str>,
        default: DefaultSetting,
    ) -> Result<bool, SettingsError> {
        let given = default.value();
        let (min, max) = default.bounds();
        let mut connection = self.pool.acquire().await?;
        repository::insert_default(
            &mut connection,
            name.as_ref(),
            given.kind(),
            &given.to_json_in_whole_seconds(),
            min,
            max,
        )
        .await
    }

    /// Changes the default of a setting and its bounds, e.g. when a person changes them on a
    /// screen of the program, and says whether anything changed: `false` when the database
    /// had that default and those bounds already, and then not even `updated_at` moves.
    ///
    /// ```ignore
    /// // 14 days instead of 30, at least a week, at most 90 days.
    /// let window = DefaultSetting::Duration { value: two_weeks, min: Some(a_week), max: Some(ninety_days) };
    /// settings.change_default(IamSetting::UserDeletionWindow, window).await?; // true
    /// ```
    ///
    /// The change is seen at once by every copy of the program: a tenant without a value of
    /// its own gets the new default, and the next [`set`](crate::TenantScope::set) of any
    /// tenant is held against the new bounds. A value a tenant set before is left as it is,
    /// also outside the new bounds. A later start of a module leaves the change, as
    /// [`add_default`](Self::add_default) does not write over a default that is there.
    ///
    /// Nothing is written for a name without a default in the database
    /// ([`SettingsError::UnknownSetting`]), for a default of another kind than the one in the
    /// database ([`SettingsError::WrongKind`]), for a duration or a bound with a fraction of a
    /// second ([`SettingsError::FractionOfASecond`]), for a least above the most
    /// ([`SettingsError::LeastAboveTheMost`]), or for a default below its least or above its
    /// most ([`SettingsError::BelowTheLeast`], [`SettingsError::AboveTheMost`]). A least equal
    /// to the most, and a default on a bound, are allowed. A default the module cannot read
    /// is written over when the kind given is the one its row names.
    pub async fn change_default(
        &self,
        name: impl AsRef<str>,
        default: DefaultSetting,
    ) -> Result<bool, SettingsError> {
        let name = name.as_ref();
        let given = default.value();
        let mut connection = self.pool.acquire().await?;
        let row = repository::select_default(&mut connection, name)
            .await?
            .ok_or_else(|| SettingsError::UnknownSetting(name.to_owned()))?;
        if row.kind != given.kind() {
            return Err(SettingsError::WrongKind {
                name: name.to_owned(),
                found: format!("{given:?}"),
            });
        }
        if default.has_a_fraction_of_a_second() {
            return Err(SettingsError::FractionOfASecond(name.to_owned()));
        }
        let (min, max) = default.bounds();
        bounds::in_order(name, &given, min, max)?;
        bounds::within(name, &given, min, max)?;
        repository::update_default(
            &mut connection,
            name,
            &given.to_json_in_whole_seconds(),
            min,
            max,
        )
        .await
    }

    /// Every default in the database, by the names of their settings, in the order of the
    /// names, also of settings this program did not give. Read at every call.
    ///
    /// [`SettingsError::UnreadableDefault`] for a row the module cannot read, which only SQL
    /// written by hand can write.
    pub async fn defaults(&self) -> Result<BTreeMap<String, Value>, SettingsError> {
        let mut connection = self.pool.acquire().await?;
        repository::select_defaults(&mut connection)
            .await?
            .into_iter()
            .map(|(name, row)| row.read(&name).map(|value| (name, value)))
            .collect()
    }

    /// The module within one tenant.
    pub fn tenant_id(&self, tenant_id: Uuid) -> TenantScope {
        TenantScope::new(self.clone(), tenant_id)
    }

    /// The pool of connections, for the functions of one tenant.
    pub(crate) fn pool(&self) -> &PgPool {
        &self.pool
    }
}
