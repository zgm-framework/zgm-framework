//! The entry point of the module.

use std::collections::BTreeMap;

use sqlx::PgPool;
use uuid::Uuid;

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
