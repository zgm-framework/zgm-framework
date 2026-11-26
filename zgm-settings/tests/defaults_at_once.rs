//! Defaults under load: many copies of a program starting at once, many defaults given at
//! once, and many starts one after another. Every default is written once, and every copy
//! gets what the database has.

use std::collections::BTreeMap;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tokio::task::JoinSet;
use zgm_settings::{DefaultSetting, Settings, Value};

/// The name of one of many settings.
fn setting(number: usize) -> String {
    format!("load.setting_{number:03}")
}

fn number(value: usize) -> DefaultSetting {
    DefaultSetting::Number {
        value: i64::try_from(value).unwrap(),
        min: None,
        max: None,
    }
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

async fn rows(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM zgm_settings.default_settings")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = false)]
async fn many_copies_starting_at_once_write_every_default_once(pool: PgPool) {
    const COPIES: usize = 16;
    const SETTINGS: usize = 50;
    zgm_settings::migrate(&pool).await.unwrap();
    let copies = copies_of_the_program(&pool, COPIES).await;

    // Every copy gives the same settings, each with a number of its own, as versions of a
    // module with other numbers would.
    let mut starts = JoinSet::new();
    for (copy_number, copy) in copies.iter().cloned().enumerate() {
        starts.spawn(async move {
            let mut written = Vec::new();
            for setting_number in 0..SETTINGS {
                if copy
                    .add_default(setting(setting_number), number(copy_number))
                    .await
                    .unwrap()
                {
                    written.push(setting_number);
                }
            }
            (copy_number, written)
        });
    }
    let mut writer_of = BTreeMap::new();
    while let Some(start) = starts.join_next().await {
        let (copy_number, written) = start.unwrap();
        for setting_number in written {
            assert_eq!(
                writer_of.insert(setting_number, copy_number),
                None,
                "{} written twice",
                setting(setting_number)
            );
        }
    }

    assert_eq!(writer_of.len(), SETTINGS);
    assert_eq!(rows(&pool).await, i64::try_from(SETTINGS).unwrap());
    // The database has the number of the copy that wrote each row, and every copy got it.
    let in_the_database: BTreeMap<String, Value> = writer_of
        .iter()
        .map(|(setting_number, copy_number)| {
            (
                setting(*setting_number),
                Value::Number(i64::try_from(*copy_number).unwrap()),
            )
        })
        .collect();
    for copy in &copies {
        assert_eq!(copy.defaults().await.unwrap(), in_the_database);
    }
}

#[sqlx::test(migrations = false)]
async fn many_defaults_given_at_once_through_clones_are_all_written(pool: PgPool) {
    const SETTINGS: usize = 200;
    zgm_settings::migrate(&pool).await.unwrap();
    let settings = Settings::builder().pool(pool.clone()).build();

    let mut given = JoinSet::new();
    for setting_number in 0..SETTINGS {
        let settings = settings.clone();
        given.spawn(async move {
            settings
                .add_default(setting(setting_number), number(setting_number))
                .await
                .unwrap()
        });
    }
    let written: Vec<bool> = given.join_all().await;

    assert!(written.iter().all(|written| *written));
    assert_eq!(rows(&pool).await, i64::try_from(SETTINGS).unwrap());
    let expected: BTreeMap<String, Value> = (0..SETTINGS)
        .map(|setting_number| {
            (
                setting(setting_number),
                Value::Number(i64::try_from(setting_number).unwrap()),
            )
        })
        .collect();
    assert_eq!(settings.defaults().await.unwrap(), expected);
}

#[sqlx::test(migrations = false)]
async fn many_starts_one_after_another_leave_every_row_as_the_first_wrote_it(pool: PgPool) {
    const STARTS: usize = 20;
    const SETTINGS: usize = 10;
    zgm_settings::migrate(&pool).await.unwrap();

    for start in 0..STARTS {
        // Every start a new version, with numbers of its own.
        let settings = Settings::builder().pool(pool.clone()).build();
        for setting_number in 0..SETTINGS {
            let written = settings
                .add_default(
                    setting(setting_number),
                    number(start * 100 + setting_number),
                )
                .await
                .unwrap();
            assert_eq!(written, start == 0, "start {start}");
        }
        let first: BTreeMap<String, Value> = (0..SETTINGS)
            .map(|setting_number| {
                (
                    setting(setting_number),
                    Value::Number(i64::try_from(setting_number).unwrap()),
                )
            })
            .collect();
        assert_eq!(settings.defaults().await.unwrap(), first, "start {start}");
    }

    let untouched: bool = sqlx::query_scalar(
        "SELECT bool_and(updated_at = created_at) FROM zgm_settings.default_settings",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(untouched);
    assert_eq!(rows(&pool).await, i64::try_from(SETTINGS).unwrap());
}
