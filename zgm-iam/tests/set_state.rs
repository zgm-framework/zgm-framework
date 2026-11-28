//! `Iam::tenant_id(...).user_id(id).set_state(state)`: a user is blocked, unblocked, or
//! brought back from waiting for deletion.

mod common;

use sqlx::PgPool;
use zgm_iam::{Iam, IamError, State, Tenant, User};

use common::{
    RecordedSettings, an_id_of_nothing, iam_with_settings, row_of, set_state_of_tenant,
    set_state_of_user,
};

/// The module with the tenant Kowalski and its active user Anna.
async fn kowalski_with_anna(pool: &PgPool) -> (Iam, Tenant, User) {
    let iam = iam_with_settings(pool, RecordedSettings::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let anna = iam.tenant_id(kowalski.id).create_user().await.unwrap();
    (iam, kowalski, anna)
}

/// Puts the date of deletion of a user in the past, by hand, as when its window ran out and
/// the loop that deletes has not come yet.
async fn deadline_passed(pool: &PgPool, user_id: uuid::Uuid) {
    sqlx::query(
        "UPDATE zgm_iam.users SET deletes_at = statement_timestamp() - interval '1 hour' \
         WHERE id = $1",
    )
    .bind(user_id)
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test(migrations = false)]
async fn an_active_user_is_suspended_and_then_active_again(pool: PgPool) {
    let (iam, kowalski, anna) = kowalski_with_anna(&pool).await;
    let in_kowalski = iam.tenant_id(kowalski.id);

    let suspended = in_kowalski
        .user_id(anna.id)
        .set_state(State::Suspended)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(suspended.id, anna.id);
    assert_eq!(suspended.tenant_id, kowalski.id);
    assert_eq!(suspended.state, State::Suspended);
    assert_eq!(suspended.deletes_at, None);
    assert_eq!(suspended.created_at, anna.created_at);
    assert!(suspended.updated_at > anna.updated_at);
    assert_eq!(suspended, row_of(&pool, anna.id).await);

    let active = in_kowalski
        .user_id(anna.id)
        .set_state(State::Active)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(active.state, State::Active);
    assert!(active.updated_at > suspended.updated_at);
    assert_eq!(active, row_of(&pool, anna.id).await);
}

#[sqlx::test(migrations = false)]
async fn a_user_waiting_for_deletion_comes_back_active_or_suspended_without_a_date(pool: PgPool) {
    let (iam, kowalski, anna) = kowalski_with_anna(&pool).await;
    let in_kowalski = iam.tenant_id(kowalski.id);

    for state in [State::Active, State::Suspended] {
        let deleted = in_kowalski
            .user_id(anna.id)
            .delete()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(deleted.state, State::PendingDeletion);

        let back = in_kowalski
            .user_id(anna.id)
            .set_state(state)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(back.state, state);
        assert_eq!(back.deletes_at, None);
        assert!(back.updated_at > deleted.updated_at);
        assert_eq!(back, row_of(&pool, anna.id).await);
    }
}

#[sqlx::test(migrations = false)]
async fn a_user_whose_date_of_deletion_passed_comes_back_until_the_row_is_gone(pool: PgPool) {
    let (iam, kowalski, anna) = kowalski_with_anna(&pool).await;
    let in_kowalski = iam.tenant_id(kowalski.id);
    in_kowalski.user_id(anna.id).delete().await.unwrap();
    deadline_passed(&pool, anna.id).await;

    let back = in_kowalski
        .user_id(anna.id)
        .set_state(State::Active)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(back.state, State::Active);
    assert_eq!(back.deletes_at, None);
}

#[sqlx::test(migrations = false)]
async fn the_same_state_again_changes_nothing_not_even_updated_at(pool: PgPool) {
    let (iam, kowalski, anna) = kowalski_with_anna(&pool).await;
    let in_kowalski = iam.tenant_id(kowalski.id);

    let same = in_kowalski
        .user_id(anna.id)
        .set_state(State::Active)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(same, anna);

    let suspended = in_kowalski
        .user_id(anna.id)
        .set_state(State::Suspended)
        .await
        .unwrap()
        .unwrap();
    let again = in_kowalski
        .user_id(anna.id)
        .set_state(State::Suspended)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(again, suspended);
    assert_eq!(again, row_of(&pool, anna.id).await);
}

#[sqlx::test(migrations = false)]
async fn waiting_for_deletion_is_refused_and_changes_nothing(pool: PgPool) {
    let (iam, kowalski, anna) = kowalski_with_anna(&pool).await;
    let in_kowalski = iam.tenant_id(kowalski.id);

    let refused = in_kowalski
        .user_id(anna.id)
        .set_state(State::PendingDeletion)
        .await
        .unwrap_err();

    assert!(
        matches!(refused, IamError::NotSettable(State::PendingDeletion)),
        "{refused}"
    );
    assert_eq!(row_of(&pool, anna.id).await, anna);
}

#[sqlx::test(migrations = false)]
async fn an_id_of_no_user_is_none_and_changes_nothing(pool: PgPool) {
    let (iam, kowalski, anna) = kowalski_with_anna(&pool).await;
    let nobody = an_id_of_nothing(&pool).await;

    let found = iam
        .tenant_id(kowalski.id)
        .user_id(nobody)
        .set_state(State::Suspended)
        .await
        .unwrap();

    assert_eq!(found, None);
    assert_eq!(row_of(&pool, anna.id).await, anna);
}

#[sqlx::test(migrations = false)]
async fn a_user_of_another_tenant_is_none_and_stays_as_it_was(pool: PgPool) {
    let (iam, _kowalski, anna) = kowalski_with_anna(&pool).await;
    let nowak = iam.create_tenant("Nowak S.A.").await.unwrap();

    let found = iam
        .tenant_id(nowak.id)
        .user_id(anna.id)
        .set_state(State::Suspended)
        .await
        .unwrap();

    assert_eq!(found, None);
    assert_eq!(row_of(&pool, anna.id).await, anna);
}

#[sqlx::test(migrations = false)]
async fn a_tenant_that_does_not_exist_is_an_error(pool: PgPool) {
    let (iam, _kowalski, anna) = kowalski_with_anna(&pool).await;
    let nobody = an_id_of_nothing(&pool).await;

    let refused = iam
        .tenant_id(nobody)
        .user_id(anna.id)
        .set_state(State::Suspended)
        .await
        .unwrap_err();

    assert!(matches!(refused, IamError::NoSuchTenant), "{refused}");
    assert_eq!(row_of(&pool, anna.id).await, anna);
}

#[sqlx::test(migrations = false)]
async fn a_tenant_that_is_not_active_refuses_and_its_user_stays_as_it_was(pool: PgPool) {
    let (iam, kowalski, anna) = kowalski_with_anna(&pool).await;
    let nobody = an_id_of_nothing(&pool).await;

    for (word, state) in [
        ("suspended", State::Suspended),
        ("pending_deletion", State::PendingDeletion),
    ] {
        set_state_of_tenant(&pool, kowalski.id, word).await;
        for user_id in [anna.id, nobody] {
            let refused = iam
                .tenant_id(kowalski.id)
                .user_id(user_id)
                .set_state(State::Suspended)
                .await
                .unwrap_err();
            assert!(
                matches!(refused, IamError::TenantNotActive(found) if found == state),
                "{word}: {refused}"
            );
        }
        assert_eq!(row_of(&pool, anna.id).await, anna);
    }
}

#[sqlx::test(migrations = false)]
async fn a_state_of_the_user_the_module_does_not_know_is_an_error_and_stays(pool: PgPool) {
    let (iam, kowalski, anna) = kowalski_with_anna(&pool).await;
    set_state_of_user(&pool, anna.id, "closed").await;

    let refused = iam
        .tenant_id(kowalski.id)
        .user_id(anna.id)
        .set_state(State::Active)
        .await
        .unwrap_err();

    assert!(
        matches!(&refused, IamError::UnknownState(word) if word == "closed"),
        "{refused}"
    );
    let word: String = sqlx::query_scalar("SELECT state FROM zgm_iam.users WHERE id = $1")
        .bind(anna.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(word, "closed");
}
