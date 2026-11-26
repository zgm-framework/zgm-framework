//! What the table `zgm_settings.default_settings` itself guarantees, checked with SQL by hand:
//! no function of the module writes it yet.

use sqlx::error::ErrorKind;
use sqlx::{AssertSqlSafe, PgPool};

/// The name of a setting, with the prefix of its module.
const WINDOW: &str = "iam.user_deletion_window";

/// 30 days in seconds, as the text of its JSON.
const THIRTY_DAYS: &str = "2592000";

/// A day and a year, in seconds.
const A_DAY: i64 = 24 * 60 * 60;
const A_YEAR: i64 = 365 * A_DAY;

/// Writes the default of a setting with SQL by hand; `value` is the text of the JSON.
async fn insert(
    pool: &PgPool,
    name: &str,
    kind: &str,
    value: &str,
    min: Option<i64>,
    max: Option<i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO zgm_settings.default_settings (name, kind, value, min, max) \
         VALUES ($1, $2, $3::jsonb, $4, $5)",
    )
    .bind(name)
    .bind(kind)
    .bind(value)
    .bind(min)
    .bind(max)
    .execute(pool)
    .await
    .map(|_| ())
}

/// The bounds of a setting, as the table keeps them.
async fn bounds_of(pool: &PgPool, name: &str) -> (Option<i64>, Option<i64>) {
    sqlx::query_as("SELECT min, max FROM zgm_settings.default_settings WHERE name = $1")
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// How many rows the table has.
async fn count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM zgm_settings.default_settings")
        .fetch_one(pool)
        .await
        .unwrap()
}

fn error_kind_of(error: &sqlx::Error) -> Option<ErrorKind> {
    error.as_database_error().map(|error| error.kind())
}

/// The SQLSTATE of an error of the database.
fn code_of(error: &sqlx::Error) -> Option<String> {
    error
        .as_database_error()
        .and_then(|error| error.code().map(|code| code.into_owned()))
}

#[sqlx::test(migrations = false)]
async fn a_new_row_gets_the_moment_of_its_statement_as_both_dates(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();

    // In a transaction that began earlier: the dates are the moment of the statement, not of
    // the start of the transaction.
    let mut transaction = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_sleep(0.01)")
        .execute(&mut *transaction)
        .await
        .unwrap();
    let dated: bool = sqlx::query_scalar(
        "INSERT INTO zgm_settings.default_settings (name, kind, value) \
         VALUES ($1, 'duration', '2592000') \
         RETURNING created_at = statement_timestamp() AND updated_at = created_at \
                   AND created_at > transaction_timestamp()",
    )
    .bind(WINDOW)
    .fetch_one(&mut *transaction)
    .await
    .unwrap();
    assert!(dated);
}

#[sqlx::test(migrations = false)]
async fn a_change_sets_updated_at_to_the_moment_of_the_change(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();

    let mut transaction = pool.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO zgm_settings.default_settings (name, kind, value) \
         VALUES ($1, 'duration', '2592000')",
    )
    .bind(WINDOW)
    .execute(&mut *transaction)
    .await
    .unwrap();
    sqlx::query("SELECT pg_sleep(0.01)")
        .execute(&mut *transaction)
        .await
        .unwrap();
    // Whatever the statement says, and also later in the same transaction.
    let touched: bool = sqlx::query_scalar(
        "UPDATE zgm_settings.default_settings SET max = 31536000, updated_at = '2000-01-01' \
         WHERE name = $1 \
         RETURNING updated_at = statement_timestamp() AND updated_at > created_at",
    )
    .bind(WINDOW)
    .fetch_one(&mut *transaction)
    .await
    .unwrap();
    assert!(touched);
}

#[sqlx::test(migrations = false)]
async fn the_same_setting_twice_is_refused(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();
    insert(
        &pool,
        WINDOW,
        "duration",
        THIRTY_DAYS,
        Some(A_DAY),
        Some(A_YEAR),
    )
    .await
    .unwrap();

    // Every other column different: the name alone is the key.
    let refused = insert(&pool, WINDOW, "number", "10", None, None)
        .await
        .unwrap_err();

    assert!(
        matches!(error_kind_of(&refused), Some(ErrorKind::UniqueViolation)),
        "{refused}"
    );
    let kept: bool = sqlx::query_scalar(
        "SELECT kind = 'duration' AND value = '2592000' \
         FROM zgm_settings.default_settings WHERE name = $1",
    )
    .bind(WINDOW)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(kept);
    assert_eq!(bounds_of(&pool, WINDOW).await, (Some(A_DAY), Some(A_YEAR)));
}

#[sqlx::test(migrations = false)]
async fn two_settings_are_two_rows(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();

    insert(
        &pool,
        WINDOW,
        "duration",
        THIRTY_DAYS,
        Some(A_DAY),
        Some(A_YEAR),
    )
    .await
    .unwrap();
    insert(&pool, "iam.max_users", "number", "10", Some(1), None)
        .await
        .unwrap();

    assert_eq!(count(&pool).await, 2);
}

#[sqlx::test(migrations = false)]
async fn a_missing_bound_is_kept_empty(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();

    insert(&pool, "iam.max_users", "number", "10", Some(1), None)
        .await
        .unwrap();
    insert(&pool, "iam.grace", "duration", "0", None, Some(A_DAY))
        .await
        .unwrap();
    insert(&pool, "iam.self_sign_up", "flag", "false", None, None)
        .await
        .unwrap();

    assert_eq!(bounds_of(&pool, "iam.max_users").await, (Some(1), None));
    assert_eq!(bounds_of(&pool, "iam.grace").await, (None, Some(A_DAY)));
    assert_eq!(bounds_of(&pool, "iam.self_sign_up").await, (None, None));
}

#[sqlx::test(migrations = false)]
async fn the_bounds_keep_every_whole_number_of_64_bits(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();

    insert(
        &pool,
        "iam.offset",
        "number",
        "0",
        Some(i64::MIN),
        Some(i64::MAX),
    )
    .await
    .unwrap();

    assert_eq!(
        bounds_of(&pool, "iam.offset").await,
        (Some(i64::MIN), Some(i64::MAX))
    );
}

#[sqlx::test(migrations = false)]
async fn a_bound_written_as_a_fraction_or_a_word_is_refused(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();

    for (min, max) in [
        ("'1.5'", "NULL"),
        ("'a day'", "NULL"),
        ("'true'", "NULL"),
        ("NULL", "'1.5'"),
        ("NULL", "'a year'"),
        ("NULL", "'true'"),
    ] {
        // Literals without a cast: the column itself decides what it takes. Only the
        // constants of this file go into the text.
        let refused = sqlx::raw_sql(AssertSqlSafe(format!(
            "INSERT INTO zgm_settings.default_settings (name, kind, value, min, max) \
             VALUES ('{WINDOW}', 'duration', '{THIRTY_DAYS}', {min}, {max})"
        )))
        .execute(&pool)
        .await
        .unwrap_err();
        // invalid_text_representation: the text is no whole number.
        assert_eq!(
            code_of(&refused).as_deref(),
            Some("22P02"),
            "{min}, {max}: {refused}"
        );
    }
    assert_eq!(count(&pool).await, 0);
}

#[sqlx::test(migrations = false)]
async fn a_row_without_a_name_a_kind_or_a_value_is_refused(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();

    for values in [
        format!("(NULL, 'duration', '{THIRTY_DAYS}')"),
        format!("('{WINDOW}', NULL, '{THIRTY_DAYS}')"),
        format!("('{WINDOW}', 'duration', NULL)"),
    ] {
        // Only the constants of this file go into the text.
        let refused = sqlx::raw_sql(AssertSqlSafe(format!(
            "INSERT INTO zgm_settings.default_settings (name, kind, value) VALUES {values}"
        )))
        .execute(&pool)
        .await
        .unwrap_err();
        assert!(
            matches!(error_kind_of(&refused), Some(ErrorKind::NotNullViolation)),
            "{values}: {refused}"
        );
    }
    assert_eq!(count(&pool).await, 0);
}

#[sqlx::test(migrations = false)]
async fn a_value_that_is_not_json_is_refused(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();

    // A literal without a cast: the column itself decides what it takes. Only the constants
    // of this file go into the text.
    let refused = sqlx::raw_sql(AssertSqlSafe(format!(
        "INSERT INTO zgm_settings.default_settings (name, kind, value) \
         VALUES ('{WINDOW}', 'duration', '30 days')"
    )))
    .execute(&pool)
    .await
    .unwrap_err();

    // invalid_text_representation: the text is no JSON.
    assert_eq!(code_of(&refused).as_deref(), Some("22P02"), "{refused}");
    assert_eq!(count(&pool).await, 0);
}

#[sqlx::test(migrations = false)]
async fn a_value_of_every_kind_is_kept_as_it_was_written(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();

    for (name, kind, value) in [
        (WINDOW, "duration", THIRTY_DAYS),
        ("iam.max_users", "number", "10"),
        ("iam.self_sign_up", "flag", "false"),
        ("iam.welcome_text", "text", "\"Welcome\""),
    ] {
        insert(&pool, name, kind, value, None, None).await.unwrap();
        let kept: bool = sqlx::query_scalar(
            "SELECT kind = $2 AND value = $3::jsonb FROM zgm_settings.default_settings \
             WHERE name = $1",
        )
        .bind(name)
        .bind(kind)
        .bind(value)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(kept, "{name}");
    }
}

#[sqlx::test(migrations = false)]
async fn the_database_leaves_the_kind_and_the_bounds_to_the_module(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();

    // Only the module's code writes the table, so it is the code that refuses these: a kind
    // it does not know, a least above a most, and bounds on a flag.
    insert(&pool, "iam.colour", "colour", "\"blue\"", None, None)
        .await
        .unwrap();
    insert(
        &pool,
        WINDOW,
        "duration",
        THIRTY_DAYS,
        Some(A_YEAR),
        Some(A_DAY),
    )
    .await
    .unwrap();
    insert(&pool, "iam.self_sign_up", "flag", "false", Some(0), Some(1))
        .await
        .unwrap();

    assert_eq!(count(&pool).await, 3);
}
