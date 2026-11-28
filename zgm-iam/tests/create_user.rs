//! `Iam::tenant_id(...).create_user()`: a new user under an active tenant.

mod common;

use sqlx::PgPool;
use uuid::Uuid;
use zgm_iam::{IamError, State};

use common::{FlippingCipher, count_users, iam_with, set_state_of_tenant};

#[sqlx::test(migrations = false)]
async fn create_user_gives_back_the_user_with_what_the_database_gave_it(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();

    let anna = iam.tenant_id(kowalski.id).create_user().await.unwrap();

    assert_eq!(anna.id.get_version_num(), 7);
    assert_eq!(anna.tenant_id, kowalski.id);
    assert_eq!(anna.state, State::Active);
    assert_eq!(anna.deletes_at, None);
    assert_eq!(anna.created_at, anna.updated_at);

    // What it gave back is the row in the table.
    let (id, tenant_id, state, no_date_of_deletion, created_at, updated_at): (
        Uuid,
        Uuid,
        String,
        bool,
        jiff_sqlx::Timestamp,
        jiff_sqlx::Timestamp,
    ) = sqlx::query_as(
        "SELECT id, tenant_id, state, deletes_at IS NULL, created_at, updated_at \
         FROM zgm_iam.users",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(id, anna.id);
    assert_eq!(tenant_id, kowalski.id);
    assert_eq!(state, "active");
    assert!(no_date_of_deletion);
    assert_eq!(created_at.to_jiff(), anna.created_at);
    assert_eq!(updated_at.to_jiff(), anna.updated_at);
}

#[sqlx::test(migrations = false)]
async fn a_tenant_that_does_not_exist_gets_no_user(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let nobody: Uuid = sqlx::query_scalar("SELECT uuidv7()")
        .fetch_one(&pool)
        .await
        .unwrap();

    let refused = iam.tenant_id(nobody).create_user().await.unwrap_err();

    assert!(matches!(refused, IamError::NoSuchTenant), "{refused}");
    assert_eq!(refused.to_string(), "there is no such tenant");
    assert_eq!(count_users(&pool).await, 0);
}

#[sqlx::test(migrations = false)]
async fn a_suspended_tenant_gets_no_user_and_the_error_says_it_is_suspended(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    set_state_of_tenant(&pool, kowalski.id, "suspended").await;

    let refused = iam.tenant_id(kowalski.id).create_user().await.unwrap_err();

    assert!(
        matches!(refused, IamError::TenantNotActive(State::Suspended)),
        "{refused}"
    );
    assert_eq!(refused.to_string(), "the tenant is not active: Suspended");
    assert_eq!(count_users(&pool).await, 0);
}

#[sqlx::test(migrations = false)]
async fn a_tenant_pending_deletion_gets_no_user_and_the_error_says_so(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    set_state_of_tenant(&pool, kowalski.id, "pending_deletion").await;

    let refused = iam.tenant_id(kowalski.id).create_user().await.unwrap_err();

    assert!(
        matches!(refused, IamError::TenantNotActive(State::PendingDeletion)),
        "{refused}"
    );
    assert_eq!(count_users(&pool).await, 0);
}

#[sqlx::test(migrations = false)]
async fn a_state_of_the_tenant_the_module_does_not_know_is_an_error_and_gets_no_user(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    set_state_of_tenant(&pool, kowalski.id, "activ").await;

    let refused = iam.tenant_id(kowalski.id).create_user().await.unwrap_err();

    assert!(
        matches!(&refused, IamError::UnknownState(word) if word == "activ"),
        "{refused}"
    );
    assert_eq!(count_users(&pool).await, 0);
}

#[sqlx::test(migrations = false)]
async fn a_tenant_suspended_while_its_user_is_created_gets_no_user(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();

    // Somebody suspends the tenant and has not committed yet.
    let mut suspension = pool.begin().await.unwrap();
    sqlx::query("UPDATE zgm_iam.tenants SET state = 'suspended' WHERE id = $1")
        .bind(kowalski.id)
        .execute(&mut *suspension)
        .await
        .unwrap();

    // Meanwhile the user is being created.
    let in_kowalski = iam.tenant_id(kowalski.id);
    let creation = tokio::spawn(async move { in_kowalski.create_user().await });

    // The creation must wait for the suspension, not write the user past it.
    let mut waits = false;
    for _ in 0..500 {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity \
             WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        if waiting > 0 {
            waits = true;
            break;
        }
    }
    assert!(
        waits,
        "create_user did not wait for the suspension of the tenant"
    );

    suspension.commit().await.unwrap();
    let refused = creation.await.unwrap().unwrap_err();

    assert!(
        matches!(refused, IamError::TenantNotActive(State::Suspended)),
        "{refused}"
    );
    assert_eq!(count_users(&pool).await, 0);
}
