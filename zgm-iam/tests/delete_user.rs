//! `Iam::tenant_id(...).user_id(id).delete()`: a user waits for deletion for the window of its
//! tenant.

mod common;

use std::time::Duration;

use jiff::SignedDuration;
use sqlx::PgPool;
use zgm_iam::{IamError, State, User};

use common::{
    FailingSettings, RecordedSettings, an_id_of_nothing, iam_with_settings, row_of,
    set_state_of_tenant, set_state_of_user,
};

const WINDOW: &str = "iam.user_deletion_window";
const DAY: u64 = 24 * 60 * 60;

fn days(count: u64) -> Duration {
    Duration::from_secs(count * DAY)
}

/// The date a user changed at `user.updated_at` must be deleted at, with a window of `days`.
fn deletes_after(user: &User, count: u64) -> jiff::Timestamp {
    user.updated_at
        .checked_add(SignedDuration::try_from(days(count)).unwrap())
        .unwrap()
}

#[sqlx::test(migrations = false)]
async fn an_active_user_waits_for_deletion_30_days_from_the_moment_of_the_change(pool: PgPool) {
    let iam = iam_with_settings(&pool, RecordedSettings::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let in_kowalski = iam.tenant_id(kowalski.id);
    let anna = in_kowalski.create_user().await.unwrap();

    let deleted = in_kowalski
        .user_id(anna.id)
        .delete()
        .await
        .unwrap()
        .unwrap();

    assert_eq!(deleted.id, anna.id);
    assert_eq!(deleted.tenant_id, kowalski.id);
    assert_eq!(deleted.state, State::PendingDeletion);
    assert_eq!(deleted.created_at, anna.created_at);
    assert!(deleted.updated_at > anna.updated_at);
    assert_eq!(deleted.deletes_at, Some(deletes_after(&deleted, 30)));
    // What it gives back is what the row says.
    assert_eq!(deleted, row_of(&pool, anna.id).await);
}

#[sqlx::test(migrations = false)]
async fn the_window_is_the_one_of_the_tenant(pool: PgPool) {
    let settings = RecordedSettings::default();
    let iam = iam_with_settings(&pool, settings.clone()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let nowak = iam.create_tenant("Nowak S.A.").await.unwrap();
    settings.set(nowak.id, WINDOW, days(90));
    let anna = iam.tenant_id(kowalski.id).create_user().await.unwrap();
    let jan = iam.tenant_id(nowak.id).create_user().await.unwrap();

    let anna = iam
        .tenant_id(kowalski.id)
        .user_id(anna.id)
        .delete()
        .await
        .unwrap()
        .unwrap();
    let jan = iam
        .tenant_id(nowak.id)
        .user_id(jan.id)
        .delete()
        .await
        .unwrap()
        .unwrap();

    assert_eq!(anna.deletes_at, Some(deletes_after(&anna, 30)));
    assert_eq!(jan.deletes_at, Some(deletes_after(&jan, 90)));
}

#[sqlx::test(migrations = false)]
async fn the_settings_are_asked_for_the_window_of_the_tenant_once(pool: PgPool) {
    let settings = RecordedSettings::default();
    let iam = iam_with_settings(&pool, settings.clone()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let anna = iam.tenant_id(kowalski.id).create_user().await.unwrap();

    iam.tenant_id(kowalski.id)
        .user_id(anna.id)
        .delete()
        .await
        .unwrap();

    assert_eq!(
        *settings.asked.lock().unwrap(),
        [(kowalski.id, WINDOW.to_owned())]
    );
}

#[sqlx::test(migrations = false)]
async fn a_window_of_0_days_deletes_at_the_moment_of_the_change(pool: PgPool) {
    let settings = RecordedSettings::default();
    let iam = iam_with_settings(&pool, settings.clone()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    settings.set(kowalski.id, WINDOW, Duration::ZERO);
    let anna = iam.tenant_id(kowalski.id).create_user().await.unwrap();

    let anna = iam
        .tenant_id(kowalski.id)
        .user_id(anna.id)
        .delete()
        .await
        .unwrap()
        .unwrap();

    assert_eq!(anna.state, State::PendingDeletion);
    assert_eq!(anna.deletes_at, Some(anna.updated_at));
}

#[sqlx::test(migrations = false)]
async fn a_user_that_waits_for_deletion_already_stays_as_it_was(pool: PgPool) {
    let settings = RecordedSettings::default();
    let iam = iam_with_settings(&pool, settings.clone()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let in_kowalski = iam.tenant_id(kowalski.id);
    let anna = in_kowalski.create_user().await.unwrap();
    let first = in_kowalski
        .user_id(anna.id)
        .delete()
        .await
        .unwrap()
        .unwrap();
    // The tenant shortens its window meanwhile: Anna keeps the date she was given.
    settings.set(kowalski.id, WINDOW, days(3));

    let second = in_kowalski
        .user_id(anna.id)
        .delete()
        .await
        .unwrap()
        .unwrap();

    assert_eq!(second, first);
    assert_eq!(row_of(&pool, anna.id).await, first);
}

#[sqlx::test(migrations = false)]
async fn a_suspended_user_waits_for_deletion_too(pool: PgPool) {
    let iam = iam_with_settings(&pool, RecordedSettings::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let anna = iam.tenant_id(kowalski.id).create_user().await.unwrap();
    set_state_of_user(&pool, anna.id, "suspended").await;

    let anna = iam
        .tenant_id(kowalski.id)
        .user_id(anna.id)
        .delete()
        .await
        .unwrap()
        .unwrap();

    assert_eq!(anna.state, State::PendingDeletion);
    assert_eq!(anna.deletes_at, Some(deletes_after(&anna, 30)));
}

#[sqlx::test(migrations = false)]
async fn an_id_of_no_user_is_none_and_changes_nothing(pool: PgPool) {
    let iam = iam_with_settings(&pool, RecordedSettings::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let anna = iam.tenant_id(kowalski.id).create_user().await.unwrap();

    let nobody = an_id_of_nothing(&pool).await;
    let deleted = iam
        .tenant_id(kowalski.id)
        .user_id(nobody)
        .delete()
        .await
        .unwrap();

    assert_eq!(deleted, None);
    assert_eq!(row_of(&pool, anna.id).await, anna);
}

#[sqlx::test(migrations = false)]
async fn a_user_of_another_tenant_is_none_and_stays_as_it_was(pool: PgPool) {
    let iam = iam_with_settings(&pool, RecordedSettings::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let nowak = iam.create_tenant("Nowak S.A.").await.unwrap();
    let jan = iam.tenant_id(nowak.id).create_user().await.unwrap();

    let deleted = iam
        .tenant_id(kowalski.id)
        .user_id(jan.id)
        .delete()
        .await
        .unwrap();

    assert_eq!(deleted, None);
    assert_eq!(row_of(&pool, jan.id).await, jan);
}

#[sqlx::test(migrations = false)]
async fn a_tenant_that_does_not_exist_is_an_error(pool: PgPool) {
    let iam = iam_with_settings(&pool, RecordedSettings::default()).await;
    let nowak = iam.create_tenant("Nowak S.A.").await.unwrap();
    let jan = iam.tenant_id(nowak.id).create_user().await.unwrap();
    let nobody = an_id_of_nothing(&pool).await;

    let refused = iam
        .tenant_id(nobody)
        .user_id(jan.id)
        .delete()
        .await
        .unwrap_err();

    assert!(matches!(refused, IamError::NoSuchTenant), "{refused}");
    assert_eq!(row_of(&pool, jan.id).await, jan);
}

#[sqlx::test(migrations = false)]
async fn a_tenant_that_is_not_active_refuses_and_its_user_stays_as_it_was(pool: PgPool) {
    let iam = iam_with_settings(&pool, RecordedSettings::default()).await;
    for (word, state) in [
        ("suspended", State::Suspended),
        ("pending_deletion", State::PendingDeletion),
    ] {
        let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
        let anna = iam.tenant_id(kowalski.id).create_user().await.unwrap();
        set_state_of_tenant(&pool, kowalski.id, word).await;
        let nobody = an_id_of_nothing(&pool).await;

        for id in [anna.id, nobody] {
            let refused = iam
                .tenant_id(kowalski.id)
                .user_id(id)
                .delete()
                .await
                .unwrap_err();
            assert!(
                matches!(refused, IamError::TenantNotActive(found) if found == state),
                "{word}: {refused}"
            );
        }
        assert_eq!(row_of(&pool, anna.id).await, anna, "{word}");
    }
}

#[sqlx::test(migrations = false)]
async fn a_state_of_the_tenant_the_module_does_not_know_is_an_error(pool: PgPool) {
    let iam = iam_with_settings(&pool, RecordedSettings::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let anna = iam.tenant_id(kowalski.id).create_user().await.unwrap();
    set_state_of_tenant(&pool, kowalski.id, "closed").await;

    let refused = iam
        .tenant_id(kowalski.id)
        .user_id(anna.id)
        .delete()
        .await
        .unwrap_err();

    assert!(
        matches!(&refused, IamError::UnknownState(word) if word == "closed"),
        "{refused}"
    );
    assert_eq!(row_of(&pool, anna.id).await, anna);
}

#[sqlx::test(migrations = false)]
async fn a_state_of_the_user_the_module_does_not_know_is_an_error_and_stays(pool: PgPool) {
    let iam = iam_with_settings(&pool, RecordedSettings::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let anna = iam.tenant_id(kowalski.id).create_user().await.unwrap();
    set_state_of_user(&pool, anna.id, "closed").await;

    let refused = iam
        .tenant_id(kowalski.id)
        .user_id(anna.id)
        .delete()
        .await
        .unwrap_err();

    assert!(
        matches!(&refused, IamError::UnknownState(word) if word == "closed"),
        "{refused}"
    );
    let (state, deletes_at): (String, Option<jiff_sqlx::Timestamp>) =
        sqlx::query_as("SELECT state, deletes_at FROM zgm_iam.users WHERE id = $1")
            .bind(anna.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(state, "closed");
    assert!(deletes_at.is_none());
}

#[sqlx::test(migrations = false)]
async fn an_error_of_the_settings_is_an_error_and_changes_nothing(pool: PgPool) {
    let iam = iam_with_settings(&pool, FailingSettings).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let anna = iam.tenant_id(kowalski.id).create_user().await.unwrap();

    let refused = iam
        .tenant_id(kowalski.id)
        .user_id(anna.id)
        .delete()
        .await
        .unwrap_err();

    assert!(matches!(refused, IamError::Settings(_)), "{refused}");
    assert_eq!(
        refused.to_string(),
        "the settings of tenants failed: the settings did not answer"
    );
    assert_eq!(row_of(&pool, anna.id).await, anna);
}

#[sqlx::test(migrations = false)]
async fn a_date_of_deletion_past_the_dates_the_module_holds_is_an_error_and_changes_nothing(
    pool: PgPool,
) {
    let settings = RecordedSettings::default();
    let iam = iam_with_settings(&pool, settings.clone()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let anna = iam.tenant_id(kowalski.id).create_user().await.unwrap();

    // 9 000 years: PostgreSQL would keep the year 11 000, the module cannot read it back.
    // The largest `Duration`: no date at all.
    for window in [days(9_000 * 365), Duration::MAX] {
        settings.set(kowalski.id, WINDOW, window);

        let refused = iam
            .tenant_id(kowalski.id)
            .user_id(anna.id)
            .delete()
            .await
            .unwrap_err();

        assert!(
            matches!(refused, IamError::DeletionDateOutOfRange),
            "{window:?}: {refused}"
        );
        assert_eq!(row_of(&pool, anna.id).await, anna, "{window:?}");
    }
}

#[sqlx::test(migrations = false)]
async fn a_window_up_to_the_year_9999_is_kept(pool: PgPool) {
    let settings = RecordedSettings::default();
    let iam = iam_with_settings(&pool, settings.clone()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    // 7 000 years from now is still before the end of the year 9999.
    settings.set(kowalski.id, WINDOW, days(7_000 * 365));
    let anna = iam.tenant_id(kowalski.id).create_user().await.unwrap();

    let anna = iam
        .tenant_id(kowalski.id)
        .user_id(anna.id)
        .delete()
        .await
        .unwrap()
        .unwrap();

    assert_eq!(anna.deletes_at, Some(deletes_after(&anna, 7_000 * 365)));
    assert_eq!(row_of(&pool, anna.id).await, anna);
}

#[sqlx::test(migrations = false)]
async fn a_tenant_suspended_while_its_user_is_deleted_refuses_and_the_user_stays(pool: PgPool) {
    let iam = iam_with_settings(&pool, RecordedSettings::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let anna = iam.tenant_id(kowalski.id).create_user().await.unwrap();

    // Somebody suspends the tenant and has not committed yet.
    let mut suspension = pool.begin().await.unwrap();
    sqlx::query("UPDATE zgm_iam.tenants SET state = 'suspended' WHERE id = $1")
        .bind(kowalski.id)
        .execute(&mut *suspension)
        .await
        .unwrap();

    // Meanwhile the user is being deleted.
    let in_kowalski = iam.tenant_id(kowalski.id);
    let deletion = tokio::spawn(async move { in_kowalski.user_id(anna.id).delete().await });

    // The deletion must wait for the suspension, not change the user past it.
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
        "delete_user did not wait for the suspension of the tenant"
    );

    suspension.commit().await.unwrap();
    let refused = deletion.await.unwrap().unwrap_err();

    assert!(
        matches!(refused, IamError::TenantNotActive(State::Suspended)),
        "{refused}"
    );
    assert_eq!(row_of(&pool, anna.id).await, anna);
}
