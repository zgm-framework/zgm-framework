//! `restore_default` under load: many tenants restoring at once, one tenant restoring many
//! times at once, and restores and sets of one tenant at once.

mod common;

use sqlx::PgPool;
use tokio::task::JoinSet;
use uuid::Uuid;

use common::{IamSetting, NOWAK, days, rows, set_by_hand, settings_on};

const WINDOW: IamSetting = IamSetting::UserDeletionWindow;

/// The tenant of a number.
fn tenant(number: u64) -> Uuid {
    Uuid::from_u128(0x019ab000_0000_7000_8000_000000000000 + u128::from(number))
}

#[sqlx::test(migrations = false)]
async fn many_tenants_restoring_at_once_each_lose_their_row(pool: PgPool) {
    const TENANTS: u64 = 100;
    let settings = settings_on(&pool).await;
    for number in 0..TENANTS {
        set_by_hand(&pool, tenant(number), WINDOW, "7776000").await;
    }

    let mut restores = JoinSet::new();
    for number in 0..TENANTS {
        let in_tenant = settings.tenant_id(tenant(number));
        restores.spawn(async move { in_tenant.restore_default(WINDOW).await.unwrap() });
    }

    assert!(
        restores
            .join_all()
            .await
            .into_iter()
            .all(|restored| restored)
    );
    assert_eq!(rows(&pool).await, 0);
}

#[sqlx::test(migrations = false)]
async fn one_tenant_restoring_many_times_at_once_finds_its_row_once(pool: PgPool) {
    const RESTORES: usize = 50;
    let settings = settings_on(&pool).await;
    set_by_hand(&pool, NOWAK, WINDOW, "7776000").await;

    let mut restores = JoinSet::new();
    for _ in 0..RESTORES {
        let in_nowak = settings.tenant_id(NOWAK);
        restores.spawn(async move { in_nowak.restore_default(WINDOW).await.unwrap() });
    }
    let restored = restores.join_all().await;

    assert_eq!(restored.len(), RESTORES);
    assert_eq!(restored.iter().filter(|restored| **restored).count(), 1);
    assert_eq!(rows(&pool).await, 0);
}

#[sqlx::test(migrations = false)]
async fn restores_and_sets_of_one_tenant_at_once_end_with_one_of_them(pool: PgPool) {
    const PAIRS: usize = 100;
    let settings = settings_on(&pool).await;

    let mut calls = JoinSet::new();
    for _ in 0..PAIRS {
        let in_nowak = settings.tenant_id(NOWAK);
        calls.spawn(async move { in_nowak.set(WINDOW, days(90)).await.map(|_| ()) });
        let in_nowak = settings.tenant_id(NOWAK);
        calls.spawn(async move { in_nowak.restore_default(WINDOW).await.map(|_| ()) });
    }

    // None fails, and the tenant has either its 90 days or the default.
    for call in calls.join_all().await {
        call.unwrap();
    }
    let kept = settings.tenant_id(NOWAK).get(WINDOW).await.unwrap();
    assert!(kept == days(90) || kept == days(30), "{kept:?}");
}
