//! The migrations of the module's own schema.

use sqlx::PgPool;
use sqlx::migrate::Migrator;

use crate::error::SettingsError;

/// The migrations built into the module, with the module's own schema and its own table of
/// history.
fn migrator() -> Migrator {
    let mut migrator = sqlx::migrate!("./migrations");
    migrator.create_schema("zgm_settings");
    // The history lies in the module's schema, not in the `_sqlx_migrations` every program
    // with `sqlx` shares by default: there the migrations of the application and of other
    // modules would be taken for missing ones of this module, and the other way round.
    //
    // The name is set from the first deployment and never changes: after a change the
    // migrator would forget which migrations it has already run.
    migrator.dangerous_set_table_name("zgm_settings._sqlx_migrations");
    migrator
}

/// Creates or updates the tables of the module, in the schema `zgm_settings`. The application
/// calls it when it is deployed or when it starts, before it uses anything else of the
/// module.
///
/// Safe to call again and from several copies of the application at once: a migration that
/// already ran is passed over, and the copies wait for each other.
///
/// The account of `pool` needs the right to create a schema in the database the first time,
/// and owns the tables.
pub async fn migrate(pool: &PgPool) -> Result<(), SettingsError> {
    migrator().run(pool).await?;
    Ok(())
}
