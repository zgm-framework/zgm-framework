//! What can go wrong in the module.

use crate::value::Value;

/// An error of the module.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// The database failed.
    #[error("the database failed: {0}")]
    Database(#[from] sqlx::Error),

    /// A migration of the module's schema failed.
    #[error("the migration failed: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),

    /// The setting of this name has no default in the database, so the module does not know
    /// the setting, nor of which kind its value is. A mistake in the program, not in the data.
    #[error("no default was given for the setting {0}")]
    UnknownSetting(String),

    /// A value of another kind than the default of its setting, e.g. a text for a duration:
    /// given to [`set`](crate::TenantScope::set) or as a new default to
    /// [`change_default`](crate::Settings::change_default), which then write nothing, or held
    /// by the row of a tenant, which only SQL written by hand can do.
    #[error("the value of the setting {name} is not of the kind of its default: {found}")]
    WrongKind {
        /// The name of the setting.
        name: String,
        /// What was found: the JSON the row holds, or the value given to `set` or
        /// `change_default`, as Rust shows it.
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

    /// A value below the least its setting allows was given to
    /// [`set`](crate::TenantScope::set), or a default below the least given with it to
    /// [`change_default`](crate::Settings::change_default); nothing is written.
    #[error("the value for the setting {name} is below the least it allows, {min:?}")]
    BelowTheLeast {
        /// The name of the setting.
        name: String,
        /// The least, of the kind of the setting, e.g. a duration of a day.
        min: Value,
    },

    /// A value above the most its setting allows was given to
    /// [`set`](crate::TenantScope::set), or a default above the most given with it to
    /// [`change_default`](crate::Settings::change_default); nothing is written.
    #[error("the value for the setting {name} is above the most it allows, {max:?}")]
    AboveTheMost {
        /// The name of the setting.
        name: String,
        /// The most, of the kind of the setting, e.g. a duration of a year.
        max: Value,
    },

    /// A least above the most was given to
    /// [`change_default`](crate::Settings::change_default), which then writes nothing. A least
    /// equal to the most is allowed: the setting then has one value.
    #[error("the least for the setting {name}, {min:?}, is above the most, {max:?}")]
    LeastAboveTheMost {
        /// The name of the setting.
        name: String,
        /// The least given, of the kind of the setting.
        min: Value,
        /// The most given, of the kind of the setting.
        max: Value,
    },

    /// A duration with a fraction of a second was given to
    /// [`set`](crate::TenantScope::set), or a default or a bound with one to
    /// [`change_default`](crate::Settings::change_default). The table keeps whole seconds, so
    /// it would keep another value than the one given; nothing is written.
    #[error("the duration for the setting {0} has a fraction of a second")]
    FractionOfASecond(String),
}
