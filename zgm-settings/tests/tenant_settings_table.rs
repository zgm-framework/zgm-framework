//! What the table `zgm_settings.tenant_settings` itself guarantees, checked with SQL by hand:
//! no function of the module writes it yet.

use sqlx::error::ErrorKind;
use sqlx::{AssertSqlSafe, PgPool};

/// Two tenants, as the plain UUIDs the table keeps.
const NOWAK: &str = "019ab000-0000-7000-8000-000000000001";
const KOWALSKI: &str = "019ab000-0000-7000-8000-000000000002";

/// The name of a setting, with the prefix of its module.
const WINDOW: &str = "iam.user_deletion_window";

/// Writes a setting of a tenant with SQL by hand; `value` is the text of the JSON.
async fn insert(
    pool: &PgPool,
    tenant_id: &str,
    name: &str,
    value: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO zgm_settings.tenant_settings (tenant_id, name, value) \
         VALUES ($1::uuid, $2, $3::jsonb)",
    )
    .bind(tenant_id)
    .bind(name)
    .bind(value)
    .execute(pool)
    .await
    .map(|_| ())
}

/// How many rows the table has.
async fn count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM zgm_settings.tenant_settings")
        .fetch_one(pool)
        .await
        .unwrap()
}

fn kind_of(error: &sqlx::Error) -> Option<ErrorKind> {
    error.as_database_error().map(|error| error.kind())
}

#[sqlx::test(migrations = false)]
async fn a_new_row_gets_the_moment_of_its_statement_as_both_dates(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();

    // In a transaction that began earlier: the dates are the moment of the statement, not of
    // the start of the transaction.
    let mut transaction = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_sleep(0.01)")
        .execute(&mut *transaction)
        .await
        .unwrap();
    let dated: bool = sqlx::query_scalar(
        "INSERT INTO zgm_settings.tenant_settings (tenant_id, name, value) \
         VALUES ($1::uuid, $2, '7776000') \
         RETURNING created_at = statement_timestamp() AND updated_at = created_at \
                   AND created_at > transaction_timestamp()",
    )
    .bind(NOWAK)
    .bind(WINDOW)
    .fetch_one(&mut *transaction)
    .await
    .unwrap();
    assert!(dated);
}

#[sqlx::test(migrations = false)]
async fn a_change_sets_updated_at_to_the_moment_of_the_change(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();

    let mut transaction = pool.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO zgm_settings.tenant_settings (tenant_id, name, value) \
         VALUES ($1::uuid, $2, '7776000')",
    )
    .bind(NOWAK)
    .bind(WINDOW)
    .execute(&mut *transaction)
    .await
    .unwrap();
    sqlx::query("SELECT pg_sleep(0.01)")
        .execute(&mut *transaction)
        .await
        .unwrap();
    // Whatever the statement says, and also later in the same transaction.
    let touched: bool = sqlx::query_scalar(
        "UPDATE zgm_settings.tenant_settings SET value = '1209600', updated_at = '2000-01-01' \
         WHERE tenant_id = $1::uuid AND name = $2 \
         RETURNING updated_at = statement_timestamp() AND updated_at > created_at",
    )
    .bind(NOWAK)
    .bind(WINDOW)
    .fetch_one(&mut *transaction)
    .await
    .unwrap();
    assert!(touched);
}

#[sqlx::test(migrations = false)]
async fn the_same_setting_twice_in_one_tenant_is_refused(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();
    insert(&pool, NOWAK, WINDOW, "7776000").await.unwrap();

    let refused = insert(&pool, NOWAK, WINDOW, "1209600").await.unwrap_err();

    assert!(
        matches!(kind_of(&refused), Some(ErrorKind::UniqueViolation)),
        "{refused}"
    );
    let kept: bool = sqlx::query_scalar(
        "SELECT value = '7776000' FROM zgm_settings.tenant_settings \
         WHERE tenant_id = $1::uuid AND name = $2",
    )
    .bind(NOWAK)
    .bind(WINDOW)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(kept);
}

#[sqlx::test(migrations = false)]
async fn the_same_setting_in_two_tenants_is_two_rows(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();

    insert(&pool, NOWAK, WINDOW, "7776000").await.unwrap();
    insert(&pool, KOWALSKI, WINDOW, "1209600").await.unwrap();

    assert_eq!(count(&pool).await, 2);
}

#[sqlx::test(migrations = false)]
async fn two_settings_of_one_tenant_are_two_rows(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();

    insert(&pool, NOWAK, WINDOW, "7776000").await.unwrap();
    insert(&pool, NOWAK, "iam.tenant_deletion_window", "7776000")
        .await
        .unwrap();

    assert_eq!(count(&pool).await, 2);
}

#[sqlx::test(migrations = false)]
async fn a_row_without_a_tenant_a_name_or_a_value_is_refused(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();

    for values in [
        format!("(NULL, '{WINDOW}', '7776000')"),
        format!("('{NOWAK}', NULL, '7776000')"),
        format!("('{NOWAK}', '{WINDOW}', NULL)"),
    ] {
        // Only the constants of this file go into the text.
        let refused = sqlx::raw_sql(AssertSqlSafe(format!(
            "INSERT INTO zgm_settings.tenant_settings (tenant_id, name, value) VALUES {values}"
        )))
        .execute(&pool)
        .await
        .unwrap_err();
        assert!(
            matches!(kind_of(&refused), Some(ErrorKind::NotNullViolation)),
            "{values}: {refused}"
        );
    }
    assert_eq!(count(&pool).await, 0);
}

#[sqlx::test(migrations = false)]
async fn a_value_that_is_not_json_is_refused(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();

    // A literal without a cast: the column itself decides what it takes. Only the constants
    // of this file go into the text.
    let refused = sqlx::raw_sql(AssertSqlSafe(format!(
        "INSERT INTO zgm_settings.tenant_settings (tenant_id, name, value) \
         VALUES ('{NOWAK}', '{WINDOW}', '90 days')"
    )))
    .execute(&pool)
    .await
    .unwrap_err();

    // invalid_text_representation: the text is no JSON.
    let code = refused
        .as_database_error()
        .and_then(|error| error.code().map(|code| code.into_owned()));
    assert_eq!(code.as_deref(), Some("22P02"), "{refused}");
    assert_eq!(count(&pool).await, 0);
}

#[sqlx::test(migrations = false)]
async fn json_of_every_kind_is_kept_as_it_was_written(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();

    for (name, value) in [
        ("iam.a_number", "7776000"),
        ("iam.a_flag", "true"),
        ("iam.a_text", "\"Kowalski\""),
        ("limits.a_rule", r#"{"at_most": 10, "per_seconds": 3600}"#),
    ] {
        insert(&pool, NOWAK, name, value).await.unwrap();
        let kept: bool = sqlx::query_scalar(
            "SELECT value = $3::jsonb FROM zgm_settings.tenant_settings \
             WHERE tenant_id = $1::uuid AND name = $2",
        )
        .bind(NOWAK)
        .bind(name)
        .bind(value)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(kept, "{name}");
    }
}

#[sqlx::test(migrations = false)]
async fn a_row_can_be_deleted(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();
    insert(&pool, NOWAK, WINDOW, "7776000").await.unwrap();

    // Going back to the default will be this.
    let deleted = sqlx::query(
        "DELETE FROM zgm_settings.tenant_settings WHERE tenant_id = $1::uuid AND name = $2",
    )
    .bind(NOWAK)
    .bind(WINDOW)
    .execute(&pool)
    .await
    .unwrap();

    assert_eq!(deleted.rows_affected(), 1);
    assert_eq!(count(&pool).await, 0);
}

#[sqlx::test(migrations = false)]
async fn the_key_is_the_tenant_first_and_then_the_name(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();

    // The tenant first: then the same index finds one setting of a tenant and all of them.
    let columns: Vec<String> = sqlx::query_scalar(
        "SELECT a.attname::text \
         FROM pg_index i \
         CROSS JOIN LATERAL unnest(i.indkey) WITH ORDINALITY AS k(attnum, position) \
         JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = k.attnum \
         WHERE i.indrelid = 'zgm_settings.tenant_settings'::regclass AND i.indisprimary \
         ORDER BY k.position",
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(columns, ["tenant_id", "name"]);
}
