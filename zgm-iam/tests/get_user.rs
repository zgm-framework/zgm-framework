//! `Iam::tenant_id(...).user_id(id).get()`: a user of an active tenant, or none.

mod common;

use sqlx::PgPool;
use zgm_iam::{IamError, State};

use common::{
    FlippingCipher, an_id_of_nothing, iam_with, row_of, set_state_of_tenant, set_state_of_user,
};

#[sqlx::test(migrations = false)]
async fn get_user_gives_back_each_user_as_create_user_gave_it(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let in_kowalski = iam.tenant_id(kowalski.id);
    let anna = in_kowalski.create_user().await.unwrap();
    let jan = in_kowalski.create_user().await.unwrap();

    assert_eq!(
        in_kowalski.user_id(anna.id).get().await.unwrap(),
        Some(anna)
    );
    assert_eq!(in_kowalski.user_id(jan.id).get().await.unwrap(), Some(jan));
}

#[sqlx::test(migrations = false)]
async fn a_suspended_user_is_given_back_with_its_state(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let in_kowalski = iam.tenant_id(kowalski.id);
    let jan = in_kowalski.create_user().await.unwrap();
    set_state_of_user(&pool, jan.id, "suspended").await;

    let found = in_kowalski.user_id(jan.id).get().await.unwrap().unwrap();

    assert_eq!(found.state, State::Suspended);
    assert!(found.updated_at > found.created_at);
    assert_eq!(found, row_of(&pool, jan.id).await);
}

#[sqlx::test(migrations = false)]
async fn a_user_pending_deletion_is_given_back_with_the_date_of_its_deletion(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let in_kowalski = iam.tenant_id(kowalski.id);
    let anna = in_kowalski.create_user().await.unwrap();
    sqlx::query(
        "UPDATE zgm_iam.users \
         SET state = 'pending_deletion', deletes_at = statement_timestamp() + interval '30 days' \
         WHERE id = $1",
    )
    .bind(anna.id)
    .execute(&pool)
    .await
    .unwrap();

    let found = in_kowalski.user_id(anna.id).get().await.unwrap().unwrap();

    assert_eq!(found.state, State::PendingDeletion);
    assert!(found.deletes_at.is_some_and(|date| date > found.updated_at));
    assert_eq!(found, row_of(&pool, anna.id).await);
}

#[sqlx::test(migrations = false)]
async fn a_user_of_another_tenant_is_none(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let nowak = iam.create_tenant("Nowak i synowie").await.unwrap();
    iam.tenant_id(kowalski.id).create_user().await.unwrap();
    let ewa = iam.tenant_id(nowak.id).create_user().await.unwrap();

    let found = iam
        .tenant_id(kowalski.id)
        .user_id(ewa.id)
        .get()
        .await
        .unwrap();

    assert_eq!(found, None);
}

#[sqlx::test(migrations = false)]
async fn an_id_of_no_user_is_none(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let in_kowalski = iam.tenant_id(kowalski.id);
    in_kowalski.create_user().await.unwrap();

    let found = in_kowalski
        .user_id(an_id_of_nothing(&pool).await)
        .get()
        .await
        .unwrap();

    assert_eq!(found, None);
}

#[sqlx::test(migrations = false)]
async fn a_tenant_that_does_not_exist_is_an_error_even_for_a_user_of_another(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let nowak = iam.create_tenant("Nowak i synowie").await.unwrap();
    let ewa = iam.tenant_id(nowak.id).create_user().await.unwrap();

    let refused = iam
        .tenant_id(an_id_of_nothing(&pool).await)
        .user_id(ewa.id)
        .get()
        .await
        .unwrap_err();

    assert!(matches!(refused, IamError::NoSuchTenant), "{refused}");
}

#[sqlx::test(migrations = false)]
async fn a_suspended_tenant_is_an_error_for_its_user_and_for_an_id_of_no_user(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let in_kowalski = iam.tenant_id(kowalski.id);
    let anna = in_kowalski.create_user().await.unwrap();
    set_state_of_tenant(&pool, kowalski.id, "suspended").await;

    let refused = in_kowalski.user_id(anna.id).get().await.unwrap_err();
    assert!(
        matches!(refused, IamError::TenantNotActive(State::Suspended)),
        "{refused}"
    );

    let refused = in_kowalski
        .user_id(an_id_of_nothing(&pool).await)
        .get()
        .await
        .unwrap_err();
    assert!(
        matches!(refused, IamError::TenantNotActive(State::Suspended)),
        "{refused}"
    );
}

#[sqlx::test(migrations = false)]
async fn a_tenant_pending_deletion_is_an_error_and_says_so(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let in_kowalski = iam.tenant_id(kowalski.id);
    let anna = in_kowalski.create_user().await.unwrap();
    set_state_of_tenant(&pool, kowalski.id, "pending_deletion").await;

    let refused = in_kowalski.user_id(anna.id).get().await.unwrap_err();

    assert!(
        matches!(refused, IamError::TenantNotActive(State::PendingDeletion)),
        "{refused}"
    );
}

#[sqlx::test(migrations = false)]
async fn a_state_of_the_tenant_the_module_does_not_know_is_an_error(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let in_kowalski = iam.tenant_id(kowalski.id);
    let anna = in_kowalski.create_user().await.unwrap();
    set_state_of_tenant(&pool, kowalski.id, "activ").await;

    let refused = in_kowalski.user_id(anna.id).get().await.unwrap_err();

    assert!(
        matches!(&refused, IamError::UnknownState(word) if word == "activ"),
        "{refused}"
    );
}

#[sqlx::test(migrations = false)]
async fn a_state_of_the_user_the_module_does_not_know_is_an_error(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let in_kowalski = iam.tenant_id(kowalski.id);
    let anna = in_kowalski.create_user().await.unwrap();
    set_state_of_user(&pool, anna.id, "suspendet").await;

    let refused = in_kowalski.user_id(anna.id).get().await.unwrap_err();

    assert!(
        matches!(&refused, IamError::UnknownState(word) if word == "suspendet"),
        "{refused}"
    );
}
