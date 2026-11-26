//! `set` under load: many tenants setting at once, one tenant setting many values at once,
//! and sets while the most is lowered.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use sqlx::PgPool;
use tokio::task::JoinSet;
use uuid::Uuid;
use zgm_settings::{DefaultSetting, Settings, SettingsError};

use common::{IamSetting, NOWAK, days};

const WINDOW: IamSetting = IamSetting::UserDeletionWindow;
const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// The tenant of a number.
fn tenant(number: u64) -> Uuid {
    Uuid::from_u128(0x019ab000_0000_7000_8000_000000000000 + u128::from(number))
}

/// The module with the deletion window: 30 days, at least a day and at most a year.
async fn settings_on(pool: &PgPool) -> Settings {
    zgm_settings::migrate(pool).await.unwrap();
    let settings = Settings::builder().pool(pool.clone()).build();
    settings
        .add_default(
            WINDOW,
            DefaultSetting::Duration {
                value: DAY * 30,
                min: Some(DAY),
                max: Some(DAY * 365),
            },
        )
        .await
        .unwrap();
    settings
}

#[sqlx::test(migrations = false)]
async fn many_tenants_setting_at_once_each_keep_their_own_value(pool: PgPool) {
    const TENANTS: u64 = 100;
    let settings = settings_on(&pool).await;

    let mut sets = JoinSet::new();
    for number in 1..=TENANTS {
        let in_tenant = settings.tenant_id(tenant(number));
        sets.spawn(async move { in_tenant.set(WINDOW, days(number)).await.unwrap() });
    }
    assert!(sets.join_all().await.into_iter().all(|changed| changed));

    for number in 1..=TENANTS {
        assert_eq!(
            settings
                .tenant_id(tenant(number))
                .get(WINDOW)
                .await
                .unwrap(),
            days(number)
        );
    }
}

#[sqlx::test(migrations = false)]
async fn one_tenant_setting_many_values_at_once_keeps_one_of_them(pool: PgPool) {
    const VALUES: u64 = 50;
    let settings = settings_on(&pool).await;

    let mut sets = JoinSet::new();
    for number in 1..=VALUES {
        let in_nowak = settings.tenant_id(NOWAK);
        sets.spawn(async move { in_nowak.set(WINDOW, days(number)).await.unwrap() });
    }
    let changed = sets.join_all().await;

    // Every set wrote over the one before it; none failed.
    assert_eq!(changed.len(), usize::try_from(VALUES).unwrap());
    let kept = settings.tenant_id(NOWAK).get(WINDOW).await.unwrap();
    assert!((1..=VALUES).any(|number| kept == days(number)), "{kept:?}");
}

#[sqlx::test(migrations = false)]
async fn sets_while_the_most_is_lowered_are_taken_and_then_refused(pool: PgPool) {
    const TENANTS: u64 = 16;
    const SETS: u64 = 200;
    let settings = settings_on(&pool).await;
    let done = Arc::new(AtomicU64::new(0));

    let mut sets = JoinSet::new();
    for number in 0..TENANTS {
        let in_tenant = settings.tenant_id(tenant(number));
        let done = Arc::clone(&done);
        sets.spawn(async move {
            // 100 and 101 days in turn: within a year, above 60 days. Counted also when it
            // fails, so that the wait below ends.
            let mut answers = Vec::new();
            for set in 0..SETS {
                let value = days(100 + set % 2);
                let answer = in_tenant.set(WINDOW, value.clone()).await;
                done.fetch_add(1, Ordering::Relaxed);
                answers.push((value, answer));
            }
            (number, answers)
        });
    }
    // The most lowered to 60 days, once a quarter of the sets are done.
    while done.load(Ordering::Relaxed) < TENANTS * SETS / 4 {
        tokio::task::yield_now().await;
    }
    sqlx::query("UPDATE zgm_settings.default_settings SET max = 5184000 WHERE name = $1")
        .bind(WINDOW.as_ref())
        .execute(&pool)
        .await
        .unwrap();

    let mut taken_in_all = 0;
    let mut refused_in_all = 0;
    for (number, answers) in sets.join_all().await {
        // Taken, and then refused for the new most, never taken again; nothing else.
        let first_refused = answers
            .iter()
            .position(|(_, answer)| answer.is_err())
            .unwrap_or(answers.len());
        let (taken, refused) = answers.split_at(first_refused);
        for (value, answer) in taken {
            assert!(answer.is_ok(), "tenant {number}, {value:?}: {answer:?}");
        }
        for (value, answer) in refused {
            assert!(
                matches!(answer, Err(SettingsError::AboveTheMost { max, .. }) if *max == days(60)),
                "tenant {number}, {value:?}: {answer:?}"
            );
        }
        // The tenant keeps the last value taken, above the new most.
        if let Some((last, _)) = taken.last() {
            assert_eq!(
                &settings
                    .tenant_id(tenant(number))
                    .get(WINDOW)
                    .await
                    .unwrap(),
                last
            );
        }
        taken_in_all += taken.len();
        refused_in_all += refused.len();
    }
    assert!(
        taken_in_all >= usize::try_from(TENANTS * SETS / 4).unwrap() && refused_in_all > 0,
        "the change came in the middle of the sets"
    );
}
