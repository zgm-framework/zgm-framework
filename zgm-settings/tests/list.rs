//! `list`: all the settings of one tenant, every default in the database, each with its value,
//! its default, whether the tenant changed it, and its bounds.

mod common;

use std::time::Duration;

use sqlx::PgPool;
use zgm_settings::{DefaultSetting, Setting, Settings, SettingsError, Value};

use common::{IamSetting, KOWALSKI, NOWAK, days, default_by_hand, set_by_hand, settings_on};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// A setting the tenant did not change, without bounds.
fn unchanged(name: &str, default: Value) -> Setting {
    Setting {
        name: name.to_owned(),
        value: default.clone(),
        default,
        overridden: false,
        min: None,
        max: None,
    }
}

/// A setting the tenant changed, without bounds.
fn changed(name: &str, value: Value, default: Value) -> Setting {
    Setting {
        name: name.to_owned(),
        value,
        default,
        overridden: true,
        min: None,
        max: None,
    }
}

/// The same setting with bounds.
fn within(setting: Setting, min: Option<Value>, max: Option<Value>) -> Setting {
    Setting {
        min,
        max,
        ..setting
    }
}

/// Changes a default and its bounds with SQL by hand, as the function that changes a default
/// will.
async fn default_changed_by_hand(
    pool: &PgPool,
    name: impl AsRef<str>,
    json: &str,
    min: Option<i64>,
    max: Option<i64>,
) {
    sqlx::query(
        "UPDATE zgm_settings.default_settings SET value = $2::jsonb, min = $3, max = $4 \
         WHERE name = $1",
    )
    .bind(name.as_ref())
    .bind(json)
    .bind(min)
    .bind(max)
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test(migrations = false)]
async fn a_tenant_without_rows_has_all_the_defaults_in_the_order_of_their_names(pool: PgPool) {
    let settings = settings_on(&pool).await;

    let all = settings.tenant_id(NOWAK).list().await.unwrap();

    assert_eq!(
        all,
        [
            unchanged("iam.max_users", Value::Number(10)),
            unchanged("iam.self_sign_up", Value::Flag(false)),
            unchanged("iam.user_deletion_window", days(30)),
            unchanged("iam.welcome_text", Value::Text("Welcome".to_owned())),
        ]
    );
}

#[sqlx::test(migrations = false)]
async fn a_changed_setting_has_the_value_of_the_tenant_and_the_others_their_defaults(pool: PgPool) {
    let settings = settings_on(&pool).await;
    set_by_hand(&pool, NOWAK, IamSetting::UserDeletionWindow, "7776000").await;
    set_by_hand(&pool, NOWAK, IamSetting::WelcomeText, r#""Witamy""#).await;

    let all = settings.tenant_id(NOWAK).list().await.unwrap();

    assert_eq!(
        all,
        [
            unchanged("iam.max_users", Value::Number(10)),
            unchanged("iam.self_sign_up", Value::Flag(false)),
            changed("iam.user_deletion_window", days(90), days(30)),
            changed(
                "iam.welcome_text",
                Value::Text("Witamy".to_owned()),
                Value::Text("Welcome".to_owned())
            ),
        ]
    );
}

#[sqlx::test(migrations = false)]
async fn a_row_equal_to_the_default_is_a_change_of_the_tenant(pool: PgPool) {
    let settings = settings_on(&pool).await;
    set_by_hand(&pool, NOWAK, IamSetting::MaxUsers, "10").await;

    let all = settings.tenant_id(NOWAK).list().await.unwrap();

    assert_eq!(
        all[0],
        changed("iam.max_users", Value::Number(10), Value::Number(10))
    );
}

#[sqlx::test(migrations = false)]
async fn the_rows_of_another_tenant_do_not_count(pool: PgPool) {
    let settings = settings_on(&pool).await;
    set_by_hand(&pool, KOWALSKI, IamSetting::UserDeletionWindow, "1209600").await;

    let all = settings.tenant_id(NOWAK).list().await.unwrap();

    assert!(all.iter().all(|setting| !setting.overridden), "{all:?}");
    assert_eq!(all[2], unchanged("iam.user_deletion_window", days(30)));
}

#[sqlx::test(migrations = false)]
async fn a_setting_this_program_did_not_give_is_listed_too(pool: PgPool) {
    let settings = settings_on(&pool).await;
    // Defaults an earlier version of the module left, which this one no longer gives.
    default_by_hand(&pool, "iam.dropped_setting", "duration", "2592000").await;
    default_by_hand(&pool, "aaa.before_all", "flag", "false").await;
    set_by_hand(&pool, NOWAK, "iam.dropped_setting", "7776000").await;

    let all = settings.tenant_id(NOWAK).list().await.unwrap();

    assert_eq!(
        all,
        [
            unchanged("aaa.before_all", Value::Flag(false)),
            changed("iam.dropped_setting", days(90), days(30)),
            unchanged("iam.max_users", Value::Number(10)),
            unchanged("iam.self_sign_up", Value::Flag(false)),
            unchanged("iam.user_deletion_window", days(30)),
            unchanged("iam.welcome_text", Value::Text("Welcome".to_owned())),
        ]
    );
}

#[sqlx::test(migrations = false)]
async fn the_order_is_the_order_of_the_bytes_of_the_names(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();
    // The database sorts these another way in its own collation, e.g. `en_US.utf8`.
    for name in ["iam.a", "iam._x", "iam.Z", "iam.B"] {
        default_by_hand(&pool, name, "flag", "false").await;
    }
    let settings = Settings::builder().pool(pool.clone()).build();

    let names: Vec<String> = settings
        .tenant_id(NOWAK)
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|setting| setting.name)
        .collect();

    assert_eq!(names, ["iam.B", "iam.Z", "iam._x", "iam.a"]);
    let in_rust: Vec<String> = settings.defaults().await.unwrap().into_keys().collect();
    assert_eq!(names, in_rust);
}

#[sqlx::test(migrations = false)]
async fn each_setting_has_the_bounds_of_its_default(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();
    let settings = Settings::builder().pool(pool.clone()).build();
    settings
        .add_default(
            IamSetting::UserDeletionWindow,
            DefaultSetting::Duration {
                value: DAY * 30,
                min: Some(DAY),
                max: Some(DAY * 365),
            },
        )
        .await
        .unwrap();
    settings
        .add_default(
            IamSetting::MaxUsers,
            DefaultSetting::Number {
                value: 10,
                min: Some(1),
                max: None,
            },
        )
        .await
        .unwrap();
    settings
        .add_default(IamSetting::SelfSignUp, DefaultSetting::Flag(false))
        .await
        .unwrap();
    // Bounds only SQL by hand can give a flag, which holds none.
    default_changed_by_hand(&pool, IamSetting::SelfSignUp, "false", Some(0), Some(1)).await;
    set_by_hand(&pool, NOWAK, IamSetting::UserDeletionWindow, "7776000").await;

    let all = settings.tenant_id(NOWAK).list().await.unwrap();

    assert_eq!(
        all,
        [
            within(
                unchanged("iam.max_users", Value::Number(10)),
                Some(Value::Number(1)),
                None
            ),
            unchanged("iam.self_sign_up", Value::Flag(false)),
            within(
                changed("iam.user_deletion_window", days(90), days(30)),
                Some(days(1)),
                Some(days(365))
            ),
        ]
    );
}

#[sqlx::test(migrations = false)]
async fn a_change_of_a_default_or_its_bounds_is_seen_by_the_next_list(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let in_kowalski = settings.tenant_id(KOWALSKI);
    assert_eq!(
        in_kowalski.list().await.unwrap()[2],
        unchanged("iam.user_deletion_window", days(30))
    );

    // 45 days, from a day to 60 days.
    default_changed_by_hand(
        &pool,
        IamSetting::UserDeletionWindow,
        "3888000",
        Some(86_400),
        Some(5_184_000),
    )
    .await;

    assert_eq!(
        in_kowalski.list().await.unwrap()[2],
        within(
            unchanged("iam.user_deletion_window", days(45)),
            Some(days(1)),
            Some(days(60))
        )
    );
}

#[sqlx::test(migrations = false)]
async fn a_value_above_a_most_lowered_since_is_listed_as_it_is(pool: PgPool) {
    let settings = settings_on(&pool).await;
    set_by_hand(&pool, NOWAK, IamSetting::UserDeletionWindow, "7776000").await;
    default_changed_by_hand(
        &pool,
        IamSetting::UserDeletionWindow,
        "2592000",
        None,
        Some(5_184_000),
    )
    .await;

    assert_eq!(
        settings.tenant_id(NOWAK).list().await.unwrap()[2],
        within(
            changed("iam.user_deletion_window", days(90), days(30)),
            None,
            Some(days(60))
        )
    );
}

#[sqlx::test(migrations = false)]
async fn a_default_the_module_cannot_read_is_an_error(pool: PgPool) {
    let settings = settings_on(&pool).await;
    // A row only SQL by hand can write.
    default_by_hand(&pool, "iam.colour", "colour", r#""blue""#).await;

    let refused = settings.tenant_id(NOWAK).list().await.unwrap_err();

    assert!(
        matches!(
            &refused,
            SettingsError::UnreadableDefault { name, kind, found }
                if name == "iam.colour" && kind == "colour" && found == r#""blue""#
        ),
        "{refused}"
    );
}

#[sqlx::test(migrations = false)]
async fn a_row_of_another_kind_than_the_default_is_an_error(pool: PgPool) {
    let settings = settings_on(&pool).await;
    // A string, whose JSON has quotes its text has not.
    set_by_hand(&pool, NOWAK, IamSetting::SelfSignUp, r#""yes""#).await;

    let refused = settings.tenant_id(NOWAK).list().await.unwrap_err();

    assert!(
        matches!(
            &refused,
            SettingsError::WrongKind { name, found }
                if name == "iam.self_sign_up" && found == r#""yes""#
        ),
        "{refused}"
    );
}

#[sqlx::test(migrations = false)]
async fn without_defaults_in_the_database_the_list_is_empty(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();
    let settings = Settings::builder().pool(pool.clone()).build();

    assert_eq!(settings.tenant_id(NOWAK).list().await.unwrap(), []);
}

#[sqlx::test(migrations = false)]
async fn a_copy_of_the_program_that_gave_no_default_lists_those_of_the_database(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();
    // A default an earlier start left, which this program has not given.
    default_by_hand(&pool, IamSetting::UserDeletionWindow, "duration", "2592000").await;
    set_by_hand(&pool, NOWAK, IamSetting::UserDeletionWindow, "7776000").await;
    let settings = Settings::builder().pool(pool.clone()).build();

    assert_eq!(
        settings.tenant_id(NOWAK).list().await.unwrap(),
        [changed("iam.user_deletion_window", days(90), days(30))]
    );
}
