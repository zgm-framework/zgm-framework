//! `list`: all the settings of one tenant, each with its value, its default and whether the
//! tenant changed it.

mod common;

use sqlx::PgPool;
use zgm_settings::{Setting, Settings, SettingsError, Value};

use common::{IamSetting, KOWALSKI, NOWAK, days, default_by_hand, set_by_hand, settings_on};

fn unchanged(name: &str, default: Value) -> Setting {
    Setting {
        name: name.to_owned(),
        value: default.clone(),
        default,
        overridden: false,
    }
}

fn changed(name: &str, value: Value, default: Value) -> Setting {
    Setting {
        name: name.to_owned(),
        value,
        default,
        overridden: true,
    }
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
async fn the_row_of_a_name_without_a_default_is_left_out(pool: PgPool) {
    let settings = settings_on(&pool).await;
    // Defaults an earlier version of the module left, which this one no longer gives.
    default_by_hand(&pool, "iam.dropped_setting", "duration", "2592000").await;
    default_by_hand(&pool, "aaa.before_all", "flag", "false").await;
    set_by_hand(&pool, NOWAK, "iam.dropped_setting", "7776000").await;
    set_by_hand(&pool, NOWAK, "aaa.before_all", "true").await;

    let names: Vec<String> = settings
        .tenant_id(NOWAK)
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|setting| setting.name)
        .collect();

    assert_eq!(
        names,
        [
            "iam.max_users",
            "iam.self_sign_up",
            "iam.user_deletion_window",
            "iam.welcome_text"
        ]
    );
}

#[sqlx::test(migrations = false)]
async fn a_row_of_another_kind_than_the_default_is_an_error(pool: PgPool) {
    let settings = settings_on(&pool).await;
    set_by_hand(&pool, NOWAK, IamSetting::SelfSignUp, "1").await;

    let refused = settings.tenant_id(NOWAK).list().await.unwrap_err();

    assert!(
        matches!(
            &refused,
            SettingsError::WrongKind { name, found }
                if name == "iam.self_sign_up" && found == "1"
        ),
        "{refused}"
    );
}

#[sqlx::test(migrations = false)]
async fn without_defaults_the_list_is_empty(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();
    let settings = Settings::builder().pool(pool.clone()).build();
    // A default an earlier start left, which this program has not given.
    default_by_hand(&pool, IamSetting::UserDeletionWindow, "duration", "2592000").await;
    set_by_hand(&pool, NOWAK, IamSetting::UserDeletionWindow, "7776000").await;

    let all = settings.tenant_id(NOWAK).list().await.unwrap();

    assert_eq!(all, []);
}
