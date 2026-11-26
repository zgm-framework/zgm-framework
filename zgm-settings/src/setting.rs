//! One setting of a tenant, as the list of all of them gives it.

use crate::value::Value;

/// One setting of a tenant: what it is in the tenant, what the default is, and whether the
/// tenant changed it. E.g. for a screen of settings that shows "90 days (default 30)" and a
/// button to restore the default next to what the tenant changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setting {
    /// The name, with the prefix of its module, e.g. `iam.user_deletion_window`.
    pub name: String,
    /// The value in the tenant: its own, or the default.
    pub value: Value,
    /// The default a module gave.
    pub default: Value,
    /// Whether the tenant has a value of its own, also one equal to the default.
    pub overridden: bool,
    /// The least a tenant may set, of the kind of the setting, or `None` for no bound. A flag
    /// and a text have none.
    pub min: Option<Value>,
    /// The most a tenant may set, of the kind of the setting, or `None` for no bound. A flag
    /// and a text have none.
    pub max: Option<Value>,
}
