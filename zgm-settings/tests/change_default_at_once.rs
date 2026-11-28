//! `change_default` under load: many changes of one default at once, the same change from
//! many copies at once, lists of tenants while a default and its bounds change, and starts
//! of a module while a person changes its default.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tokio::task::JoinSet;
use uuid::Uuid;
use zgm_settings::{DefaultSetting, Settings, Value};

use common::{IamSetting, days, default_row};

const WINDOW: IamSetting = IamSetting::UserDeletionWindow;
const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// The tenant of a number.
fn tenant(number: u64) -> Uuid {
    Uuid::from_u128(0x019ab000_0000_7000_8000_000000000000 + u128::from(number))
}

fn window(value: Duration, min: Option<Duration>, max: Option<Duration>) -> DefaultSetting {
    DefaultSetting::Duration { value, min, max }
}

/// The window as the module gives it: 30 days, at least a day and at most a year.
fn as_the_module_gives_it() -> DefaultSetting {
    window(DAY * 30, Some(DAY), Some(DAY * 365))
}

/// The module with the deletion window as [`as_the_module_gives_it`].
async fn settings_on(pool: &PgPool) -> Settings {
    zgm_settings::migrate(pool).await.unwrap();
    let settings = Settings::builder().pool(pool.clone()).build();
    settings
        .add_default(WINDOW, as_the_module_gives_it())
        .await
        .unwrap();
    settings
}

/// Copies of a program on one database, each with a pool of its own of one connection, so
/// that each copy talks to the database on its own.
async fn copies_of_the_program(pool: &PgPool, count: usize) -> Vec<Settings> {
    let mut copies = Vec::new();
    for _ in 0..count {
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_with((*pool.connect_options()).clone())
            .await
            .unwrap();
        copies.push(Settings::builder().pool(pool).build());
    }
    copies
}

#[sqlx::test(migrations = false)]
async fn many_changes_of_one_default_at_once_end_with_one_of_them(pool: PgPool) {
    const CHANGES: u64 = 50;
    settings_on(&pool).await;
    let copies = copies_of_the_program(&pool, 16).await;

    // 101 to 150 days, none of them the 30 the database has.
    let mut changes = JoinSet::new();
    for number in 1..=CHANGES {
        let copy = copies[usize::try_from(number).unwrap() % copies.len()].clone();
        changes.spawn(async move {
            copy.change_default(
                WINDOW,
                window(
                    DAY * u32::try_from(100 + number).unwrap(),
                    Some(DAY),
                    Some(DAY * 365),
                ),
            )
            .await
        });
    }
    let answers = changes.join_all().await;

    // Every change wrote over the one before it; none failed.
    assert!(
        answers.iter().all(|answer| matches!(answer, Ok(true))),
        "{answers:?}"
    );
    let kept = copies[0].defaults().await.unwrap()[WINDOW.as_ref()].clone();
    assert!(
        (1..=CHANGES).any(|number| kept == days(100 + number)),
        "{kept:?}"
    );
}

#[sqlx::test(migrations = false)]
async fn the_same_change_from_many_copies_at_once_is_one_change(pool: PgPool) {
    const COPIES: usize = 16;
    const TIMES: usize = 5;
    settings_on(&pool).await;
    let copies = copies_of_the_program(&pool, COPIES).await;

    let mut changes = JoinSet::new();
    for copy in copies {
        changes.spawn(async move {
            let mut answers = Vec::new();
            for _ in 0..TIMES {
                answers.push(
                    copy.change_default(WINDOW, window(DAY * 14, Some(DAY * 7), Some(DAY * 90)))
                        .await
                        .unwrap(),
                );
            }
            answers
        });
    }
    let answers: Vec<bool> = changes.join_all().await.into_iter().flatten().collect();

    // The statement locks the row, so every change after the first finds the same default
    // and bounds there, and writes nothing.
    assert_eq!(answers.len(), COPIES * TIMES);
    assert_eq!(answers.iter().filter(|changed| **changed).count(), 1);
    assert_eq!(
        default_row(&pool, WINDOW).await,
        Some((
            "duration".to_owned(),
            "1209600".to_owned(),
            Some(604_800),
            Some(7_776_000)
        ))
    );
}

#[sqlx::test(migrations = false)]
async fn lists_while_the_default_and_its_bounds_change_see_the_old_three_or_the_new(pool: PgPool) {
    const TENANTS: u64 = 16;
    const LISTS: u64 = 200;
    let settings = settings_on(&pool).await;
    let done = Arc::new(AtomicU64::new(0));

    let mut lists = JoinSet::new();
    for number in 0..TENANTS {
        let in_tenant = settings.tenant_id(tenant(number));
        let done = Arc::clone(&done);
        lists.spawn(async move {
            // Counted also when it fails, so that the wait below ends and the test fails on
            // the error.
            let mut seen = Vec::new();
            for _ in 0..LISTS {
                let listed = in_tenant.list().await;
                done.fetch_add(1, Ordering::Relaxed);
                seen.push(listed.map(|listed| {
                    let window = listed.into_iter().next().unwrap();
                    (window.default, window.min, window.max)
                }));
            }
            seen
        });
    }
    // 14 days, at least a week and at most 90 days, once a quarter of the lists are done.
    while done.load(Ordering::Relaxed) < TENANTS * LISTS / 4 {
        tokio::task::yield_now().await;
    }
    assert!(
        settings
            .change_default(WINDOW, window(DAY * 14, Some(DAY * 7), Some(DAY * 90)))
            .await
            .unwrap()
    );

    let old: (Value, Option<Value>, Option<Value>) = (days(30), Some(days(1)), Some(days(365)));
    let new: (Value, Option<Value>, Option<Value>) = (days(14), Some(days(7)), Some(days(90)));
    let mut old_in_all = 0;
    let mut new_in_all = 0;
    for seen in lists.join_all().await {
        let seen: Vec<_> = seen.into_iter().collect::<Result<_, _>>().unwrap();
        // The default and both bounds change together, and once the new ones are seen, the
        // old never again.
        let first_new = seen
            .iter()
            .position(|three| *three == new)
            .unwrap_or(seen.len());
        assert!(
            seen[..first_new].iter().all(|three| *three == old),
            "{seen:?}"
        );
        assert!(
            seen[first_new..].iter().all(|three| *three == new),
            "{seen:?}"
        );
        old_in_all += first_new;
        new_in_all += seen.len() - first_new;
    }
    assert!(
        old_in_all >= usize::try_from(TENANTS * LISTS / 4).unwrap() && new_in_all > 0,
        "the change came in the middle of the lists"
    );
}

#[sqlx::test(migrations = false)]
async fn starts_of_the_module_while_a_person_changes_the_default_leave_the_change(pool: PgPool) {
    const COPIES: usize = 16;
    const STARTS: usize = 20;
    let settings = settings_on(&pool).await;
    let copies = copies_of_the_program(&pool, COPIES).await;
    let done = Arc::new(AtomicU64::new(0));

    // Every copy starts again and again, giving the default as in the code of the module.
    let mut starts = JoinSet::new();
    for copy in copies {
        let done = Arc::clone(&done);
        starts.spawn(async move {
            let mut written = Vec::new();
            for _ in 0..STARTS {
                let answer = copy.add_default(WINDOW, as_the_module_gives_it()).await;
                done.fetch_add(1, Ordering::Relaxed);
                written.push(answer.map_err(|error| error.to_string()));
            }
            written
        });
    }
    while done.load(Ordering::Relaxed) < u64::try_from(COPIES * STARTS / 4).unwrap() {
        tokio::task::yield_now().await;
    }
    assert!(
        settings
            .change_default(WINDOW, window(DAY * 14, Some(DAY * 7), Some(DAY * 90)))
            .await
            .unwrap()
    );

    let written: Vec<bool> = starts
        .join_all()
        .await
        .into_iter()
        .flatten()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(written.len(), COPIES * STARTS);
    assert!(written.iter().all(|written| !written), "{written:?}");
    assert_eq!(
        default_row(&pool, WINDOW).await,
        Some((
            "duration".to_owned(),
            "1209600".to_owned(),
            Some(604_800),
            Some(7_776_000)
        ))
    );
}
