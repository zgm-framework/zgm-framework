//! What the tests on the database share: a module's settings, two tenants, and SQL by hand.
#![allow(
    dead_code,
    reason = "every file of tests is a program of its own and uses only some of these"
)]

use std::time::Duration;

use sqlx::PgPool;
use uuid::Uuid;
use zgm_settings::{DefaultSetting, Settings, Value};

/// The settings of a module, one of every kind.
#[derive(Clone, Copy, strum::AsRefStr)]
#[strum(serialize_all = "snake_case", prefix = "iam.")]
pub enum IamSetting {
    UserDeletionWindow,
    MaxUsers,
    SelfSignUp,
    WelcomeText,
}

pub const NOWAK: Uuid = Uuid::from_u128(0x019ab000_0000_7000_8000_000000000001);
pub const KOWALSKI: Uuid = Uuid::from_u128(0x019ab000_0000_7000_8000_000000000002);

const DAY: u64 = 24 * 60 * 60;

pub fn days(count: u64) -> Value {
    Value::Duration(Duration::from_secs(count * DAY))
}

/// The module on a migrated database, with the defaults of [`IamSetting`]: 30 days, 10,
/// no, and "Welcome", without bounds.
pub async fn settings_on(pool: &PgPool) -> Settings {
    zgm_settings::migrate(pool).await.unwrap();
    let settings = Settings::builder().pool(pool.clone()).build();
    for (setting, default) in [
        (
            IamSetting::UserDeletionWindow,
            DefaultSetting::Duration {
                value: Duration::from_secs(30 * DAY),
                min: None,
                max: None,
            },
        ),
        (
            IamSetting::MaxUsers,
            DefaultSetting::Number {
                value: 10,
                min: None,
                max: None,
            },
        ),
        (IamSetting::SelfSignUp, DefaultSetting::Flag(false)),
        (
            IamSetting::WelcomeText,
            DefaultSetting::Text("Welcome".to_owned()),
        ),
    ] {
        settings.add_default(setting, default).await.unwrap();
    }
    settings
}

/// Writes the row of a default with SQL by hand, as an earlier version of a module would
/// have left it: a setting the module no longer gives, or a row the module cannot read.
/// `json` is the text of the JSON.
pub async fn default_by_hand(pool: &PgPool, name: impl AsRef<str>, kind: &str, json: &str) {
    sqlx::query(
        "INSERT INTO zgm_settings.default_settings (name, kind, value) \
         VALUES ($1, $2, $3::jsonb)",
    )
    .bind(name.as_ref())
    .bind(kind)
    .bind(json)
    .execute(pool)
    .await
    .unwrap();
}

/// The row of a default as the database holds it: the kind, the JSON as the database writes
/// it, and the bounds; `None` without a row.
pub async fn default_row(
    pool: &PgPool,
    name: impl AsRef<str>,
) -> Option<(String, String, Option<i64>, Option<i64>)> {
    sqlx::query_as(
        "SELECT kind, value::text, min, max FROM zgm_settings.default_settings WHERE name = $1",
    )
    .bind(name.as_ref())
    .fetch_optional(pool)
    .await
    .unwrap()
}

/// Writes the value of a setting of a tenant with SQL by hand, as the text of its JSON.
pub async fn set_by_hand(pool: &PgPool, tenant_id: Uuid, name: impl AsRef<str>, json: &str) {
    sqlx::query(
        "INSERT INTO zgm_settings.tenant_settings (tenant_id, name, value) \
         VALUES ($1, $2, $3::jsonb) \
         ON CONFLICT (tenant_id, name) DO UPDATE SET value = excluded.value",
    )
    .bind(tenant_id)
    .bind(name.as_ref())
    .bind(json)
    .execute(pool)
    .await
    .unwrap();
}

/// The JSON of the row of a tenant and a setting, as the database writes it, or `None`
/// without a row.
pub async fn json_of(pool: &PgPool, tenant_id: Uuid, name: impl AsRef<str>) -> Option<String> {
    sqlx::query_scalar(
        "SELECT value::text FROM zgm_settings.tenant_settings \
         WHERE tenant_id = $1 AND name = $2",
    )
    .bind(tenant_id)
    .bind(name.as_ref())
    .fetch_optional(pool)
    .await
    .unwrap()
}

/// Whether the row of a tenant and a setting was changed after it was written: `updated_at`
/// later than `created_at`.
pub async fn changed_after_written(pool: &PgPool, tenant_id: Uuid, name: impl AsRef<str>) -> bool {
    sqlx::query_scalar(
        "SELECT updated_at > created_at FROM zgm_settings.tenant_settings \
         WHERE tenant_id = $1 AND name = $2",
    )
    .bind(tenant_id)
    .bind(name.as_ref())
    .fetch_one(pool)
    .await
    .unwrap()
}

/// How many rows the table has.
pub async fn rows(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM zgm_settings.tenant_settings")
        .fetch_one(pool)
        .await
        .unwrap()
}
