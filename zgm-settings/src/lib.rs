//! `zgm-settings`: the settings of an application's tenants, kept in PostgreSQL.
//!
//! The modules give [`Settings`] the defaults of their settings, with their bounds, when they
//! start ([`Settings::add_default`], [`DefaultSetting`]). The first start writes each default
//! to the database, and later ones leave it there; [`Settings::change_default`] changes it
//! later, e.g. from a screen of the program. What a tenant changed is in a row of its own. [`migrate`] creates both tables. Within one tenant ([`Settings::tenant_id`]),
//! [`TenantScope::get`] reads a setting, [`TenantScope::set`] changes it,
//! [`TenantScope::restore_default`] brings the default back, and [`TenantScope::list`] gives
//! all of them. A value is of one of the kinds of [`Value`].

mod bounds;
mod default_setting;
mod error;
mod migrate;
mod repository;
mod setting;
mod settings;
mod tenant_scope;
mod value;

pub use default_setting::DefaultSetting;
pub use error::SettingsError;
pub use migrate::migrate;
pub use setting::Setting;
pub use settings::Settings;
pub use tenant_scope::TenantScope;
pub use value::Value;
