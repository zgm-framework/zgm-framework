//! `Iam::create_tenant`: a new tenant with a name the application's cipher encrypts.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use sqlx::PgPool;
use uuid::Uuid;
use zgm_iam::{IamError, State};

use common::{EmptyCipher, FailingCipher, FlippingCipher, count_tenants, flipped, iam_with};

#[sqlx::test(migrations = false)]
async fn create_tenant_gives_back_the_tenant_with_the_id_and_the_dates_the_database_gave(
    pool: PgPool,
) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;

    let tenant = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();

    assert_eq!(tenant.name, "Kowalski sp. z o.o.");
    assert_eq!(tenant.id.get_version_num(), 7);
    assert_eq!(tenant.created_at, tenant.updated_at);

    // The id and the dates are those of the row in the table.
    let (id, created_at, updated_at): (Uuid, jiff_sqlx::Timestamp, jiff_sqlx::Timestamp) =
        sqlx::query_as("SELECT id, created_at, updated_at FROM zgm_iam.tenants")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(id, tenant.id);
    assert_eq!(created_at.to_jiff(), tenant.created_at);
    assert_eq!(updated_at.to_jiff(), tenant.updated_at);
}

#[sqlx::test(migrations = false)]
async fn a_new_tenant_is_active_and_waits_for_no_deletion_as_its_row_says(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;

    let tenant = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();

    assert_eq!(tenant.state, State::Active);
    assert_eq!(tenant.deletes_at, None);
    let (state, no_date_of_deletion): (String, bool) =
        sqlx::query_as("SELECT state, deletes_at IS NULL FROM zgm_iam.tenants WHERE id = $1")
            .bind(tenant.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(state, "active");
    assert!(no_date_of_deletion);
}

#[sqlx::test(migrations = false)]
async fn a_state_the_module_does_not_know_is_an_error_and_not_taken_for_another(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    // Only by hand can the table get a word that is not a state of the module.
    sqlx::query("ALTER TABLE zgm_iam.tenants ALTER COLUMN state SET DEFAULT 'activ'")
        .execute(&pool)
        .await
        .unwrap();

    let refused = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap_err();

    assert!(
        matches!(&refused, IamError::UnknownState(word) if word == "activ"),
        "{refused}"
    );
    assert_eq!(
        refused.to_string(),
        "the database holds a state the module does not know: activ"
    );
}

#[sqlx::test(migrations = false)]
async fn the_table_holds_what_the_cipher_made_of_the_name_and_not_the_name(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;

    let tenant = iam.create_tenant("Zakład Łódź").await.unwrap();

    let stored: Vec<u8> = sqlx::query_scalar("SELECT name FROM zgm_iam.tenants WHERE id = $1")
        .bind(tenant.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, flipped("Zakład Łódź".as_bytes()));
}

#[sqlx::test(migrations = false)]
async fn an_empty_name_is_refused_before_the_cipher_is_called_and_nothing_is_written(pool: PgPool) {
    let calls = Arc::new(AtomicUsize::new(0));
    let iam = iam_with(
        &pool,
        FlippingCipher {
            calls: calls.clone(),
        },
    )
    .await;

    let refused = iam.create_tenant("").await.unwrap_err();

    assert!(matches!(refused, IamError::EmptyName), "{refused}");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(count_tenants(&pool).await, 0);
}

#[sqlx::test(migrations = false)]
async fn an_error_of_the_cipher_is_the_error_of_create_tenant_and_nothing_is_written(pool: PgPool) {
    let iam = iam_with(&pool, FailingCipher).await;

    let refused = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap_err();

    assert!(matches!(refused, IamError::Cipher(_)), "{refused}");
    assert_eq!(
        refused.to_string(),
        "the cipher failed: the key store did not answer"
    );
    assert_eq!(count_tenants(&pool).await, 0);
}

#[sqlx::test(migrations = false)]
async fn empty_bytes_from_the_cipher_are_refused_and_nothing_is_written(pool: PgPool) {
    let iam = iam_with(&pool, EmptyCipher).await;

    let refused = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap_err();

    assert!(matches!(refused, IamError::EmptyCiphertext), "{refused}");
    assert_eq!(count_tenants(&pool).await, 0);
}
