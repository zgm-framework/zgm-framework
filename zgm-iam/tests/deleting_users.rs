//! The loop that deletes the users past their date of deletion, on a real database. `build()`
//! runs the first pass at once, so a test puts a date in the past by hand and builds the module
//! again to get a pass.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use sqlx::PgPool;
use uuid::Uuid;
use zgm_iam::{Iam, IamError, Job, State};

use common::{FlippingCipher, RecordedSettings, iam_with_settings, set_state_of_tenant};

/// The module, built at this moment: its loop starts its first pass at once.
async fn built_now(pool: &PgPool) -> Iam {
    Iam::builder()
        .pool(pool.clone())
        .cipher(FlippingCipher::default())
        .settings(RecordedSettings::default())
        // A failed pass shows up as users that are not deleted, which the tests wait for.
        .run_jobs_in_background(|error| eprintln!("the deletion failed: {}", error.error))
        .build()
        .await
        .unwrap()
}

/// Puts the date of deletion of a user an hour in the past, by hand, as when its window ran
/// out.
async fn date_passed(pool: &PgPool, user_id: Uuid) {
    sqlx::query(
        "UPDATE zgm_iam.users SET deletes_at = statement_timestamp() - interval '1 hour' \
         WHERE id = $1",
    )
    .bind(user_id)
    .execute(pool)
    .await
    .unwrap();
}

/// The ids of the users in the table, in order.
async fn users(pool: &PgPool) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT id FROM zgm_iam.users ORDER BY id")
        .fetch_all(pool)
        .await
        .unwrap()
}

/// Waits, for at most five seconds, until the table has `expected` users.
async fn wait_for_users(pool: &PgPool, expected: usize) {
    for _ in 0..500 {
        if users(pool).await.len() == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the table did not get to {expected} users");
}

#[sqlx::test(migrations = false)]
async fn a_user_past_its_date_is_deleted_and_the_others_stay(pool: PgPool) {
    let iam = iam_with_settings(&pool, RecordedSettings::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let in_kowalski = iam.tenant_id(kowalski.id);
    let mut ids = Vec::new();
    for _ in 0..4 {
        ids.push(in_kowalski.create_user().await.unwrap().id);
    }
    let (anna, jan, ola, ewa) = (ids[0], ids[1], ids[2], ids[3]);
    // Anna's date has passed; Jan waits 30 more days; Ola is active; Ewa is blocked.
    in_kowalski.user_id(anna).delete().await.unwrap();
    date_passed(&pool, anna).await;
    in_kowalski.user_id(jan).delete().await.unwrap();
    in_kowalski
        .user_id(ewa)
        .set_state(State::Suspended)
        .await
        .unwrap();

    let _second = built_now(&pool).await;
    wait_for_users(&pool, 3).await;

    let mut left = vec![jan, ola, ewa];
    left.sort();
    assert_eq!(users(&pool).await, left);
    assert_eq!(in_kowalski.user_id(anna).get().await.unwrap(), None);
}

#[sqlx::test(migrations = false)]
async fn a_user_brought_back_before_the_pass_stays(pool: PgPool) {
    let iam = iam_with_settings(&pool, RecordedSettings::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let in_kowalski = iam.tenant_id(kowalski.id);
    let anna = in_kowalski.create_user().await.unwrap();
    in_kowalski.user_id(anna.id).delete().await.unwrap();
    date_passed(&pool, anna.id).await;
    in_kowalski
        .user_id(anna.id)
        .set_state(State::Active)
        .await
        .unwrap();

    let _second = built_now(&pool).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(users(&pool).await, [anna.id]);
}

#[sqlx::test(migrations = false)]
async fn the_users_of_a_tenant_that_is_not_active_are_deleted_too(pool: PgPool) {
    let iam = iam_with_settings(&pool, RecordedSettings::default()).await;
    for (name, word) in [("Kowalski", "suspended"), ("Nowak", "pending_deletion")] {
        let tenant = iam.create_tenant(name).await.unwrap();
        let in_tenant = iam.tenant_id(tenant.id);
        let user = in_tenant.create_user().await.unwrap();
        in_tenant.user_id(user.id).delete().await.unwrap();
        date_passed(&pool, user.id).await;
        set_state_of_tenant(&pool, tenant.id, word).await;
    }

    let _second = built_now(&pool).await;

    wait_for_users(&pool, 0).await;
}

#[sqlx::test(migrations = false)]
async fn one_pass_deletes_more_than_one_batch(pool: PgPool) {
    let iam = iam_with_settings(&pool, RecordedSettings::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    // 250 users whose date passed an hour ago: two full batches and a half.
    sqlx::query(
        "INSERT INTO zgm_iam.users (tenant_id, state, deletes_at) \
         SELECT $1, 'pending_deletion', statement_timestamp() - interval '1 hour' \
         FROM generate_series(1, 250)",
    )
    .bind(kowalski.id)
    .execute(&pool)
    .await
    .unwrap();

    let _second = built_now(&pool).await;

    wait_for_users(&pool, 0).await;
}

#[tokio::test(start_paused = true)]
async fn a_failed_pass_goes_to_the_function_for_errors_and_a_panic_there_does_not_end_the_loop() {
    // A pool that is closed: every pass fails at once, with no database at all.
    let pool = PgPool::connect_lazy("postgres://nobody@localhost/nothing").unwrap();
    pool.close().await;
    let reported: Arc<Mutex<Vec<(Job, bool)>>> = Arc::default();
    let reported_in_loop = Arc::clone(&reported);

    let _iam = Iam::builder()
        .pool(pool)
        .cipher(FlippingCipher::default())
        .settings(RecordedSettings::default())
        .run_jobs_in_background(move |error| {
            let database = matches!(error.error, IamError::Database(_));
            reported_in_loop.lock().unwrap().push((error.job, database));
            panic!("the function for errors itself fails");
        })
        .build()
        .await
        .unwrap();

    // The first pass runs at once, then one every two minutes. The clock of the test is
    // paused, so three minutes pass in an instant, three times.
    for passes in 1..=4 {
        for _ in 0..100 {
            if reported.lock().unwrap().len() == 2 * passes {
                break;
            }
            tokio::task::yield_now().await;
        }
        // Each pass runs both jobs, and each fails on its own.
        let each_pass = [
            (Job::DeleteUsersPastTheirDate, true),
            (Job::DeleteTenantsPastTheirDate, true),
        ];
        assert_eq!(*reported.lock().unwrap(), each_pass.repeat(passes));
        tokio::time::advance(Duration::from_mins(3)).await;
    }
    assert_eq!(
        Job::DeleteUsersPastTheirDate.to_string(),
        "delete_users_past_their_date"
    );
    assert_eq!(
        Job::DeleteTenantsPastTheirDate.to_string(),
        "delete_tenants_past_their_date"
    );
}
