//! `Limits::get_blocked` and `Limits::unblock` on a real database.

mod common;

use std::time::Duration;

use common::{HOUR, MINUTE, TestLimit, counters, key_version, limits_at_noon, noon};
use sqlx::PgPool;
use uuid::Uuid;
use zgm_limits::{Answer, Blocked, BlockedPage, Limits};

/// The keys of `tenant` blocked now.
async fn blocked_now(limits: &Limits, tenant: Uuid) -> Vec<Blocked> {
    limits
        .get_blocked()
        .tenant_id(tenant)
        .call()
        .await
        .unwrap()
        .items
}

/// The same with the keys whose block is over.
async fn blocked_with_expired(limits: &Limits, tenant: Uuid) -> Vec<Blocked> {
    limits
        .get_blocked()
        .tenant_id(tenant)
        .include_expired(true)
        .call()
        .await
        .unwrap()
        .items
}

/// Blocks the whole tenant in `limit`, which must be full after one attempt.
async fn block(limits: &Limits, limit: TestLimit, tenant: Uuid) {
    let answer = limits
        .attempts(limit)
        .tenant_id(tenant)
        .call()
        .await
        .unwrap()
        .hit()
        .await
        .unwrap();
    assert_eq!(answer, Answer::Allowed);
}

/// Blocks one account of the tenant in `limit`, which must be full after one attempt.
async fn block_account(limits: &Limits, limit: TestLimit, tenant: Uuid, account: Uuid) {
    let answer = limits
        .attempts(limit)
        .tenant_id(tenant)
        .subject_id(account)
        .call()
        .await
        .unwrap()
        .hit()
        .await
        .unwrap();
    assert_eq!(answer, Answer::Allowed);
}

#[sqlx::test]
async fn a_key_is_blocked_from_the_attempt_that_fills_the_window(pool: PgPool) {
    let (limits, clock) = limits_at_noon(&pool);
    let tenant = Uuid::new_v4();
    let account = Uuid::new_v4();
    let attempts = limits
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .subject_id(account)
        .hidden("anna@firma.pl")
        .call()
        .await
        .unwrap();

    // Two attempts, 12:00 and 12:01: there is still room, so nobody is blocked.
    let _counted = attempts.hit().await.unwrap();
    clock.advance(MINUTE);
    let _counted = attempts.hit().await.unwrap();
    assert_eq!(blocked_now(&limits, tenant).await, []);

    // The third, 12:02, fills the window.
    clock.advance(MINUTE);
    let _counted = attempts.hit().await.unwrap();

    let blocked = blocked_now(&limits, tenant).await;
    let expected = Blocked {
        id: blocked[0].id,
        limit_name: "three_per_hour".to_owned(),
        tenant_id: tenant,
        subject_id: account,
        key_hash: blocked[0].key_hash.clone(),
        whole_tenant: false,
        blocked_at: noon() + MINUTE + MINUTE,
        blocked_until: noon() + HOUR,
        expired: false,
    };
    assert_eq!(blocked, std::slice::from_ref(&expected));
    // The hidden part is on the list as a hash, not as the e-mail.
    assert_eq!(key_version(&expected.key_hash), Some(1));

    // A refused attempt changes nothing in the block.
    clock.advance(MINUTE);
    let _counted = attempts.hit().await.unwrap();
    assert_eq!(blocked_now(&limits, tenant).await, [expected]);
}

#[sqlx::test]
async fn a_block_that_is_over_is_listed_only_with_the_expired_ones(pool: PgPool) {
    let (limits, clock) = limits_at_noon(&pool);
    let tenant = Uuid::new_v4();
    block(&limits, TestLimit::OnePerHour, tenant).await;

    // A second before the end the key is still blocked.
    clock.advance(HOUR - Duration::from_secs(1));
    assert_eq!(blocked_now(&limits, tenant).await.len(), 1);

    // At the end, 13:00, the block is over; the row is still there.
    clock.advance(Duration::from_secs(1));
    assert_eq!(blocked_now(&limits, tenant).await, []);
    let with_expired = blocked_with_expired(&limits, tenant).await;
    assert_eq!(with_expired.len(), 1);
    assert!(with_expired[0].expired);
    assert_eq!(with_expired[0].blocked_until, noon() + HOUR);
}

#[sqlx::test]
async fn a_new_window_starts_not_blocked(pool: PgPool) {
    let (limits, clock) = limits_at_noon(&pool);
    let tenant = Uuid::new_v4();
    let attempts = limits
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .call()
        .await
        .unwrap();
    for _ in 0..3 {
        let _counted = attempts.hit().await.unwrap();
    }

    // The first attempt after the end opens a new window in the same row.
    clock.advance(HOUR);
    let _counted = attempts.hit().await.unwrap();

    assert_eq!(blocked_with_expired(&limits, tenant).await, []);
}

#[sqlx::test]
async fn the_list_holds_only_the_keys_of_its_tenant(pool: PgPool) {
    let (limits, _clock) = limits_at_noon(&pool);
    let tenant = Uuid::new_v4();
    let other_tenant = Uuid::new_v4();
    block(&limits, TestLimit::OnePerHour, tenant).await;
    block(&limits, TestLimit::OnePerHour, other_tenant).await;
    block_account(&limits, TestLimit::OnePerHour, other_tenant, Uuid::new_v4()).await;

    let of_tenant = blocked_now(&limits, tenant).await;
    let of_other_tenant = blocked_now(&limits, other_tenant).await;

    assert_eq!(of_tenant.len(), 1);
    assert_eq!(of_other_tenant.len(), 2);
    assert!(of_tenant.iter().all(|blocked| blocked.tenant_id == tenant));
    assert!(
        of_other_tenant
            .iter()
            .all(|blocked| blocked.tenant_id == other_tenant)
    );
    // A tenant with no blocked key has an empty list.
    assert_eq!(blocked_now(&limits, Uuid::new_v4()).await, []);
}

#[sqlx::test]
async fn the_filters_narrow_the_list_to_a_limit_or_to_whole_tenant_counters(pool: PgPool) {
    let (limits, clock) = limits_at_noon(&pool);
    let tenant = Uuid::new_v4();
    let account = Uuid::new_v4();
    // Three blocks in one tenant, a minute apart; the list gives the newest first.
    block(&limits, TestLimit::OnePerHour, tenant).await;
    clock.advance(MINUTE);
    block_account(&limits, TestLimit::OnePerHour, tenant, account).await;
    clock.advance(MINUTE);
    let hidden = limits
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .hidden("anna@firma.pl")
        .call()
        .await
        .unwrap();
    for _ in 0..3 {
        let _counted = hidden.hit().await.unwrap();
    }
    // A key with room left is on no list.
    let _counted = limits
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .subject_id(account)
        .call()
        .await
        .unwrap()
        .hit()
        .await
        .unwrap();

    // What tells the three apart: the limit, the account, and whether there is a hidden part.
    let named = |page: BlockedPage| -> Vec<(String, Uuid, bool, bool)> {
        page.items
            .into_iter()
            .map(|blocked| {
                (
                    blocked.limit_name,
                    blocked.subject_id,
                    blocked.key_hash.is_empty(),
                    blocked.whole_tenant,
                )
            })
            .collect()
    };
    let of_hidden_part = ("three_per_hour".to_owned(), Uuid::nil(), false, false);
    let of_account = ("one_per_hour".to_owned(), account, true, false);
    let of_whole_tenant = ("one_per_hour".to_owned(), Uuid::nil(), true, true);

    let all = limits.get_blocked().tenant_id(tenant).call().await.unwrap();
    assert_eq!(
        named(all),
        [
            of_hidden_part.clone(),
            of_account.clone(),
            of_whole_tenant.clone()
        ]
    );

    let of_the_limit = limits
        .get_blocked()
        .tenant_id(tenant)
        .limit_name(TestLimit::OnePerHour)
        .call()
        .await
        .unwrap();
    assert_eq!(
        named(of_the_limit),
        [of_account.clone(), of_whole_tenant.clone()]
    );

    let whole = limits
        .get_blocked()
        .tenant_id(tenant)
        .whole_tenant(true)
        .call()
        .await
        .unwrap();
    assert_eq!(named(whole), [of_whole_tenant]);

    let others = limits
        .get_blocked()
        .tenant_id(tenant)
        .whole_tenant(false)
        .call()
        .await
        .unwrap();
    assert_eq!(named(others), [of_hidden_part, of_account.clone()]);

    let both_filters = limits
        .get_blocked()
        .tenant_id(tenant)
        .limit_name(TestLimit::OnePerHour)
        .whole_tenant(false)
        .call()
        .await
        .unwrap();
    assert_eq!(named(both_filters), [of_account]);
}

#[sqlx::test]
async fn the_list_comes_in_pages_of_a_hundred(pool: PgPool) {
    let (limits, clock) = limits_at_noon(&pool);
    let tenant = Uuid::new_v4();
    // 250 blocked accounts of one tenant. Every seven of them are blocked at the same moment,
    // so that a page ends in the middle of rows with the same `blocked_at` and the next one has
    // to go on exactly after its last row.
    let mut accounts = Vec::new();
    for number in 0..250 {
        if number % 7 == 0 {
            clock.advance(Duration::from_secs(1));
        }
        let account = Uuid::new_v4();
        block_account(&limits, TestLimit::OnePerHour, tenant, account).await;
        accounts.push(account);
    }
    // The blocked keys of another tenant are on no page.
    block(&limits, TestLimit::OnePerHour, Uuid::new_v4()).await;

    let first = limits.get_blocked().tenant_id(tenant).call().await.unwrap();
    let second = limits
        .get_blocked()
        .tenant_id(tenant)
        .maybe_after(first.next_cursor)
        .call()
        .await
        .unwrap();
    let third = limits
        .get_blocked()
        .tenant_id(tenant)
        .maybe_after(second.next_cursor)
        .call()
        .await
        .unwrap();

    assert_eq!(first.items.len(), 100);
    assert_eq!(second.items.len(), 100);
    assert_eq!(third.items.len(), 50);
    assert!(first.next_cursor.is_some());
    assert!(second.next_cursor.is_some());
    // The last page says that nothing follows.
    assert_eq!(third.next_cursor, None);

    // Together the pages hold every account once, and the newest block comes first.
    let pages = [first.items, second.items, third.items].concat();
    let mut listed: Vec<Uuid> = pages.iter().map(|blocked| blocked.subject_id).collect();
    assert!(pages.is_sorted_by(|earlier, later| earlier.blocked_at >= later.blocked_at));
    listed.sort();
    accounts.sort();
    assert_eq!(listed, accounts);
}

#[sqlx::test]
async fn a_list_that_fills_one_page_exactly_has_no_next_page(pool: PgPool) {
    let (limits, _clock) = limits_at_noon(&pool);
    let tenant = Uuid::new_v4();
    for _ in 0..100 {
        block_account(&limits, TestLimit::OnePerHour, tenant, Uuid::new_v4()).await;
    }

    let page = limits.get_blocked().tenant_id(tenant).call().await.unwrap();

    assert_eq!(page.items.len(), 100);
    assert_eq!(page.next_cursor, None);
}

#[sqlx::test]
async fn unblock_deletes_the_counter_so_the_key_starts_from_nothing(pool: PgPool) {
    let (limits, _clock) = limits_at_noon(&pool);
    let tenant = Uuid::new_v4();
    let account = Uuid::new_v4();
    let other_account = Uuid::new_v4();
    block_account(&limits, TestLimit::OnePerHour, tenant, account).await;
    block_account(&limits, TestLimit::OnePerHour, tenant, other_account).await;
    let blocked = blocked_now(&limits, tenant).await;
    let id = blocked
        .iter()
        .find(|blocked| blocked.subject_id == account)
        .unwrap()
        .id;

    let unblocked = limits.unblock(tenant, id).await.unwrap();

    assert!(unblocked);
    // Only the other account is still blocked, and the unblocked one passes again.
    let still_blocked: Vec<Uuid> = blocked_now(&limits, tenant)
        .await
        .into_iter()
        .map(|blocked| blocked.subject_id)
        .collect();
    assert_eq!(still_blocked, [other_account]);
    let answer = limits
        .attempts(TestLimit::OnePerHour)
        .tenant_id(tenant)
        .subject_id(account)
        .call()
        .await
        .unwrap()
        .hit()
        .await
        .unwrap();
    assert_eq!(answer, Answer::Allowed);
}

#[sqlx::test]
async fn unblock_with_the_id_of_another_tenant_changes_nothing(pool: PgPool) {
    let (limits, _clock) = limits_at_noon(&pool);
    let tenant = Uuid::new_v4();
    let other_tenant = Uuid::new_v4();
    block(&limits, TestLimit::OnePerHour, tenant).await;
    block(&limits, TestLimit::OnePerHour, other_tenant).await;
    let id_of_other_tenant = blocked_now(&limits, other_tenant).await[0].id;

    // An administrator of `tenant` tries the id of a key of `other_tenant`.
    let unblocked = limits.unblock(tenant, id_of_other_tenant).await.unwrap();

    assert!(!unblocked);
    assert_eq!(blocked_now(&limits, other_tenant).await.len(), 1);
    // The right tenant can still unblock it.
    assert!(
        limits
            .unblock(other_tenant, id_of_other_tenant)
            .await
            .unwrap()
    );
}

#[sqlx::test]
async fn unblock_of_a_key_that_is_not_blocked_changes_nothing(pool: PgPool) {
    let (limits, _clock) = limits_at_noon(&pool);
    let tenant = Uuid::new_v4();
    block(&limits, TestLimit::OnePerHour, tenant).await;
    let id = blocked_now(&limits, tenant).await[0].id;
    // A counter with room left: it has an id too, but it is not blocked.
    let _counted = limits
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .call()
        .await
        .unwrap()
        .hit()
        .await
        .unwrap();
    let not_blocked_id: i64 = sqlx::query_scalar(
        "SELECT id FROM zgm_limits.counters WHERE limit_name = 'three_per_hour'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    // Unblocked twice: the second time there is nothing to unblock.
    assert!(limits.unblock(tenant, id).await.unwrap());
    assert!(!limits.unblock(tenant, id).await.unwrap());
    // Neither an id that is not blocked nor one that does not exist deletes anything.
    assert!(!limits.unblock(tenant, not_blocked_id).await.unwrap());
    assert!(!limits.unblock(tenant, i64::MAX).await.unwrap());

    assert_eq!(counters(&pool).await, [(Vec::new(), 1)]);
}
