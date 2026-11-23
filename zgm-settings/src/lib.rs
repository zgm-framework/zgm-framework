//! `zgm-settings`: the settings of an application's tenants, kept in PostgreSQL.
//!
//! The modules give [`Settings`] the defaults of their settings when they are built
//! ([`Settings::add_default`]), and the database holds only what a tenant changed, in the
//! table [`migrate`] creates. Within one tenant ([`Settings::tenant_id`]),
//! [`TenantScope::get`] reads a setting, [`TenantScope::set`] changes it,
//! [`TenantScope::restore_default`] brings the default back, and [`TenantScope::list`] gives
//! all of them. A value is of one of the kinds of [`Value`].

mod error;
mod migrate;
mod repository;
mod setting;
mod settings;
mod tenant_scope;
mod value;

pub use error::SettingsError;
pub use migrate::migrate;
pub use setting::Setting;
pub use settings::Settings;
pub use tenant_scope::TenantScope;
pub use value::Value;
