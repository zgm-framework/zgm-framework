//! What can go wrong in the module.

/// An error of the module.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// The database failed.
    #[error("the database failed: {0}")]
    Database(#[from] sqlx::Error),

    /// A migration of the module's schema failed.
    #[error("the migration failed: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),

    /// No module gave a default for a setting of this name, so the module does not know the
    /// setting, nor of which kind its value is. A mistake in the program, not in the data.
    #[error("no default was given for the setting {0}")]
    UnknownSetting(String),

    /// A value of another kind than the default of its setting, e.g. a text for a duration:
    /// given to [`set`](crate::TenantScope::set), which then writes nothing, or held by the row
    /// of a tenant, which only SQL written by hand can do.
    #[error("the value of the setting {name} is not of the kind of its default: {found}")]
    WrongKind {
        /// The name of the setting.
        name: String,
        /// What was found: the JSON the row holds, or the value given to `set`, as Rust
        /// shows it.
        found: String,
    },

    /// The row of the default of a setting holds a kind the module does not know, or a value
    /// not of the kind it names, e.g. `colour` or a text for a duration. Only SQL written by
    /// hand can write such a row.
    #[error("the default of the setting {name} cannot be read: the kind {kind}, the value {found}")]
    UnreadableDefault {
        /// The name of the setting.
        name: String,
        /// The word in the column `kind`.
        kind: String,
        /// The JSON the row holds.
        found: String,
    },

    /// A duration with a fraction of a second was given to
    /// [`set`](crate::TenantScope::set). The table keeps whole seconds, so it would keep
    /// another value than the one given; nothing is written.
    #[error("the duration for the setting {0} has a fraction of a second")]
    FractionOfASecond(String),
}
