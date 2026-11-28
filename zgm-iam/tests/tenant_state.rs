//! `Iam::tenant_id(id).set_state(state)` and `.delete()`: a tenant is blocked, unblocked,
//! deleted after its window, or brought back from waiting for deletion.

mod common;

use std::time::Duration;

use jiff::SignedDuration;
use sqlx::PgPool;
use zgm_iam::{Iam, IamError, State, Tenant};

use common::{
    FailingCipher, FailingSettings, GarbledCipher, RecordedSettings, an_id_of_nothing, iam_with,
    iam_with_settings, set_state_of_tenant, tenant_row_of,
};

const WINDOW: &str = "iam.tenant_deletion_window";
const DAY: u64 = 24 * 60 * 60;

fn days(count: u64) -> Duration {
    Duration::from_secs(count * DAY)
}

/// The date a tenant changed at `tenant.updated_at` must be deleted at, with a window of
/// `count` days.
fn deletes_after(tenant: &Tenant, count: u64) -> jiff::Timestamp {
    tenant
        .updated_at
        .checked_add(SignedDuration::try_from(days(count)).unwrap())
        .unwrap()
}

/// The module with the tenant Kowalski, and the settings it was built with.
async fn kowalski(pool: &PgPool) -> (Iam, Tenant, RecordedSettings) {
    let settings = RecordedSettings::default();
    let iam = iam_with_settings(pool, settings.clone()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    (iam, kowalski, settings)
}

#[sqlx::test(migrations = false)]
async fn an_active_tenant_waits_for_deletion_90_days_from_the_moment_of_the_change(pool: PgPool) {
    let (iam, kowalski, _settings) = kowalski(&pool).await;

    let deleted = iam.tenant_id(kowalski.id).delete().await.unwrap().unwrap();

    assert_eq!(deleted.id, kowalski.id);
    assert_eq!(deleted.name, "Kowalski sp. z o.o.");
    assert_eq!(deleted.state, State::PendingDeletion);
    assert_eq!(deleted.created_at, kowalski.created_at);
    assert!(deleted.updated_at > kowalski.updated_at);
    assert_eq!(deleted.deletes_at, Some(deletes_after(&deleted, 90)));
    assert_eq!(deleted, tenant_row_of(&pool, kowalski.id).await);
}

#[sqlx::test(migrations = false)]
async fn the_window_is_the_one_the_tenant_set_and_asked_once(pool: PgPool) {
    let (iam, kowalski, settings) = kowalski(&pool).await;
    settings.set(kowalski.id, WINDOW, days(14));

    let deleted = iam.tenant_id(kowalski.id).delete().await.unwrap().unwrap();

    assert_eq!(deleted.deletes_at, Some(deletes_after(&deleted, 14)));
    assert_eq!(
        *settings.asked.lock().unwrap(),
        [(kowalski.id, WINDOW.to_owned())]
    );
}

#[sqlx::test(migrations = false)]
async fn a_suspended_tenant_waits_for_deletion_too(pool: PgPool) {
    let (iam, kowalski, _settings) = kowalski(&pool).await;
    let in_kowalski = iam.tenant_id(kowalski.id);
    in_kowalski.set_state(State::Suspended).await.unwrap();

    let deleted = in_kowalski.delete().await.unwrap().unwrap();

    assert_eq!(deleted.state, State::PendingDeletion);
    assert_eq!(deleted.deletes_at, Some(deletes_after(&deleted, 90)));
}

#[sqlx::test(migrations = false)]
async fn a_tenant_that_waits_for_deletion_already_stays_as_it_was(pool: PgPool) {
    let (iam, kowalski, settings) = kowalski(&pool).await;
    let in_kowalski = iam.tenant_id(kowalski.id);
    let first = in_kowalski.delete().await.unwrap().unwrap();
    // The tenant shortens its window meanwhile: it keeps the date it was given.
    settings.set(kowalski.id, WINDOW, days(7));

    let second = in_kowalski.delete().await.unwrap().unwrap();

    assert_eq!(second, first);
    assert_eq!(tenant_row_of(&pool, kowalski.id).await, first);
}

#[sqlx::test(migrations = false)]
async fn a_tenant_is_suspended_and_then_active_again(pool: PgPool) {
    let (iam, kowalski, _settings) = kowalski(&pool).await;
    let in_kowalski = iam.tenant_id(kowalski.id);

    let suspended = in_kowalski
        .set_state(State::Suspended)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(suspended.state, State::Suspended);
    assert_eq!(suspended.name, "Kowalski sp. z o.o.");
    assert_eq!(suspended.deletes_at, None);
    assert!(suspended.updated_at > kowalski.updated_at);
    assert_eq!(suspended, tenant_row_of(&pool, kowalski.id).await);
    // Nothing else works in it meanwhile.
    let refused = in_kowalski.create_user().await.unwrap_err();
    assert!(
        matches!(refused, IamError::TenantNotActive(State::Suspended)),
        "{refused}"
    );

    let active = in_kowalski.set_state(State::Active).await.unwrap().unwrap();
    assert_eq!(active.state, State::Active);
    assert!(active.updated_at > suspended.updated_at);
    assert_eq!(active, tenant_row_of(&pool, kowalski.id).await);
    in_kowalski.create_user().await.unwrap();
}

#[sqlx::test(migrations = false)]
async fn a_tenant_waiting_for_deletion_comes_back_active_or_suspended_without_a_date(pool: PgPool) {
    let (iam, kowalski, _settings) = kowalski(&pool).await;
    let in_kowalski = iam.tenant_id(kowalski.id);

    for state in [State::Active, State::Suspended] {
        let deleted = in_kowalski.delete().await.unwrap().unwrap();
        assert_eq!(deleted.state, State::PendingDeletion);

        let back = in_kowalski.set_state(state).await.unwrap().unwrap();

        assert_eq!(back.state, state);
        assert_eq!(back.deletes_at, None);
        assert!(back.updated_at > deleted.updated_at);
        assert_eq!(back, tenant_row_of(&pool, kowalski.id).await);
    }
}

#[sqlx::test(migrations = false)]
async fn the_same_state_again_changes_nothing_not_even_updated_at(pool: PgPool) {
    let (iam, kowalski, _settings) = kowalski(&pool).await;
    let in_kowalski = iam.tenant_id(kowalski.id);

    let same = in_kowalski.set_state(State::Active).await.unwrap().unwrap();

    assert_eq!(same, kowalski);
    assert_eq!(tenant_row_of(&pool, kowalski.id).await, kowalski);
}

#[sqlx::test(migrations = false)]
async fn waiting_for_deletion_is_refused_and_changes_nothing(pool: PgPool) {
    let (iam, kowalski, _settings) = kowalski(&pool).await;

    let refused = iam
        .tenant_id(kowalski.id)
        .set_state(State::PendingDeletion)
        .await
        .unwrap_err();

    assert!(
        matches!(refused, IamError::NotSettable(State::PendingDeletion)),
        "{refused}"
    );
    assert_eq!(tenant_row_of(&pool, kowalski.id).await, kowalski);
}

#[sqlx::test(migrations = false)]
async fn a_tenant_that_does_not_exist_is_none(pool: PgPool) {
    let (iam, kowalski, _settings) = kowalski(&pool).await;
    let nobody = an_id_of_nothing(&pool).await;

    assert_eq!(iam.tenant_id(nobody).delete().await.unwrap(), None);
    assert_eq!(
        iam.tenant_id(nobody)
            .set_state(State::Suspended)
            .await
            .unwrap(),
        None
    );
    assert_eq!(tenant_row_of(&pool, kowalski.id).await, kowalski);
}

#[sqlx::test(migrations = false)]
async fn a_state_of_the_tenant_the_module_does_not_know_is_an_error_and_stays(pool: PgPool) {
    let (iam, kowalski, _settings) = kowalski(&pool).await;
    set_state_of_tenant(&pool, kowalski.id, "closed").await;
    let in_kowalski = iam.tenant_id(kowalski.id);

    let refused = in_kowalski.set_state(State::Active).await.unwrap_err();
    assert!(
        matches!(&refused, IamError::UnknownState(word) if word == "closed"),
        "{refused}"
    );
    let refused = in_kowalski.delete().await.unwrap_err();
    assert!(
        matches!(&refused, IamError::UnknownState(word) if word == "closed"),
        "{refused}"
    );

    let word: String = sqlx::query_scalar("SELECT state FROM zgm_iam.tenants WHERE id = $1")
        .bind(kowalski.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(word, "closed");
}

#[sqlx::test(migrations = false)]
async fn when_the_name_cannot_be_decrypted_nothing_changes(pool: PgPool) {
    let (_iam, kowalski, _settings) = kowalski(&pool).await;
    let without_the_key = iam_with(&pool, FailingCipher).await;
    let with_another_key = iam_with(&pool, GarbledCipher).await;

    let refused = without_the_key
        .tenant_id(kowalski.id)
        .set_state(State::Suspended)
        .await
        .unwrap_err();
    assert!(matches!(refused, IamError::Cipher(_)), "{refused}");
    let refused = without_the_key
        .tenant_id(kowalski.id)
        .delete()
        .await
        .unwrap_err();
    assert!(matches!(refused, IamError::Cipher(_)), "{refused}");
    let refused = with_another_key
        .tenant_id(kowalski.id)
        .set_state(State::Suspended)
        .await
        .unwrap_err();
    assert!(matches!(refused, IamError::UnreadableName), "{refused}");

    assert_eq!(tenant_row_of(&pool, kowalski.id).await, kowalski);
}

#[sqlx::test(migrations = false)]
async fn an_error_of_the_settings_is_an_error_and_changes_nothing(pool: PgPool) {
    let (_iam, kowalski, _settings) = kowalski(&pool).await;
    let iam = iam_with_settings(&pool, FailingSettings).await;

    let refused = iam.tenant_id(kowalski.id).delete().await.unwrap_err();

    assert!(matches!(refused, IamError::Settings(_)), "{refused}");
    assert_eq!(tenant_row_of(&pool, kowalski.id).await, kowalski);
}

#[sqlx::test(migrations = false)]
async fn a_date_of_deletion_past_the_dates_the_module_holds_is_an_error_and_changes_nothing(
    pool: PgPool,
) {
    let (iam, kowalski, settings) = kowalski(&pool).await;
    settings.set(kowalski.id, WINDOW, Duration::MAX);

    let refused = iam.tenant_id(kowalski.id).delete().await.unwrap_err();

    assert!(
        matches!(refused, IamError::DeletionDateOutOfRange),
        "{refused}"
    );
    assert_eq!(tenant_row_of(&pool, kowalski.id).await, kowalski);
}

#[sqlx::test(migrations = false)]
async fn the_users_stay_as_they_were_while_the_tenant_waits(pool: PgPool) {
    let (iam, kowalski, _settings) = kowalski(&pool).await;
    let in_kowalski = iam.tenant_id(kowalski.id);
    let anna = in_kowalski.create_user().await.unwrap();

    in_kowalski.delete().await.unwrap();
    in_kowalski.set_state(State::Active).await.unwrap();

    assert_eq!(
        in_kowalski.user_id(anna.id).get().await.unwrap(),
        Some(anna)
    );
}
