//! `Iam::tenant_id(...).list_users()`: the users of an active tenant, a page at a time.

mod common;

use sqlx::PgPool;
use uuid::Uuid;
use zgm_iam::{IamError, Order, State, TenantScope, UsersPage};

use common::{
    FlippingCipher, an_id_of_nothing, iam_with, row_of, set_state_of_tenant, set_state_of_user,
};

/// Writes users of two tenants by hand, one of each in turn, so that the users of one tenant
/// lie between the users of the other.
async fn insert_users_of_both(pool: &PgPool, first: Uuid, second: Uuid, of_each: i64) {
    sqlx::query(
        "INSERT INTO zgm_iam.users (tenant_id) \
         SELECT CASE WHEN g % 2 = 0 THEN $1 ELSE $2 END::uuid \
         FROM generate_series(1, $3 * 2) AS g",
    )
    .bind(first)
    .bind(second)
    .bind(of_each)
    .execute(pool)
    .await
    .unwrap();
}

/// Writes users of one tenant by hand.
async fn insert_users(pool: &PgPool, tenant_id: Uuid, how_many: i64) {
    sqlx::query(
        "INSERT INTO zgm_iam.users (tenant_id) \
         SELECT $1 FROM generate_series(1, $2)",
    )
    .bind(tenant_id)
    .bind(how_many)
    .execute(pool)
    .await
    .unwrap();
}

/// The ids of the users of a tenant in one state, or in any with `None`, read by hand and put
/// in the given order here, not by the module.
async fn ids_of(pool: &PgPool, tenant_id: Uuid, state: Option<&str>, order: Order) -> Vec<Uuid> {
    let mut ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM zgm_iam.users \
         WHERE tenant_id = $1 AND ($2::text IS NULL OR state = $2)",
    )
    .bind(tenant_id)
    .bind(state)
    .fetch_all(pool)
    .await
    .unwrap();
    ids.sort();
    if order == Order::NewestFirst {
        ids.reverse();
    }
    ids
}

/// Goes through all pages of the list, each from the `next_cursor` of the page before, and
/// gives back how many users each page had and the ids of all of them, in the order of the
/// pages.
async fn walk(
    in_tenant: &TenantScope,
    order: Order,
    state: Option<State>,
) -> (Vec<usize>, Vec<Uuid>) {
    let mut sizes = Vec::new();
    let mut ids = Vec::new();
    let mut after = None;
    loop {
        let page = in_tenant
            .list_users()
            .order(order)
            .maybe_state(state)
            .maybe_after(after)
            .call()
            .await
            .unwrap();
        assert!(sizes.len() < 10, "the pages do not end");
        sizes.push(page.items.len());
        ids.extend(page.items.iter().map(|user| user.id));
        let Some(cursor) = page.next_cursor else {
            return (sizes, ids);
        };
        assert_eq!(Some(cursor), page.items.last().map(|user| user.id));
        after = Some(cursor);
    }
}

/// The first page of each of the four kinds of list: both orders, without a state and with
/// one.
async fn first_page_of_every_kind(in_tenant: &TenantScope) -> Vec<Result<UsersPage, IamError>> {
    let mut pages = Vec::new();
    for order in [Order::NewestFirst, Order::OldestFirst] {
        for state in [None, Some(State::Active)] {
            pages.push(
                in_tenant
                    .list_users()
                    .order(order)
                    .maybe_state(state)
                    .call()
                    .await,
            );
        }
    }
    pages
}

#[sqlx::test(migrations = false)]
async fn without_an_order_the_list_goes_from_the_newest_user(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let in_kowalski = iam.tenant_id(kowalski.id);
    let anna = in_kowalski.create_user().await.unwrap();
    let jan = in_kowalski.create_user().await.unwrap();
    let ewa = in_kowalski.create_user().await.unwrap();

    let page = in_kowalski.list_users().call().await.unwrap();

    assert_eq!(
        page,
        UsersPage {
            items: vec![ewa, jan, anna],
            next_cursor: None,
        }
    );
}

#[sqlx::test(migrations = false)]
async fn newest_first_is_the_order_without_an_order(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let in_kowalski = iam.tenant_id(kowalski.id);
    let anna = in_kowalski.create_user().await.unwrap();
    let jan = in_kowalski.create_user().await.unwrap();

    let page = in_kowalski
        .list_users()
        .order(Order::NewestFirst)
        .call()
        .await
        .unwrap();

    assert_eq!(page.items, vec![jan, anna]);
}

#[sqlx::test(migrations = false)]
async fn oldest_first_goes_from_the_oldest_user(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let in_kowalski = iam.tenant_id(kowalski.id);
    let anna = in_kowalski.create_user().await.unwrap();
    let jan = in_kowalski.create_user().await.unwrap();
    let ewa = in_kowalski.create_user().await.unwrap();

    let page = in_kowalski
        .list_users()
        .order(Order::OldestFirst)
        .call()
        .await
        .unwrap();

    assert_eq!(
        page,
        UsersPage {
            items: vec![anna, jan, ewa],
            next_cursor: None,
        }
    );
}

#[sqlx::test(migrations = false)]
async fn the_pages_give_every_user_of_the_tenant_once_in_both_orders(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let nowak = iam.create_tenant("Nowak i Synowie").await.unwrap();
    insert_users_of_both(&pool, kowalski.id, nowak.id, 250).await;
    let in_kowalski = iam.tenant_id(kowalski.id);

    for order in [Order::NewestFirst, Order::OldestFirst] {
        let (sizes, ids) = walk(&in_kowalski, order, None).await;

        assert_eq!(sizes, vec![100, 100, 50], "{order:?}");
        assert_eq!(
            ids,
            ids_of(&pool, kowalski.id, None, order).await,
            "{order:?}"
        );
    }
}

#[sqlx::test(migrations = false)]
async fn a_hundred_users_are_one_page_and_a_hundred_and_one_are_two(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let in_kowalski = iam.tenant_id(kowalski.id);
    insert_users(&pool, kowalski.id, 100).await;

    let (sizes, _) = walk(&in_kowalski, Order::NewestFirst, None).await;
    assert_eq!(sizes, vec![100]);

    insert_users(&pool, kowalski.id, 1).await;

    let (sizes, _) = walk(&in_kowalski, Order::NewestFirst, None).await;
    assert_eq!(sizes, vec![100, 1]);
}

#[sqlx::test(migrations = false)]
async fn each_user_is_on_the_list_with_its_state_and_dates(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let in_kowalski = iam.tenant_id(kowalski.id);
    let anna = in_kowalski.create_user().await.unwrap();
    let jan = in_kowalski.create_user().await.unwrap();
    let ewa = in_kowalski.create_user().await.unwrap();
    set_state_of_user(&pool, jan.id, "suspended").await;
    sqlx::query(
        "UPDATE zgm_iam.users \
         SET state = 'pending_deletion', deletes_at = statement_timestamp() + interval '30 days' \
         WHERE id = $1",
    )
    .bind(ewa.id)
    .execute(&pool)
    .await
    .unwrap();

    let page = in_kowalski.list_users().call().await.unwrap();

    let ewa = row_of(&pool, ewa.id).await;
    let jan = row_of(&pool, jan.id).await;
    assert!(ewa.deletes_at.is_some());
    assert!(jan.updated_at > jan.created_at);
    assert_eq!(page.items, vec![ewa, jan, anna]);
}

#[sqlx::test(migrations = false)]
async fn the_filter_gives_only_the_users_in_its_state(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let in_kowalski = iam.tenant_id(kowalski.id);
    let anna = in_kowalski.create_user().await.unwrap();
    let jan = in_kowalski.create_user().await.unwrap();
    let ewa = in_kowalski.create_user().await.unwrap();
    set_state_of_user(&pool, jan.id, "suspended").await;
    sqlx::query(
        "UPDATE zgm_iam.users \
         SET state = 'pending_deletion', deletes_at = statement_timestamp() + interval '30 days' \
         WHERE id = $1",
    )
    .bind(ewa.id)
    .execute(&pool)
    .await
    .unwrap();
    let jan = row_of(&pool, jan.id).await;
    let ewa = row_of(&pool, ewa.id).await;

    for order in [Order::NewestFirst, Order::OldestFirst] {
        let in_state = async |state| {
            in_kowalski
                .list_users()
                .order(order)
                .state(state)
                .call()
                .await
                .unwrap()
                .items
        };

        assert_eq!(
            in_state(State::Active).await,
            vec![anna.clone()],
            "{order:?}"
        );
        assert_eq!(
            in_state(State::Suspended).await,
            vec![jan.clone()],
            "{order:?}"
        );
        assert_eq!(
            in_state(State::PendingDeletion).await,
            vec![ewa.clone()],
            "{order:?}"
        );
    }
}

#[sqlx::test(migrations = false)]
async fn the_pages_of_one_state_skip_the_users_in_other_states_and_of_other_tenants(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let nowak = iam.create_tenant("Nowak i Synowie").await.unwrap();
    insert_users_of_both(&pool, kowalski.id, nowak.id, 300).await;
    // Every user of Nowak, and every second user of Kowalski, in the order of the ids.
    sqlx::query(
        "UPDATE zgm_iam.users u SET state = 'suspended' \
         FROM (SELECT id, tenant_id, row_number() OVER (PARTITION BY tenant_id ORDER BY id) AS n \
               FROM zgm_iam.users) r \
         WHERE u.id = r.id AND (r.tenant_id = $2 OR r.n % 2 = 0)",
    )
    .bind(kowalski.id)
    .bind(nowak.id)
    .execute(&pool)
    .await
    .unwrap();
    let in_kowalski = iam.tenant_id(kowalski.id);

    for order in [Order::NewestFirst, Order::OldestFirst] {
        for (state, word) in [(State::Suspended, "suspended"), (State::Active, "active")] {
            let (sizes, ids) = walk(&in_kowalski, order, Some(state)).await;

            assert_eq!(sizes, vec![100, 50], "{order:?} {state:?}");
            assert_eq!(
                ids,
                ids_of(&pool, kowalski.id, Some(word), order).await,
                "{order:?} {state:?}"
            );
        }
    }
}

#[sqlx::test(migrations = false)]
async fn users_of_another_tenant_are_not_on_the_list(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let nowak = iam.create_tenant("Nowak i Synowie").await.unwrap();
    let in_kowalski = iam.tenant_id(kowalski.id);
    let in_nowak = iam.tenant_id(nowak.id);
    let anna = in_kowalski.create_user().await.unwrap();
    let ewa = in_nowak.create_user().await.unwrap();
    let jan = in_kowalski.create_user().await.unwrap();

    let of_kowalski = in_kowalski.list_users().call().await.unwrap();
    let of_nowak = in_nowak.list_users().call().await.unwrap();

    assert_eq!(of_kowalski.items, vec![jan, anna]);
    assert_eq!(of_nowak.items, vec![ewa]);
}

#[sqlx::test(migrations = false)]
async fn a_tenant_without_users_gives_an_empty_page(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();

    for page in first_page_of_every_kind(&iam.tenant_id(kowalski.id)).await {
        assert_eq!(
            page.unwrap(),
            UsersPage {
                items: Vec::new(),
                next_cursor: None,
            }
        );
    }
}

#[sqlx::test(migrations = false)]
async fn a_state_no_user_is_in_gives_an_empty_page(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let in_kowalski = iam.tenant_id(kowalski.id);
    in_kowalski.create_user().await.unwrap();

    for order in [Order::NewestFirst, Order::OldestFirst] {
        let page = in_kowalski
            .list_users()
            .order(order)
            .state(State::Suspended)
            .call()
            .await
            .unwrap();

        assert_eq!(page.items, Vec::new(), "{order:?}");
        assert_eq!(page.next_cursor, None, "{order:?}");
    }
}

#[sqlx::test(migrations = false)]
async fn a_tenant_that_does_not_exist_is_an_error(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let nobody = an_id_of_nothing(&pool).await;

    for page in first_page_of_every_kind(&iam.tenant_id(nobody)).await {
        assert!(matches!(page, Err(IamError::NoSuchTenant)), "{page:?}");
    }
}

#[sqlx::test(migrations = false)]
async fn a_suspended_tenant_is_an_error_though_it_has_users(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let in_kowalski = iam.tenant_id(kowalski.id);
    in_kowalski.create_user().await.unwrap();
    set_state_of_tenant(&pool, kowalski.id, "suspended").await;

    for page in first_page_of_every_kind(&in_kowalski).await {
        assert!(
            matches!(page, Err(IamError::TenantNotActive(State::Suspended))),
            "{page:?}"
        );
    }
}

#[sqlx::test(migrations = false)]
async fn a_tenant_pending_deletion_is_an_error_and_says_so(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let in_kowalski = iam.tenant_id(kowalski.id);
    in_kowalski.create_user().await.unwrap();
    set_state_of_tenant(&pool, kowalski.id, "pending_deletion").await;

    for page in first_page_of_every_kind(&in_kowalski).await {
        assert!(
            matches!(page, Err(IamError::TenantNotActive(State::PendingDeletion))),
            "{page:?}"
        );
    }
}

#[sqlx::test(migrations = false)]
async fn a_state_of_the_tenant_the_module_does_not_know_is_an_error(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    set_state_of_tenant(&pool, kowalski.id, "activ").await;

    for page in first_page_of_every_kind(&iam.tenant_id(kowalski.id)).await {
        assert!(
            matches!(&page, Err(IamError::UnknownState(word)) if word == "activ"),
            "{page:?}"
        );
    }
}

#[sqlx::test(migrations = false)]
async fn a_state_of_a_user_the_module_does_not_know_is_an_error(pool: PgPool) {
    let iam = iam_with(&pool, FlippingCipher::default()).await;
    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await.unwrap();
    let in_kowalski = iam.tenant_id(kowalski.id);
    in_kowalski.create_user().await.unwrap();
    let jan = in_kowalski.create_user().await.unwrap();
    set_state_of_user(&pool, jan.id, "suspendet").await;

    for order in [Order::NewestFirst, Order::OldestFirst] {
        let page = in_kowalski.list_users().order(order).call().await;

        assert!(
            matches!(&page, Err(IamError::UnknownState(word)) if word == "suspendet"),
            "{order:?} {page:?}"
        );
    }
}
