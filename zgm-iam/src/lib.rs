//! `zgm-iam`: the tenants of an application, its users and what they may do, kept in
//! PostgreSQL.
//!
//! So far the module has its schema with the tables of tenants and of users, which
//! [`migrate`] creates; [`Iam::create_tenant`], which writes a tenant with a name the
//! application's [`Cipher`] encrypts; and, within one tenant ([`Iam::tenant_id`]),
//! [`TenantScope::create_user`] and [`TenantScope::list_users`], and for one user of it
//! ([`TenantScope::user_id`]) [`UserScope::get`], [`UserScope::delete`], after the window
//! of the tenant, and [`UserScope::set_state`], which blocks, unblocks and brings a user back.
//! Both a tenant and a user have a [`State`]. The settings a tenant can change for itself,
//! [`IamSetting`], come from the application's [`TenantSettings`], which gets their defaults
//! and bounds ([`DurationDefault`]) when the module is built. A loop the module runs itself
//! deletes the users past their date ([`Job`]).

mod cipher;
mod error;
mod iam;
mod iam_setting;
mod jobs;
mod migrate;
mod order;
mod repository;
mod state;
mod tenant;
mod tenant_scope;
mod tenant_settings;
mod user;
mod user_scope;

pub use cipher::{Cipher, CipherError};
pub use error::IamError;
pub use iam::Iam;
pub use iam_setting::IamSetting;
pub use jobs::{Job, JobError};
pub use migrate::migrate;
pub use order::Order;
pub use state::State;
pub use tenant::Tenant;
pub use tenant_scope::TenantScope;
pub use tenant_settings::{DurationDefault, TenantSettings, TenantSettingsError};
pub use user::{User, UsersPage};
pub use user_scope::UserScope;
