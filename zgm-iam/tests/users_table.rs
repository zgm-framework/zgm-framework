//! What the table of users itself guarantees, checked with SQL by hand: no function of the
//! module writes a user yet.

mod common;

use sqlx::PgPool;
use sqlx::error::ErrorKind;
use uuid::Uuid;

use common::{FlippingCipher, iam_with};

/// The kind of the database's error, to tell why it refused.
fn kind_of(error: &sqlx::Error) -> Option<ErrorKind> {
    error.as_database_error().map(|error| error.kind())
}

/// Writes a user of the tenant and gives back its id.
async fn insert_user(pool: &PgPool, tenant_id: Uuid) -> Uuid {
    sqlx::query_scalar("INSERT INTO zgm_iam.users (tenant_id) VALUES ($1) RETURNING id")
        .bind(tenant_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// How many users the tenant has.
async fn count_users_of(pool: &PgPool, tenant_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM zgm_iam.users WHERE tenant_id = $1")
        .bind(tenant_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = false)]
async fn the_database_gives_a_new_user_a_uuid_v7_both_dates_and_the_state_active(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let tenant = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();

    let (id, state, no_date_of_deletion, dates_equal): (Uuid, String, bool, bool) = sqlx::query_as(
        "INSERT INTO zgm_iam.users (tenant_id) VALUES ($1) \
             RETURNING id, state, deletes_at IS NULL, created_at = updated_at",
    )
    .bind(tenant.id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(id.get_version_num(), 7);
    assert_eq!(state, "active");
    assert!(no_date_of_deletion);
    assert!(dates_equal);
}

#[sqlx::test(migrations = false)]
async fn the_database_refuses_a_user_without_a_tenant(pool: PgPool) {
    zgm_iam::migrate(&pool).await.unwrap();

    let refused = sqlx::query("INSERT INTO zgm_iam.users DEFAULT VALUES")
        .execute(&pool)
        .await
        .unwrap_err();

    assert!(
        matches!(kind_of(&refused), Some(ErrorKind::NotNullViolation)),
        "{refused}"
    );
}

#[sqlx::test(migrations = false)]
async fn the_database_refuses_a_user_of_a_tenant_that_does_not_exist(pool: PgPool) {
    zgm_iam::migrate(&pool).await.unwrap();

    let refused = sqlx::query("INSERT INTO zgm_iam.users (tenant_id) VALUES (uuidv7())")
        .execute(&pool)
        .await
        .unwrap_err();

    assert!(
        matches!(kind_of(&refused), Some(ErrorKind::ForeignKeyViolation)),
        "{refused}"
    );
}

#[sqlx::test(migrations = false)]
async fn removing_a_tenant_removes_its_users_and_no_others(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let nowak = iam.create_tenant("Nowak i S-ka").await.unwrap();
    insert_user(&pool, kowalski.id).await;
    insert_user(&pool, kowalski.id).await;
    insert_user(&pool, nowak.id).await;

    sqlx::query("DELETE FROM zgm_iam.tenants WHERE id = $1")
        .bind(kowalski.id)
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(count_users_of(&pool, kowalski.id).await, 0);
    assert_eq!(count_users_of(&pool, nowak.id).await, 1);
}

#[sqlx::test(migrations = false)]
async fn a_change_of_a_user_sets_its_updated_at_to_the_moment_of_the_change(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let tenant = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let user = insert_user(&pool, tenant.id).await;

    // Whatever the statement says.
    let touched: bool = sqlx::query_scalar(
        "UPDATE zgm_iam.users SET state = 'suspended', updated_at = '2000-01-01' \
         WHERE id = $1 RETURNING updated_at >= created_at",
    )
    .bind(user)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(touched);
}

#[sqlx::test(migrations = false)]
async fn the_tenant_of_a_user_is_indexed_so_removing_a_tenant_does_not_read_all_users(
    pool: PgPool,
) {
    zgm_iam::migrate(&pool).await.unwrap();

    // An index whose first column is tenant_id: PostgreSQL does not make one for a foreign key.
    let indexed: bool = sqlx::query_scalar(
        "SELECT EXISTS ( \
             SELECT 1 FROM pg_index i \
             JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = i.indkey[0] \
             WHERE i.indrelid = 'zgm_iam.users'::regclass AND a.attname = 'tenant_id')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(indexed);
}

#[sqlx::test(migrations = false)]
async fn the_users_of_a_tenant_are_indexed_in_the_order_of_their_ids_and_by_state(pool: PgPool) {
    zgm_iam::migrate(&pool).await.unwrap();

    // The columns of every index of the table, in the order of the index.
    let indexes: Vec<Vec<String>> = sqlx::query_scalar(
        "SELECT array_agg(a.attname::text ORDER BY k.n) \
         FROM pg_index i \
         CROSS JOIN LATERAL unnest(i.indkey::int2[]) WITH ORDINALITY AS k(attnum, n) \
         JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = k.attnum \
         WHERE i.indrelid = 'zgm_iam.users'::regclass \
         GROUP BY i.indexrelid",
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    assert!(
        indexes.contains(&vec!["tenant_id".to_owned(), "id".to_owned()]),
        "{indexes:?}"
    );
    assert!(
        indexes.contains(&vec![
            "tenant_id".to_owned(),
            "state".to_owned(),
            "id".to_owned()
        ]),
        "{indexes:?}"
    );
}
