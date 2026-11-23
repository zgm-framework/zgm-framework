//! All SQL of the module: plain functions, no trait and no state.

use sqlx::PgConnection;
use uuid::Uuid;

use crate::error::SettingsError;
use crate::value::Json;

/// The columns a [`StoredValue`] is read from, the same in every query that reads one. `#>>
/// '{}'` gives SQL NULL for a JSON null, which is no kind the module knows either, so it
/// becomes the text `null`.
macro_rules! stored_value_columns {
    () => {
        "jsonb_typeof(value), coalesce(value #>> '{}', 'null'), value::text"
    };
}

/// A value as the table holds it, taken apart by the database, so that the module needs no
/// JSON of its own.
pub(crate) struct StoredValue {
    /// What `jsonb_typeof` says: `number`, `boolean`, `string`, `object`, `array` or `null`.
    pub(crate) json_type: String,
    /// A number, `true` or `false` as written, a string without its quotes and escapes, and
    /// anything else as its JSON, `null` too.
    pub(crate) text: String,
    /// The JSON as the database writes it, for an error.
    pub(crate) json: String,
}

/// The value one tenant set for one setting, or `None` when the tenant has no row for it.
pub(crate) async fn select_value(
    connection: &mut PgConnection,
    tenant_id: Uuid,
    name: &str,
) -> Result<Option<StoredValue>, SettingsError> {
    let row: Option<(String, String, String)> = sqlx::query_as(concat!(
        "SELECT ",
        stored_value_columns!(),
        " FROM zgm_settings.tenant_settings WHERE tenant_id = $1 AND name = $2"
    ))
    .bind(tenant_id)
    .bind(name)
    .fetch_optional(connection)
    .await?;
    Ok(row.map(|(json_type, text, json)| StoredValue {
        json_type,
        text,
        json,
    }))
}

/// Writes the value of a setting of a tenant, or changes the one it had, and says whether
/// anything changed: the same value again writes nothing, so `updated_at` stays.
pub(crate) async fn upsert_value(
    connection: &mut PgConnection,
    tenant_id: Uuid,
    name: &str,
    json: &Json<'_>,
) -> Result<bool, SettingsError> {
    let (literal, text) = match json {
        Json::Literal(literal) => (Some(literal.as_str()), None),
        Json::Text(text) => (None, Some(*text)),
    };
    let written = sqlx::query(
        "INSERT INTO zgm_settings.tenant_settings (tenant_id, name, value) \
         VALUES ($1, $2, coalesce($3::jsonb, to_jsonb($4::text))) \
         ON CONFLICT (tenant_id, name) DO UPDATE SET value = excluded.value \
         WHERE tenant_settings.value IS DISTINCT FROM excluded.value",
    )
    .bind(tenant_id)
    .bind(name)
    .bind(literal)
    .bind(text)
    .execute(connection)
    .await?;
    Ok(written.rows_affected() == 1)
}

/// Deletes the row of a setting of a tenant, and says whether there was one.
pub(crate) async fn delete_value(
    connection: &mut PgConnection,
    tenant_id: Uuid,
    name: &str,
) -> Result<bool, SettingsError> {
    let deleted =
        sqlx::query("DELETE FROM zgm_settings.tenant_settings WHERE tenant_id = $1 AND name = $2")
            .bind(tenant_id)
            .bind(name)
            .execute(connection)
            .await?;
    Ok(deleted.rows_affected() == 1)
}

/// All the values one tenant set, by the names of their settings, read through the primary
/// key, whose first column is the tenant.
pub(crate) async fn select_values(
    connection: &mut PgConnection,
    tenant_id: Uuid,
) -> Result<Vec<(String, StoredValue)>, SettingsError> {
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(concat!(
        "SELECT name, ",
        stored_value_columns!(),
        " FROM zgm_settings.tenant_settings WHERE tenant_id = $1"
    ))
    .bind(tenant_id)
    .fetch_all(connection)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(name, json_type, text, json)| {
            (
                name,
                StoredValue {
                    json_type,
                    text,
                    json,
                },
            )
        })
        .collect())
}
