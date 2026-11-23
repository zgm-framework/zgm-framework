//! `zgm_settings::migrate` on a database with nothing of the module in it.

use sqlx::PgPool;

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
async fn migrate_creates_the_table_and_keeps_its_history_in_its_own_schema(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();

    assert_eq!(
        tables_of(&pool, "zgm_settings").await,
        ["_sqlx_migrations", "tenant_settings"]
    );
    // Nothing of the module outside its schema.
    assert_eq!(tables_of(&pool, "public").await, Vec::<String>::new());

    // Called again, also several times at once, it changes nothing and fails nowhere.
    let (first, second, third) = tokio::join!(
        zgm_settings::migrate(&pool),
        zgm_settings::migrate(&pool),
        zgm_settings::migrate(&pool)
    );
    first.unwrap();
    second.unwrap();
    third.unwrap();
    let applied: i64 = sqlx::query_scalar("SELECT count(*) FROM zgm_settings._sqlx_migrations")
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

    zgm_settings::migrate(&pool).await.unwrap();

    // The application's history is as it was.
    let of_the_application: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(of_the_application, 1);
}
