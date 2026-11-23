//! The entry point of the module.

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};

use sqlx::PgPool;
use uuid::Uuid;

use crate::tenant_scope::TenantScope;
use crate::value::Value;

/// The module: the settings of the tenants, with the defaults the modules gave.
///
/// ```ignore
/// let settings = Settings::builder().pool(pool).build();
/// // A module, e.g. IAM, does this itself when it is built:
/// settings.add_default(IamSetting::UserDeletionWindow, Value::Duration(thirty_days));
/// settings.defaults(); // {"iam.user_deletion_window": Duration(30 days)}
/// ```
///
/// The application builds it once, when it starts, after [`migrate`](crate::migrate), and
/// gives clones of it to the modules that have settings. The clones share the defaults: what
/// a module gives through its clone, the application sees through its own.
#[derive(Debug, Clone)]
pub struct Settings {
    pool: PgPool,
    /// The defaults by the names of their settings. Kept in memory only: nothing of them is
    /// written to the database, so two versions of a program running side by side during a
    /// deployment do not overwrite each other's.
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

    /// Gives the default of a setting: the value of every tenant that has not changed it.
    ///
    /// The name is the prefix of the module and the name of the setting, e.g.
    /// `iam.user_deletion_window`. A module names its settings with an enum that derives
    /// `strum::AsRefStr` with `serialize_all = "snake_case"` and its `prefix`; plain text
    /// works too.
    ///
    /// A default given again under the same name replaces the one before.
    pub fn add_default(&self, name: impl AsRef<str>, value: Value) {
        // A panic of another thread cannot leave the map half changed, so a poisoned lock is
        // taken over as it is.
        self.defaults
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.as_ref().to_owned(), value);
    }

    /// All the defaults given so far, by the names of their settings, in the order of the
    /// names. It does not ask the database.
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
