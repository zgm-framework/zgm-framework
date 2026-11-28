//! `zgm_iam::migrate` on a database with nothing of the module in it.

mod common;

use sqlx::PgPool;

use common::{FlippingCipher, iam_with};

/// The tables of a schema, in order.
async fn tables_of(pool: &PgPool, schema: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables \
         WHERE table_schema = $1 ORDER BY table_name",
    )
    .bind(schema)
    .fetch_all(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = false)]
async fn migrate_creates_the_tables_and_keeps_their_history_in_its_own_schema(pool: PgPool) {
    zgm_iam::migrate(&pool).await.unwrap();

    assert_eq!(
        tables_of(&pool, "zgm_iam").await,
        ["_sqlx_migrations", "tenants", "users"]
    );
    // Nothing of the module outside its schema.
    assert_eq!(tables_of(&pool, "public").await, Vec::<String>::new());

    // Called again, also several times at once, it changes nothing and fails nowhere.
    let (first, second, third) = tokio::join!(
        zgm_iam::migrate(&pool),
        zgm_iam::migrate(&pool),
        zgm_iam::migrate(&pool)
    );
    first.unwrap();
    second.unwrap();
    third.unwrap();
    let applied: i64 = sqlx::query_scalar("SELECT count(*) FROM zgm_iam._sqlx_migrations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(applied, 1);
}

#[sqlx::test(migrations = false)]
async fn migrate_is_not_disturbed_by_the_migrations_of_the_application(pool: PgPool) {
    // The application keeps its own history in the table `sqlx` uses by default, with a
    // migration the module knows nothing about.
    sqlx::raw_sql(
        "CREATE TABLE _sqlx_migrations ( \
             version bigint PRIMARY KEY, description text NOT NULL, \
             installed_on timestamptz NOT NULL DEFAULT now(), success boolean NOT NULL, \
             checksum bytea NOT NULL, execution_time bigint NOT NULL); \
         INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
         VALUES (20200101000000, 'of the application', true, '\\x00', 1);",
    )
    .execute(&pool)
    .await
    .unwrap();

    zgm_iam::migrate(&pool).await.unwrap();

    // The application's history is as it was.
    let of_the_application: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(of_the_application, 1);
}

// The two tests below check what the table itself guarantees, with SQL by hand where no
// function of the module can get: none changes a tenant yet, and none can write one without
// a name.

#[sqlx::test(migrations = false)]
async fn a_change_of_a_tenant_sets_its_updated_at_to_the_moment_of_the_change(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let tenant = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();

    // Whatever the statement says.
    let touched: bool = sqlx::query_scalar(
        "UPDATE zgm_iam.tenants SET name = '\\x02', updated_at = '2000-01-01' \
         WHERE id = $1 RETURNING updated_at >= created_at",
    )
    .bind(tenant.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(touched);
}

#[sqlx::test(migrations = false)]
async fn the_database_refuses_a_tenant_without_a_name(pool: PgPool) {
    zgm_iam::migrate(&pool).await.unwrap();

    let refused = sqlx::query("INSERT INTO zgm_iam.tenants (id) VALUES (uuidv7())")
        .execute(&pool)
        .await
        .unwrap_err();

    let kind = refused.as_database_error().map(|error| error.kind());
    assert!(
        matches!(kind, Some(sqlx::error::ErrorKind::NotNullViolation)),
        "{refused}"
    );
}
