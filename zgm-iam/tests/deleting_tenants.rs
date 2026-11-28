//! The loop that deletes the tenants past their date of deletion, with their users, on a real
//! database. `build()` runs the first pass at once, so a test puts a date in the past by hand
//! and builds the module again to get a pass.

mod common;

use std::time::Duration;

use sqlx::PgPool;
use uuid::Uuid;
use zgm_iam::{Iam, State, Tenant};

use common::{FlippingCipher, RecordedSettings, iam_with_settings};

/// The module, built at this moment: its loop starts its first pass at once.
async fn built_now(pool: &PgPool) -> Iam {
    Iam::builder()
        .pool(pool.clone())
        .cipher(FlippingCipher::default())
        .settings(RecordedSettings::default())
        // A failed pass shows up as tenants that are not deleted, which the tests wait for.
        .run_jobs_in_background(|error| eprintln!("the deletion failed: {}", error.error))
        .build()
        .await
        .unwrap()
}

/// Puts the date of deletion of a tenant an hour in the past, by hand, as when its window ran
/// out.
async fn date_passed(pool: &PgPool, tenant_id: Uuid) {
    sqlx::query(
        "UPDATE zgm_iam.tenants SET deletes_at = statement_timestamp() - interval '1 hour' \
         WHERE id = $1",
    )
    .bind(tenant_id)
    .execute(pool)
    .await
    .unwrap();
}

/// The ids of the tenants in the table, in order.
async fn tenants(pool: &PgPool) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT id FROM zgm_iam.tenants ORDER BY id")
        .fetch_all(pool)
        .await
        .unwrap()
}

/// The tenants of the users in the table, one for each user, in order.
async fn tenants_of_users(pool: &PgPool) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT tenant_id FROM zgm_iam.users ORDER BY tenant_id")
        .fetch_all(pool)
        .await
        .unwrap()
}

/// Waits, for at most five seconds, until the table has `expected` tenants.
async fn wait_for_tenants(pool: &PgPool, expected: usize) {
    for _ in 0..500 {
        if tenants(pool).await.len() == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the table did not get to {expected} tenants");
}

/// A tenant with two users.
async fn tenant_with_two_users(iam: &Iam, name: &str) -> Tenant {
    let tenant = iam.create_tenant(name).await.unwrap();
    let in_tenant = iam.tenant_id(tenant.id);
    in_tenant.create_user().await.unwrap();
    in_tenant.create_user().await.unwrap();
    tenant
}

#[sqlx::test(migrations = false)]
async fn a_tenant_past_its_date_is_deleted_with_its_users_and_the_others_stay(pool: PgPool) {
    let iam = iam_with_settings(&pool, RecordedSettings::default()).await;
    let kowalski = tenant_with_two_users(&iam, "Kowalski").await;
    let nowak = tenant_with_two_users(&iam, "Nowak").await;
    let wisniewski = tenant_with_two_users(&iam, "Wiśniewski").await;
    let zielinski = tenant_with_two_users(&iam, "Zieliński").await;
    // Kowalski's date has passed; Nowak waits 90 more days; Wiśniewski is blocked; Zieliński
    // is active.
    iam.tenant_id(kowalski.id).delete().await.unwrap();
    date_passed(&pool, kowalski.id).await;
    iam.tenant_id(nowak.id).delete().await.unwrap();
    iam.tenant_id(wisniewski.id)
        .set_state(State::Suspended)
        .await
        .unwrap();

    let _second = built_now(&pool).await;
    wait_for_tenants(&pool, 3).await;

    let mut left = vec![nowak.id, wisniewski.id, zielinski.id];
    left.sort();
    assert_eq!(tenants(&pool).await, left);
    let mut users_left = [left.clone(), left].concat();
    users_left.sort();
    assert_eq!(tenants_of_users(&pool).await, users_left);
}

#[sqlx::test(migrations = false)]
async fn a_tenant_brought_back_before_the_pass_stays(pool: PgPool) {
    let iam = iam_with_settings(&pool, RecordedSettings::default()).await;
    let kowalski = tenant_with_two_users(&iam, "Kowalski").await;
    let in_kowalski = iam.tenant_id(kowalski.id);
    in_kowalski.delete().await.unwrap();
    date_passed(&pool, kowalski.id).await;
    in_kowalski.set_state(State::Active).await.unwrap();

    let _second = built_now(&pool).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(tenants(&pool).await, [kowalski.id]);
    assert_eq!(tenants_of_users(&pool).await.len(), 2);
}

#[sqlx::test(migrations = false)]
async fn one_pass_deletes_every_tenant_past_its_date(pool: PgPool) {
    let iam = iam_with_settings(&pool, RecordedSettings::default()).await;
    for number in 0..4 {
        let tenant = tenant_with_two_users(&iam, &format!("Tenant {number}")).await;
        iam.tenant_id(tenant.id).delete().await.unwrap();
        date_passed(&pool, tenant.id).await;
    }

    let _second = built_now(&pool).await;

    wait_for_tenants(&pool, 0).await;
    assert_eq!(tenants_of_users(&pool).await, Vec::<Uuid>::new());
}
