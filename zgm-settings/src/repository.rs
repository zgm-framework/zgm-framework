//! All SQL of the module: plain functions, no trait and no state.

use sqlx::PgConnection;
use uuid::Uuid;

use crate::error::SettingsError;
use crate::value::{Json, Value};

/// The columns a [`StoredValue`] is read from, the same in every query that reads one, of
/// the column `value` with the given prefix of its table, e.g. `"d."`. `#>> '{}'` gives SQL
/// NULL for a JSON null, which is no kind the module knows either, so it becomes the text
/// `null`.
macro_rules! stored_value_columns {
    () => {
        stored_value_columns!("")
    };
    ($table:literal) => {
        concat!(
            "jsonb_typeof(",
            $table,
            "value), coalesce(",
            $table,
            "value #>> '{}', 'null'), ",
            $table,
            "value::text"
        )
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

/// The JSON of a value as the two parameters a query binds: the text of a JSON literal, or a
/// text the database turns into a JSON string. One of them is `None`.
fn parameters<'a>(json: &'a Json<'_>) -> (Option<&'a str>, Option<&'a str>) {
    match json {
        Json::Literal(literal) => (Some(literal.as_str()), None),
        Json::Text(text) => (None, Some(*text)),
    }
}

/// Writes the default of a setting unless the setting has one already, and says whether it
/// wrote it: a default once written is left as it is, whatever is given later. Two copies of
/// a program writing at once write one row; the other waits for it and writes nothing.
pub(crate) async fn insert_default(
    connection: &mut PgConnection,
    name: &str,
    kind: &str,
    json: &Json<'_>,
    min: Option<i64>,
    max: Option<i64>,
) -> Result<bool, SettingsError> {
    let (literal, text) = parameters(json);
    let written = sqlx::query(
        "INSERT INTO zgm_settings.default_settings (name, kind, value, min, max) \
         VALUES ($1, $2, coalesce($3::jsonb, to_jsonb($4::text)), $5, $6) \
         ON CONFLICT (name) DO NOTHING",
    )
    .bind(name)
    .bind(kind)
    .bind(literal)
    .bind(text)
    .bind(min)
    .bind(max)
    .execute(connection)
    .await?;
    Ok(written.rows_affected() == 1)
}

/// The row of the default of a setting.
pub(crate) struct DefaultRow {
    /// The word of its kind.
    pub(crate) kind: String,
    /// The default.
    pub(crate) value: StoredValue,
    /// The least and the most, a duration in whole seconds; `None` is no bound.
    pub(crate) min: Option<i64>,
    pub(crate) max: Option<i64>,
}

impl DefaultRow {
    /// The default, read as the kind the row names; [`SettingsError::UnreadableDefault`] for a
    /// kind the module does not know, or a value not of that kind.
    pub(crate) fn read(&self, name: &str) -> Result<Value, SettingsError> {
        Value::read_as(&self.kind, &self.value).ok_or_else(|| SettingsError::UnreadableDefault {
            name: name.to_owned(),
            kind: self.kind.clone(),
            found: self.value.json.clone(),
        })
    }
}

/// The row of the default of a setting, or `None` without one.
pub(crate) async fn select_default(
    connection: &mut PgConnection,
    name: &str,
) -> Result<Option<DefaultRow>, SettingsError> {
    type Row = (String, String, String, String, Option<i64>, Option<i64>);
    let row: Option<Row> = sqlx::query_as(concat!(
        "SELECT kind, ",
        stored_value_columns!(),
        ", min, max FROM zgm_settings.default_settings WHERE name = $1"
    ))
    .bind(name)
    .fetch_optional(connection)
    .await?;
    Ok(
        row.map(|(kind, json_type, text, json, min, max)| DefaultRow {
            kind,
            value: StoredValue {
                json_type,
                text,
                json,
            },
            min,
            max,
        }),
    )
}

/// The value of a tenant from the columns of a `LEFT JOIN`, or `None` without a row. Without a
/// row every column of it is NULL, but `coalesce` makes its text `null`; the JSON itself is
/// NULL only then.
fn stored_of_the_tenant(
    json_type: Option<String>,
    text: Option<String>,
    json: Option<String>,
) -> Option<StoredValue> {
    match (json_type, text, json) {
        (Some(json_type), Some(text), Some(json)) => Some(StoredValue {
            json_type,
            text,
            json,
        }),
        _ => None,
    }
}

/// A setting in one tenant: the kind and the value of its default, and the value the tenant
/// set, if it set one; `None` when the setting has no default. One statement reads both, through
/// the primary key of each table.
pub(crate) async fn select_setting(
    connection: &mut PgConnection,
    tenant_id: Uuid,
    name: &str,
) -> Result<Option<(String, StoredValue, Option<StoredValue>)>, SettingsError> {
    type Row = (
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let row: Option<Row> = sqlx::query_as(concat!(
        "SELECT d.kind, ",
        stored_value_columns!("d."),
        ", ",
        stored_value_columns!("t."),
        " FROM zgm_settings.default_settings d \
         LEFT JOIN zgm_settings.tenant_settings t ON t.tenant_id = $1 AND t.name = d.name \
         WHERE d.name = $2"
    ))
    .bind(tenant_id)
    .bind(name)
    .fetch_optional(connection)
    .await?;
    Ok(row.map(
        |(kind, json_type, text, json, of_the_tenant_type, of_the_tenant_text, of_the_tenant)| {
            let default = StoredValue {
                json_type,
                text,
                json,
            };
            let of_the_tenant =
                stored_of_the_tenant(of_the_tenant_type, of_the_tenant_text, of_the_tenant);
            (kind, default, of_the_tenant)
        },
    ))
}

/// Writes the value of a setting of a tenant, or changes the one it had, and says whether
/// anything changed: the same value again writes nothing, so `updated_at` stays.
pub(crate) async fn upsert_value(
    connection: &mut PgConnection,
    tenant_id: Uuid,
    name: &str,
    json: &Json<'_>,
) -> Result<bool, SettingsError> {
    let (literal, text) = parameters(json);
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

/// Deletes the row of a setting of a tenant, in one statement that also says whether the
/// setting has a default: `(has a default, there was a row)`. Without a default there is no
/// row either, as the foreign key keeps none.
pub(crate) async fn delete_value(
    connection: &mut PgConnection,
    tenant_id: Uuid,
    name: &str,
) -> Result<(bool, bool), SettingsError> {
    let answer = sqlx::query_as(
        "WITH deleted AS ( \
             DELETE FROM zgm_settings.tenant_settings WHERE tenant_id = $1 AND name = $2 \
             RETURNING 1 \
         ) \
         SELECT EXISTS (SELECT FROM zgm_settings.default_settings WHERE name = $2), \
                EXISTS (SELECT FROM deleted)",
    )
    .bind(tenant_id)
    .bind(name)
    .fetch_one(connection)
    .await?;
    Ok(answer)
}

/// The columns of a row of the defaults read into a [`DefaultRow`], after its name, with the
/// prefix `d.` of the table.
macro_rules! default_row_columns {
    () => {
        concat!(
            "d.name, d.kind, ",
            stored_value_columns!("d."),
            ", d.min, d.max"
        )
    };
}

/// A row of the defaults as it is read, after its name.
type DefaultColumns = (
    String,
    String,
    String,
    String,
    String,
    Option<i64>,
    Option<i64>,
);

impl DefaultRow {
    fn from_columns(
        (name, kind, json_type, text, json, min, max): DefaultColumns,
    ) -> (String, DefaultRow) {
        (
            name,
            DefaultRow {
                kind,
                value: StoredValue {
                    json_type,
                    text,
                    json,
                },
                min,
                max,
            },
        )
    }
}

/// Every row of the defaults, in no order.
pub(crate) async fn select_defaults(
    connection: &mut PgConnection,
) -> Result<Vec<(String, DefaultRow)>, SettingsError> {
    let rows: Vec<DefaultColumns> = sqlx::query_as(concat!(
        "SELECT ",
        default_row_columns!(),
        " FROM zgm_settings.default_settings d"
    ))
    .fetch_all(connection)
    .await?;
    Ok(rows.into_iter().map(DefaultRow::from_columns).collect())
}

/// Every row of the defaults with the row of one tenant, if it has one, in the order of the
/// bytes of the names, as Rust orders text, not in the collation of the database; one
/// statement, the rows of the tenant found through the primary key.
pub(crate) async fn select_settings(
    connection: &mut PgConnection,
    tenant_id: Uuid,
) -> Result<Vec<(String, DefaultRow, Option<StoredValue>)>, SettingsError> {
    type Row = (
        String,
        String,
        String,
        String,
        String,
        Option<i64>,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let rows: Vec<Row> = sqlx::query_as(concat!(
        "SELECT ",
        default_row_columns!(),
        ", ",
        stored_value_columns!("t."),
        " FROM zgm_settings.default_settings d \
         LEFT JOIN zgm_settings.tenant_settings t ON t.tenant_id = $1 AND t.name = d.name \
         ORDER BY d.name COLLATE \"C\""
    ))
    .bind(tenant_id)
    .fetch_all(connection)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(name, kind, json_type, text, json, min, max, of_type, of_text, of_json)| {
                let (name, row) =
                    DefaultRow::from_columns((name, kind, json_type, text, json, min, max));
                (name, row, stored_of_the_tenant(of_type, of_text, of_json))
            },
        )
        .collect())
}
