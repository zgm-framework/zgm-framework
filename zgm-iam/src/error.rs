//! What can go wrong in the module.

use crate::cipher::CipherError;
use crate::state::State;
use crate::tenant_settings::TenantSettingsError;

/// An error of the module.
#[derive(Debug, thiserror::Error)]
pub enum IamError {
    /// The database failed.
    #[error("the database failed: {0}")]
    Database(#[from] sqlx::Error),

    /// A migration of the module's schema failed.
    #[error("the migration failed: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),

    /// The application's [`Cipher`](crate::Cipher) failed.
    #[error("the cipher failed: {0}")]
    Cipher(#[from] CipherError),

    /// The application's [`TenantSettings`](crate::TenantSettings) failed.
    #[error("the settings of tenants failed: {0}")]
    Settings(#[from] TenantSettingsError),

    /// The deletion window of the tenant puts the date of deletion past the latest date the
    /// module can hold, the end of the year 9999; nothing is changed.
    #[error("the date of deletion would be past the end of the year 9999")]
    DeletionDateOutOfRange,

    /// The application's [`Cipher`](crate::Cipher) gave empty bytes. A ciphertext is never
    /// empty, so the cipher is broken; with these bytes the tenant would have no name.
    #[error("the cipher gave empty bytes")]
    EmptyCiphertext,

    /// The application's [`Cipher`](crate::Cipher) decrypted the name of a tenant into bytes
    /// that are not UTF-8 text, e.g. with the key of another application; nothing is changed.
    #[error("the decrypted name of the tenant is not UTF-8 text")]
    UnreadableName,

    /// A tenant was given an empty name. Every tenant has a name.
    #[error("the name of a tenant is empty")]
    EmptyName,

    /// The database gave a word that is no [`State`](crate::State). Only SQL written by hand
    /// can put one there, since the module's code writes only its own states.
    #[error("the database holds a state the module does not know: {0}")]
    UnknownState(String),

    /// There is no tenant with the id given to [`Iam::tenant_id`](crate::Iam::tenant_id).
    #[error("there is no such tenant")]
    NoSuchTenant,

    /// [`UserScope::set_state`](crate::UserScope::set_state) was given a state it does not
    /// set: a user waits for deletion only through [`UserScope::delete`](crate::UserScope::delete),
    /// which gives it its date. Nothing is changed.
    #[error("the state {0:?} is not set directly; a user waits for deletion through delete")]
    NotSettable(State),

    /// The tenant is suspended or waits for deletion, so nothing can be done in it; the
    /// error carries its state.
    #[error("the tenant is not active: {0:?}")]
    TenantNotActive(State),
}
