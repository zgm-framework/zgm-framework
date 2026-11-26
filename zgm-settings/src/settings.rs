//! The entry point of the module.

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};

use sqlx::PgPool;
use uuid::Uuid;

use crate::default_setting::DefaultSetting;
use crate::error::SettingsError;
use crate::repository;
use crate::tenant_scope::TenantScope;
use crate::value::Value;

/// The module: the settings of the tenants, with the defaults the modules gave.
///
/// ```ignore
/// let settings = Settings::builder().pool(pool).build();
/// // A module, e.g. IAM, does this itself when it starts:
/// let window = DefaultSetting::Duration { value: thirty_days, min: Some(a_day), max: None };
/// settings.add_default(IamSetting::UserDeletionWindow, window).await?;
/// settings.defaults(); // {"iam.user_deletion_window": Duration(30 days)}
/// ```
///
/// The application builds it once, when it starts, after [`migrate`](crate::migrate), and
/// gives clones of it to the modules that have settings. The clones share the defaults: what
/// a module gives through its clone, the application sees through its own.
#[derive(Debug, Clone)]
pub struct Settings {
    pool: PgPool,
    /// The defaults by the names of their settings, as the database had them when the
    /// modules gave them.
    defaults: Arc<RwLock<BTreeMap<String, Value>>>,
}

#[bon::bon]
impl Settings {
    /// Builds the module on the application's pool of connections. It has no defaults until
    /// the modules give them.
    #[builder]
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            defaults: Arc::default(),
        }
    }

    /// Gives the default of a setting, with its bounds: the value of every tenant that has
    /// not changed it. A module gives its defaults when it starts.
    ///
    /// The first time, the default is written to the database. After that it is left as it
    /// is, whatever is given: a later start, also of a version of the module with other
    /// numbers, gets the default the database has. `true` means it was written now.
    ///
    /// The name is the prefix of the module and the name of the setting, e.g.
    /// `iam.user_deletion_window`. A module names its settings with an enum that derives
    /// `strum::AsRefStr` with `serialize_all = "snake_case"` and its `prefix`; plain text
    /// works too. Nothing given is checked ([`DefaultSetting`]).
    ///
    /// An error when the database fails, and [`SettingsError::UnreadableDefault`] for a row
    /// in the database that SQL by hand left unreadable; the default is then not kept.
    pub async fn add_default(
        &self,
        name: impl AsRef<str>,
        default: DefaultSetting,
    ) -> Result<bool, SettingsError> {
        let name = name.as_ref();
        let given = default.value();
        let (min, max) = default.bounds();
        let mut connection = self.pool.acquire().await?;
        let written = repository::insert_default(
            &mut connection,
            name,
            given.kind(),
            &given.to_json_in_whole_seconds(),
            min,
            max,
        )
        .await?;
        // What the database has, which is what was given only when it was written now. A
        // statement of its own, which sees the row another copy of the program wrote meanwhile.
        let (kind, stored) = repository::select_default(&mut connection, name)
            .await?
            .ok_or_else(|| SettingsError::UnknownSetting(name.to_owned()))?;
        let value =
            Value::read_as(&kind, &stored).ok_or_else(|| SettingsError::UnreadableDefault {
                name: name.to_owned(),
                kind: kind.clone(),
                found: stored.json.clone(),
            })?;
        // A panic of another thread cannot leave the map half changed, so a poisoned lock is
        // taken over as it is.
        self.defaults
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.to_owned(), value);
        Ok(written)
    }

    /// All the defaults given so far, as the database had them when they were given, by the
    /// names of their settings, in the order of the names. It does not ask the database.
    pub fn defaults(&self) -> BTreeMap<String, Value> {
        self.defaults
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The module within one tenant.
    pub fn tenant_id(&self, tenant_id: Uuid) -> TenantScope {
        TenantScope::new(self.clone(), tenant_id)
    }

    /// The pool of connections, for the functions of one tenant.
    pub(crate) fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// The default of one setting, or `None` when no module gave one.
    pub(crate) fn default_of(&self, name: &str) -> Option<Value> {
        self.defaults
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .cloned()
    }
}
