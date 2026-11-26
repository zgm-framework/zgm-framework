//! `get` under load: many tenants reading at once, and reads at once while the default
//! changes. Every read is a statement of its own, so it sees the default as it was committed
//! when the statement began.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use sqlx::PgPool;
use tokio::task::JoinSet;
use uuid::Uuid;

use zgm_settings::Value;

use common::{IamSetting, days, set_by_hand, settings_on};

const WINDOW: IamSetting = IamSetting::UserDeletionWindow;

/// The tenant of a number.
fn tenant(number: u64) -> Uuid {
    Uuid::from_u128(0x019ab000_0000_7000_8000_000000000000 + u128::from(number))
}

#[sqlx::test(migrations = false)]
async fn many_tenants_reading_at_once_each_get_their_own_value(pool: PgPool) {
    const TENANTS: u64 = 100;
    let settings = settings_on(&pool).await;
    // A tenant of an odd number keeps as many days as its number; the others have none set.
    for number in (1..TENANTS).step_by(2) {
        set_by_hand(
            &pool,
            tenant(number),
            WINDOW,
            &(number * 86_400).to_string(),
        )
        .await;
    }

    let mut reads = JoinSet::new();
    for number in 0..TENANTS {
        let in_tenant = settings.tenant_id(tenant(number));
        reads.spawn(async move {
            let mut values = Vec::new();
            for _ in 0..10 {
                values.push(in_tenant.get(WINDOW).await.unwrap());
            }
            (number, values)
        });
    }

    for (number, values) in reads.join_all().await {
        let own = if number % 2 == 1 {
            days(number)
        } else {
            days(30)
        };
        assert!(
            values.iter().all(|value| *value == own),
            "tenant {number}: {values:?}"
        );
    }
}

#[sqlx::test(migrations = false)]
async fn reads_while_the_default_changes_see_the_old_value_and_then_the_new(pool: PgPool) {
    const READERS: u64 = 16;
    const READS: u64 = 200;
    let settings = settings_on(&pool).await;
    let done = Arc::new(AtomicU64::new(0));

    let mut reads = JoinSet::new();
    for number in 0..READERS {
        let in_tenant = settings.tenant_id(tenant(number));
        let done = Arc::clone(&done);
        reads.spawn(async move {
            // Counted also when it fails, so that the wait below ends and the test fails
            // on the error.
            let mut values = Vec::new();
            for _ in 0..READS {
                let value = in_tenant.get(WINDOW).await;
                done.fetch_add(1, Ordering::Relaxed);
                values.push(value.map_err(|error| error.to_string()));
            }
            values
        });
    }
    // 45 days, once a quarter of the reads are done, while the others go on.
    while done.load(Ordering::Relaxed) < READERS * READS / 4 {
        tokio::task::yield_now().await;
    }
    sqlx::query("UPDATE zgm_settings.default_settings SET value = '3888000' WHERE name = $1")
        .bind(WINDOW.as_ref())
        .execute(&pool)
        .await
        .unwrap();

    let all: Vec<Vec<Value>> = reads
        .join_all()
        .await
        .into_iter()
        .map(|values| values.into_iter().collect::<Result<_, _>>().unwrap())
        .collect();
    let old = all.iter().flatten().filter(|value| **value == days(30));
    let new = all.iter().flatten().filter(|value| **value == days(45));
    assert!(
        old.count() >= usize::try_from(READERS * READS / 4).unwrap() && new.count() > 0,
        "the change came in the middle of the reads"
    );
    for values in all {
        // Only the old or the new value, and once the new one, never the old again.
        assert!(
            values
                .iter()
                .all(|value| *value == days(30) || *value == days(45)),
            "{values:?}"
        );
        let first_new = values
            .iter()
            .position(|value| *value == days(45))
            .unwrap_or(values.len());
        assert!(
            values[first_new..].iter().all(|value| *value == days(45)),
            "{values:?}"
        );
    }
    // After the change, every tenant without a row of its own reads the new default.
    for number in 0..READERS {
        assert_eq!(
            settings
                .tenant_id(tenant(number))
                .get(WINDOW)
                .await
                .unwrap(),
            days(45)
        );
    }
}
