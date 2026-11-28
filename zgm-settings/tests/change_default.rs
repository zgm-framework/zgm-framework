//! `change_default`: a person changes the default of a setting and its bounds, e.g. on a
//! screen of the program. The kind stays, the least is not above the most, and the default lies
//! within its bounds; a later start of a module leaves the change as it is.

mod common;

use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use zgm_settings::{DefaultSetting, Settings, SettingsError, Value};

use common::{
    IamSetting, KOWALSKI, NOWAK, days, default_by_hand, default_row, json_of, rows, set_by_hand,
};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

const WINDOW: IamSetting = IamSetting::UserDeletionWindow;
const MAX_USERS: IamSetting = IamSetting::MaxUsers;
const SELF_SIGN_UP: IamSetting = IamSetting::SelfSignUp;
const WELCOME_TEXT: IamSetting = IamSetting::WelcomeText;

fn window(value: Duration, min: Option<Duration>, max: Option<Duration>) -> DefaultSetting {
    DefaultSetting::Duration { value, min, max }
}

fn max_users(value: i64, min: Option<i64>, max: Option<i64>) -> DefaultSetting {
    DefaultSetting::Number { value, min, max }
}

/// A row of the defaults: the kind, the JSON as the database writes it, and the bounds.
fn row(
    kind: &str,
    json: &str,
    min: Option<i64>,
    max: Option<i64>,
) -> Option<(String, String, Option<i64>, Option<i64>)> {
    Some((kind.to_owned(), json.to_owned(), min, max))
}

/// The module on a migrated database, with the defaults of [`IamSetting`] as a module gives
/// them: the window 30 days, at least a day and at most a year; 10 users, at least 1 and at
/// most 1000; no for self sign-up; "Welcome".
async fn settings_on(pool: &PgPool) -> Settings {
    zgm_settings::migrate(pool).await.unwrap();
    let settings = Settings::builder().pool(pool.clone()).build();
    for (setting, default) in [
        (WINDOW, window(DAY * 30, Some(DAY), Some(DAY * 365))),
        (MAX_USERS, max_users(10, Some(1), Some(1000))),
        (SELF_SIGN_UP, DefaultSetting::Flag(false)),
        (WELCOME_TEXT, DefaultSetting::Text("Welcome".to_owned())),
    ] {
        settings.add_default(setting, default).await.unwrap();
    }
    settings
}

/// The rows of every default as the database holds them, in the order of the names.
async fn all_defaults(pool: &PgPool) -> Vec<(String, String, String, Option<i64>, Option<i64>)> {
    sqlx::query_as(
        "SELECT name, kind, value::text, min, max FROM zgm_settings.default_settings \
         ORDER BY name COLLATE \"C\"",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

/// When the row of a default was last changed.
async fn updated_at(pool: &PgPool, name: impl AsRef<str>) -> String {
    sqlx::query_scalar("SELECT updated_at::text FROM zgm_settings.default_settings WHERE name = $1")
        .bind(name.as_ref())
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Settings on a pool that never connects.
fn settings_without_a_database() -> Settings {
    let pool = PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(500))
        .connect_lazy("postgres://postgres:postgres@127.0.0.1:1/nothing")
        .unwrap();
    Settings::builder().pool(pool).build()
}

#[sqlx::test(migrations = false)]
async fn a_new_default_and_new_bounds_are_written_and_it_is_a_change(pool: PgPool) {
    let settings = settings_on(&pool).await;

    let changed = settings
        .change_default(WINDOW, window(DAY * 14, Some(DAY * 7), Some(DAY * 90)))
        .await
        .unwrap();

    assert!(changed);
    assert_eq!(
        default_row(&pool, WINDOW).await,
        row("duration", "1209600", Some(604_800), Some(7_776_000))
    );
    assert_eq!(
        settings.defaults().await.unwrap()[WINDOW.as_ref()],
        days(14)
    );
}

#[sqlx::test(migrations = false)]
async fn a_default_of_every_kind_is_changed(pool: PgPool) {
    let settings = settings_on(&pool).await;

    for (setting, default) in [
        (WINDOW, window(DAY * 60, None, None)),
        (MAX_USERS, max_users(-5, Some(i64::MIN), None)),
        (SELF_SIGN_UP, DefaultSetting::Flag(true)),
        (
            WELCOME_TEXT,
            DefaultSetting::Text("Witaj \"Nowak\"\\\nżółć".to_owned()),
        ),
    ] {
        assert!(settings.change_default(setting, default).await.unwrap());
    }

    assert_eq!(
        all_defaults(&pool).await,
        vec![
            (
                "iam.max_users".to_owned(),
                "number".to_owned(),
                "-5".to_owned(),
                Some(i64::MIN),
                None
            ),
            (
                "iam.self_sign_up".to_owned(),
                "flag".to_owned(),
                "true".to_owned(),
                None,
                None
            ),
            (
                "iam.user_deletion_window".to_owned(),
                "duration".to_owned(),
                "5184000".to_owned(),
                None,
                None
            ),
            (
                "iam.welcome_text".to_owned(),
                "text".to_owned(),
                r#""Witaj \"Nowak\"\\\nżółć""#.to_owned(),
                None,
                None
            ),
        ]
    );
    let defaults = settings.defaults().await.unwrap();
    assert_eq!(
        defaults[WELCOME_TEXT.as_ref()],
        Value::Text("Witaj \"Nowak\"\\\nżółć".to_owned())
    );
}

#[sqlx::test(migrations = false)]
async fn only_the_bounds_changed_is_a_change(pool: PgPool) {
    let settings = settings_on(&pool).await;

    assert!(
        settings
            .change_default(WINDOW, window(DAY * 30, Some(DAY), None))
            .await
            .unwrap()
    );
    assert_eq!(
        default_row(&pool, WINDOW).await,
        row("duration", "2592000", Some(86_400), None)
    );
    assert!(
        settings
            .change_default(WINDOW, window(DAY * 30, None, None))
            .await
            .unwrap()
    );
    assert_eq!(
        default_row(&pool, WINDOW).await,
        row("duration", "2592000", None, None)
    );
}

#[sqlx::test(migrations = false)]
async fn the_same_default_and_bounds_again_change_nothing_not_even_updated_at(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let change = window(DAY * 14, Some(DAY * 7), Some(DAY * 90));
    assert!(
        settings
            .change_default(WINDOW, change.clone())
            .await
            .unwrap()
    );
    let before = updated_at(&pool, WINDOW).await;

    assert!(!settings.change_default(WINDOW, change).await.unwrap());
    // The same as the module gave, for every kind.
    assert!(
        !settings
            .change_default(MAX_USERS, max_users(10, Some(1), Some(1000)))
            .await
            .unwrap()
    );
    assert!(
        !settings
            .change_default(SELF_SIGN_UP, DefaultSetting::Flag(false))
            .await
            .unwrap()
    );
    assert!(
        !settings
            .change_default(WELCOME_TEXT, DefaultSetting::Text("Welcome".to_owned()))
            .await
            .unwrap()
    );

    assert_eq!(updated_at(&pool, WINDOW).await, before);
}

#[sqlx::test(migrations = false)]
async fn a_change_moves_updated_at(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let before = updated_at(&pool, WINDOW).await;

    settings
        .change_default(WINDOW, window(DAY * 14, Some(DAY), Some(DAY * 365)))
        .await
        .unwrap();

    assert!(updated_at(&pool, WINDOW).await > before);
}

#[sqlx::test(migrations = false)]
async fn the_other_defaults_stay_as_they_were(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let before = all_defaults(&pool).await;

    settings
        .change_default(MAX_USERS, max_users(20, Some(1), Some(1000)))
        .await
        .unwrap();

    let after = all_defaults(&pool).await;
    assert_eq!(after[0].2, "20");
    assert_eq!(after[1..], before[1..]);
}

#[sqlx::test(migrations = false)]
async fn a_name_without_a_default_is_refused_and_nothing_is_written(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let before = all_defaults(&pool).await;

    let refused = settings
        .change_default("iam.no_such_setting", max_users(5, None, None))
        .await;

    assert!(
        matches!(&refused, Err(SettingsError::UnknownSetting(name)) if name == "iam.no_such_setting"),
        "{refused:?}"
    );
    assert_eq!(all_defaults(&pool).await, before);
}

#[sqlx::test(migrations = false)]
async fn another_kind_than_the_one_in_the_database_is_refused_and_nothing_changes(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let before = all_defaults(&pool).await;

    for (setting, default, found) in [
        (WINDOW, max_users(30, None, None), "Number(30)"),
        (
            WINDOW,
            DefaultSetting::Text("30 days".to_owned()),
            r#"Text("30 days")"#,
        ),
        (MAX_USERS, window(DAY, None, None), "Duration(86400s)"),
        (MAX_USERS, DefaultSetting::Flag(true), "Flag(true)"),
        (
            SELF_SIGN_UP,
            DefaultSetting::Text("true".to_owned()),
            r#"Text("true")"#,
        ),
        (WELCOME_TEXT, DefaultSetting::Flag(false), "Flag(false)"),
    ] {
        let refused = settings.change_default(setting, default).await;
        assert!(
            matches!(
                &refused,
                Err(SettingsError::WrongKind { name, found: given })
                    if name == setting.as_ref() && given == found
            ),
            "{refused:?}"
        );
    }

    assert_eq!(all_defaults(&pool).await, before);
}

#[sqlx::test(migrations = false)]
async fn a_least_above_the_most_is_refused_and_nothing_changes(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let before = all_defaults(&pool).await;

    let refused = settings
        .change_default(WINDOW, window(DAY * 30, Some(DAY * 60), Some(DAY * 59)))
        .await;
    assert!(
        matches!(
            &refused,
            Err(SettingsError::LeastAboveTheMost { name, min, max })
                if name == WINDOW.as_ref() && *min == days(60) && *max == days(59)
        ),
        "{refused:?}"
    );

    let refused = settings
        .change_default(MAX_USERS, max_users(10, Some(i64::MAX), Some(i64::MIN)))
        .await;
    assert!(
        matches!(
            &refused,
            Err(SettingsError::LeastAboveTheMost { name, min, max })
                if name == MAX_USERS.as_ref()
                    && *min == Value::Number(i64::MAX)
                    && *max == Value::Number(i64::MIN)
        ),
        "{refused:?}"
    );

    assert_eq!(all_defaults(&pool).await, before);
}

#[sqlx::test(migrations = false)]
async fn a_least_equal_to_the_most_is_taken(pool: PgPool) {
    let settings = settings_on(&pool).await;

    assert!(
        settings
            .change_default(WINDOW, window(DAY * 30, Some(DAY * 30), Some(DAY * 30)))
            .await
            .unwrap()
    );
    assert!(
        settings
            .change_default(MAX_USERS, max_users(7, Some(7), Some(7)))
            .await
            .unwrap()
    );

    assert_eq!(
        default_row(&pool, WINDOW).await,
        row("duration", "2592000", Some(2_592_000), Some(2_592_000))
    );
    assert_eq!(
        default_row(&pool, MAX_USERS).await,
        row("number", "7", Some(7), Some(7))
    );
}

#[sqlx::test(migrations = false)]
async fn a_default_below_its_least_is_refused_and_nothing_changes(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let before = all_defaults(&pool).await;

    for (setting, default, least) in [
        (WINDOW, window(DAY * 30, Some(DAY * 31), None), days(31)),
        (
            WINDOW,
            window(DAY - Duration::from_secs(1), Some(DAY), Some(DAY * 365)),
            days(1),
        ),
        (
            WINDOW,
            window(Duration::ZERO, Some(Duration::from_secs(1)), None),
            Value::Duration(Duration::from_secs(1)),
        ),
        (
            MAX_USERS,
            max_users(0, Some(1), Some(1000)),
            Value::Number(1),
        ),
        (
            MAX_USERS,
            max_users(i64::MIN, Some(i64::MIN + 1), None),
            Value::Number(i64::MIN + 1),
        ),
    ] {
        let refused = settings.change_default(setting, default).await;
        assert!(
            matches!(
                &refused,
                Err(SettingsError::BelowTheLeast { name, min }) if name == setting.as_ref() && *min == least
            ),
            "{refused:?}"
        );
    }

    assert_eq!(all_defaults(&pool).await, before);
}

#[sqlx::test(migrations = false)]
async fn a_default_above_its_most_is_refused_and_nothing_changes(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let before = all_defaults(&pool).await;

    for (setting, default, most) in [
        (WINDOW, window(DAY * 30, None, Some(DAY * 29)), days(29)),
        (
            WINDOW,
            window(
                DAY * 365 + Duration::from_secs(1),
                Some(DAY),
                Some(DAY * 365),
            ),
            days(365),
        ),
        (
            WINDOW,
            window(Duration::from_secs(u64::MAX), None, Some(DAY)),
            days(1),
        ),
        (
            MAX_USERS,
            max_users(1001, Some(1), Some(1000)),
            Value::Number(1000),
        ),
        (
            MAX_USERS,
            max_users(i64::MAX, None, Some(i64::MAX - 1)),
            Value::Number(i64::MAX - 1),
        ),
    ] {
        let refused = settings.change_default(setting, default).await;
        assert!(
            matches!(
                &refused,
                Err(SettingsError::AboveTheMost { name, max }) if name == setting.as_ref() && *max == most
            ),
            "{refused:?}"
        );
    }

    assert_eq!(all_defaults(&pool).await, before);
}

#[sqlx::test(migrations = false)]
async fn a_default_on_its_bounds_is_taken(pool: PgPool) {
    let settings = settings_on(&pool).await;

    assert!(
        settings
            .change_default(WINDOW, window(DAY, Some(DAY), Some(DAY * 365)))
            .await
            .unwrap()
    );
    assert!(
        settings
            .change_default(WINDOW, window(DAY * 365, Some(DAY), Some(DAY * 365)))
            .await
            .unwrap()
    );
    assert!(
        settings
            .change_default(MAX_USERS, max_users(1, Some(1), Some(1000)))
            .await
            .unwrap()
    );
    assert!(
        settings
            .change_default(MAX_USERS, max_users(1000, Some(1), Some(1000)))
            .await
            .unwrap()
    );

    assert_eq!(
        default_row(&pool, WINDOW).await,
        row("duration", "31536000", Some(86_400), Some(31_536_000))
    );
    assert_eq!(
        default_row(&pool, MAX_USERS).await,
        row("number", "1000", Some(1), Some(1000))
    );
}

#[sqlx::test(migrations = false)]
async fn a_bound_that_is_none_holds_nothing(pool: PgPool) {
    let settings = settings_on(&pool).await;

    assert!(
        settings
            .change_default(MAX_USERS, max_users(i64::MIN, None, Some(0)))
            .await
            .unwrap()
    );
    assert!(
        settings
            .change_default(MAX_USERS, max_users(i64::MAX, Some(0), None))
            .await
            .unwrap()
    );
    assert!(
        settings
            .change_default(WINDOW, window(Duration::ZERO, None, Some(DAY)))
            .await
            .unwrap()
    );

    assert_eq!(
        default_row(&pool, MAX_USERS).await,
        row("number", &i64::MAX.to_string(), Some(0), None)
    );
    assert_eq!(
        default_row(&pool, WINDOW).await,
        row("duration", "0", None, Some(86_400))
    );
}

#[sqlx::test(migrations = false)]
async fn seconds_above_a_bigint_are_held_as_the_most_a_bigint_holds(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let longest = Duration::from_secs(u64::MAX);

    // The most and the default both beyond a bigint: held as equal, as the table keeps them.
    assert!(
        settings
            .change_default(WINDOW, window(longest, Some(DAY), Some(longest)))
            .await
            .unwrap()
    );

    assert_eq!(
        default_row(&pool, WINDOW).await,
        row(
            "duration",
            &u64::MAX.to_string(),
            Some(86_400),
            Some(i64::MAX)
        )
    );
    assert_eq!(
        settings.defaults().await.unwrap()[WINDOW.as_ref()],
        Value::Duration(longest)
    );
}

#[sqlx::test(migrations = false)]
async fn a_fraction_of_a_second_in_the_default_or_a_bound_is_refused(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let before = all_defaults(&pool).await;
    let a_second_and_a_half = Duration::from_millis(1500);

    for default in [
        window(a_second_and_a_half, None, None),
        window(DAY + Duration::from_nanos(1), Some(DAY), Some(DAY * 365)),
        window(DAY * 30, Some(a_second_and_a_half), None),
        window(DAY * 30, None, Some(DAY * 365 + Duration::from_nanos(1))),
    ] {
        let refused = settings.change_default(WINDOW, default.clone()).await;
        assert!(
            matches!(&refused, Err(SettingsError::FractionOfASecond(name)) if name == WINDOW.as_ref()),
            "{default:?}: {refused:?}"
        );
    }

    assert_eq!(all_defaults(&pool).await, before);
}

#[sqlx::test(migrations = false)]
async fn a_tenant_without_a_value_of_its_own_gets_the_new_default_at_once(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let in_nowak = settings.tenant_id(NOWAK);
    assert_eq!(in_nowak.get(WINDOW).await.unwrap(), days(30));

    settings
        .change_default(WINDOW, window(DAY * 14, Some(DAY), Some(DAY * 365)))
        .await
        .unwrap();

    assert_eq!(in_nowak.get(WINDOW).await.unwrap(), days(14));
}

#[sqlx::test(migrations = false)]
async fn a_tenant_keeps_its_value_outside_new_bounds_and_its_next_set_is_held_against_them(
    pool: PgPool,
) {
    let settings = settings_on(&pool).await;
    let in_nowak = settings.tenant_id(NOWAK);
    in_nowak.set(WINDOW, days(90)).await.unwrap();

    settings
        .change_default(WINDOW, window(DAY * 14, Some(DAY), Some(DAY * 60)))
        .await
        .unwrap();

    // The bounds do not reach back: Nowak keeps 90 days, and Kowalski has the new default.
    assert_eq!(in_nowak.get(WINDOW).await.unwrap(), days(90));
    assert_eq!(
        settings.tenant_id(KOWALSKI).get(WINDOW).await.unwrap(),
        days(14)
    );
    // The next set is held against the new most.
    let refused = in_nowak.set(WINDOW, days(61)).await;
    assert!(
        matches!(&refused, Err(SettingsError::AboveTheMost { max, .. }) if *max == days(60)),
        "{refused:?}"
    );
    assert!(in_nowak.set(WINDOW, days(60)).await.unwrap());
    // The list shows the new default and bounds.
    let listed = in_nowak.list().await.unwrap();
    let window_listed = listed
        .iter()
        .find(|setting| setting.name == WINDOW.as_ref())
        .unwrap();
    assert_eq!(
        (
            &window_listed.default,
            &window_listed.min,
            &window_listed.max
        ),
        (&days(14), &Some(days(1)), &Some(days(60)))
    );
}

#[sqlx::test(migrations = false)]
async fn a_later_start_of_the_module_leaves_the_change(pool: PgPool) {
    let settings = settings_on(&pool).await;
    settings
        .change_default(WINDOW, window(DAY * 14, Some(DAY * 7), Some(DAY * 90)))
        .await
        .unwrap();

    // The module starts again and gives its default, as in its code.
    let again = Settings::builder().pool(pool.clone()).build();
    let written = again
        .add_default(WINDOW, window(DAY * 30, Some(DAY), Some(DAY * 365)))
        .await
        .unwrap();

    assert!(!written);
    assert_eq!(
        default_row(&pool, WINDOW).await,
        row("duration", "1209600", Some(604_800), Some(7_776_000))
    );
}

#[sqlx::test(migrations = false)]
async fn a_default_this_program_did_not_give_is_changed_too(pool: PgPool) {
    let settings = settings_on(&pool).await;
    // A default an earlier version of a module left, which no module gives now.
    default_by_hand(&pool, "iam.dropped_setting", "flag", "true").await;

    assert!(
        settings
            .change_default("iam.dropped_setting", DefaultSetting::Flag(false))
            .await
            .unwrap()
    );

    assert_eq!(
        default_row(&pool, "iam.dropped_setting").await,
        row("flag", "false", None, None)
    );
}

#[sqlx::test(migrations = false)]
async fn a_default_the_module_cannot_read_is_mended_when_its_kind_is_given(pool: PgPool) {
    let settings = settings_on(&pool).await;
    // A row of a duration that holds a text, which only SQL by hand can write.
    default_by_hand(&pool, "iam.broken_window", "duration", r#""thirty days""#).await;
    let in_nowak = settings.tenant_id(NOWAK);
    assert!(matches!(
        in_nowak.get("iam.broken_window").await,
        Err(SettingsError::UnreadableDefault { .. })
    ));

    assert!(
        settings
            .change_default("iam.broken_window", window(DAY * 30, None, None))
            .await
            .unwrap()
    );

    assert_eq!(in_nowak.get("iam.broken_window").await.unwrap(), days(30));
}

#[sqlx::test(migrations = false)]
async fn a_default_of_a_kind_the_module_does_not_know_is_refused(pool: PgPool) {
    let settings = settings_on(&pool).await;
    default_by_hand(&pool, "iam.colour", "colour", r#""red""#).await;

    let refused = settings
        .change_default("iam.colour", DefaultSetting::Text("blue".to_owned()))
        .await;

    assert!(
        matches!(&refused, Err(SettingsError::WrongKind { name, .. }) if name == "iam.colour"),
        "{refused:?}"
    );
    assert_eq!(
        default_row(&pool, "iam.colour").await,
        row("colour", r#""red""#, None, None)
    );
}

#[sqlx::test(migrations = false)]
async fn the_values_of_the_tenants_stay_as_they_were(pool: PgPool) {
    let settings = settings_on(&pool).await;
    set_by_hand(&pool, NOWAK, WINDOW, "7776000").await;
    set_by_hand(&pool, KOWALSKI, MAX_USERS, "5").await;

    settings
        .change_default(WINDOW, window(DAY * 14, Some(DAY), Some(DAY * 60)))
        .await
        .unwrap();

    assert_eq!(
        json_of(&pool, NOWAK, WINDOW).await.as_deref(),
        Some("7776000")
    );
    assert_eq!(
        json_of(&pool, KOWALSKI, MAX_USERS).await.as_deref(),
        Some("5")
    );
    assert_eq!(rows(&pool).await, 2);
}

#[tokio::test]
async fn when_the_database_does_not_answer_it_is_an_error() {
    let refused = settings_without_a_database()
        .change_default(WINDOW, window(DAY * 14, None, None))
        .await;

    assert!(
        matches!(refused, Err(SettingsError::Database(_))),
        "{refused:?}"
    );
}
