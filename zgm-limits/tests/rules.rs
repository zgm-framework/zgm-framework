//! The values a rule can hold, and the reading of the rules when the module is built.

mod common;

use std::time::Duration;

use common::{TestHash, TestLimit, all_limits};
use sqlx::PgPool;
use strum::VariantArray;
use uuid::Uuid;
use zgm_limits::{Count, Limit, Limits, LimitsError, MAX_DISTINCT_ELEMENTS, Period, Rule};

#[test]
fn a_count_holds_any_number_from_one_up() {
    assert_eq!(Count::new(1).get(), 1);
    assert_eq!(Count::new(u32::MAX).get(), u32::MAX);
}

#[test]
#[should_panic(expected = "a count must be at least 1")]
fn a_count_of_zero_panics() {
    let _ = Count::new(0);
}

#[test]
fn a_period_is_a_whole_number_of_seconds() {
    assert_eq!(Period::seconds(1).as_seconds(), 1);
    assert_eq!(Period::minutes(15).as_seconds(), 900);
    assert_eq!(Period::hours(1).as_seconds(), 3600);
    assert_eq!(Period::days(1).as_seconds(), 86_400);
    assert_eq!(Period::minutes(15).as_duration(), Duration::from_mins(15));
    // The longest one: `u32::MAX` seconds, about 136 years.
    assert_eq!(Period::seconds(u32::MAX).as_seconds(), u32::MAX);
    assert_eq!(Period::days(49_710).as_seconds(), 49_710 * 86_400);
}

#[test]
#[should_panic(expected = "a period must be at least 1 second")]
fn a_period_of_no_time_panics() {
    let _ = Period::seconds(0);
}

#[test]
#[should_panic(expected = "a period must be at least 1 second")]
fn a_period_of_no_hours_panics() {
    let _ = Period::hours(0);
}

#[test]
#[should_panic(expected = "a period must fit in u32::MAX seconds")]
fn a_period_longer_than_fits_panics() {
    let _ = Period::days(49_711);
}

/// A limit whose rule has a wrong value, written straight into `rule()`.
#[derive(Debug, Clone, Copy, strum::IntoStaticStr, strum::VariantArray, strum::EnumProperty)]
#[strum(serialize_all = "snake_case")]
enum WrongLimit {
    Good,
    LetsNothingThrough,
}

impl Limit for WrongLimit {
    fn rule(self) -> Rule {
        Rule::Window {
            at_most: Count::new(match self {
                Self::Good => 3,
                Self::LetsNothingThrough => 0,
            }),
            per: Period::hours(1),
        }
    }
}

#[tokio::test]
#[should_panic(expected = "a count must be at least 1")]
async fn a_wrong_rule_stops_the_module_from_being_built() {
    // A pool that has not connected to anything: the panic comes before any query. Nobody has
    // made an attempt either; building alone reads the rule of every limit.
    let pool = PgPool::connect_lazy("postgres://nobody@localhost/nothing").unwrap();

    let _limits = Limits::builder()
        .pool(pool)
        .keyed_hash(TestHash::new())
        .audit(common::NoAudit)
        .limits(WrongLimit::VARIANTS)
        .run_jobs_in_background(|_error| {})
        .build();
}

#[sqlx::test]
async fn the_longest_and_the_shortest_rules_count_without_trouble(pool: PgPool) {
    #[derive(
        Debug, Clone, Copy, strum::IntoStaticStr, strum::VariantArray, strum::EnumProperty,
    )]
    #[strum(serialize_all = "snake_case")]
    enum Edge {
        OneInASecond,
        ManyInTheLongestPeriod,
    }
    impl Limit for Edge {
        fn rule(self) -> Rule {
            match self {
                Self::OneInASecond => Rule::Window {
                    at_most: Count::new(1),
                    per: Period::seconds(1),
                },
                Self::ManyInTheLongestPeriod => Rule::Window {
                    at_most: Count::new(u32::MAX),
                    per: Period::seconds(u32::MAX),
                },
            }
        }
    }
    let limits = Limits::builder()
        .pool(pool.clone())
        .keyed_hash(TestHash::new())
        .audit(common::NoAudit)
        .limits(Edge::VARIANTS)
        .run_jobs_in_background(|_error| {})
        .build();

    // Both count without a panic and without an error of the database.
    for limit in [Edge::OneInASecond, Edge::ManyInTheLongestPeriod] {
        let _counted = limits
            .attempts(limit)
            .tenant_id(Uuid::new_v4())
            .call()
            .await
            .unwrap()
            .hit()
            .await
            .unwrap();
    }
}

#[sqlx::test]
async fn an_attempt_in_a_limit_the_builder_did_not_get_is_an_error(pool: PgPool) {
    // The module knows only `three_per_hour`.
    let limits = Limits::builder()
        .pool(pool.clone())
        .keyed_hash(TestHash::new())
        .audit(common::NoAudit)
        .limits(&[TestLimit::ThreePerHour])
        .run_jobs_in_background(|_error| {})
        .build();
    assert!(all_limits().len() > 1);

    let error = limits
        .attempts(TestLimit::OnePerHour)
        .tenant_id(Uuid::new_v4())
        .call()
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        LimitsError::UnknownLimit {
            limit: "one_per_hour"
        }
    ));
}

/// The module built with one limit; it panics when the bounds of the limit are wrong.
fn built_with<L: Limit>(limit: L) {
    let pool = PgPool::connect_lazy("postgres://nobody@localhost/nothing").unwrap();
    let _limits = Limits::builder()
        .pool(pool)
        .keyed_hash(TestHash::new())
        .audit(common::NoAudit)
        .limits(&[limit])
        .run_jobs_in_background(|_error| {})
        .build();
}

/// Limits whose bounds are wrong, one mistake each. The rule of each is three an hour.
#[derive(Debug, Clone, Copy, strum::IntoStaticStr, strum::VariantArray, strum::EnumProperty)]
#[strum(serialize_all = "snake_case")]
enum WrongBounds {
    /// `max_atmost` for `max_at_most`.
    #[strum(props(
        min_at_most = 1,
        max_atmost = 10,
        min_per_seconds = 60,
        max_per_seconds = 86400
    ))]
    SlipInAName,
    #[strum(props(min_at_most = 1, max_at_most = 10))]
    OnlyTwo,
    #[strum(props(
        min_at_most = 0,
        max_at_most = 10,
        min_per_seconds = 60,
        max_per_seconds = 86400
    ))]
    Zero,
    #[strum(props(
        min_at_most = 5,
        max_at_most = 4,
        min_per_seconds = 60,
        max_per_seconds = 86400
    ))]
    LowestAboveHighest,
    #[strum(props(
        min_at_most = 5,
        max_at_most = 10,
        min_per_seconds = 60,
        max_per_seconds = 86400
    ))]
    DefaultOutside,
}

impl Limit for WrongBounds {
    fn rule(self) -> Rule {
        Rule::Window {
            at_most: Count::new(3),
            per: Period::hours(1),
        }
    }
}

#[tokio::test]
#[should_panic(expected = "the limit `slip_in_a_name` has only some of the bounds")]
async fn a_slip_in_the_name_of_a_bound_stops_the_module_from_being_built() {
    built_with(WrongBounds::SlipInAName);
}

#[tokio::test]
#[should_panic(expected = "the limit `only_two` has only some of the bounds")]
async fn only_some_of_the_bounds_stop_the_module_from_being_built() {
    built_with(WrongBounds::OnlyTwo);
}

#[tokio::test]
#[should_panic(expected = "a bound of the limit `zero` must be from 1 to")]
async fn a_bound_of_zero_stops_the_module_from_being_built() {
    built_with(WrongBounds::Zero);
}

#[tokio::test]
#[should_panic(
    expected = "a lowest bound of the limit `lowest_above_highest` is above its highest"
)]
async fn a_lowest_bound_above_the_highest_stops_the_module_from_being_built() {
    built_with(WrongBounds::LowestAboveHighest);
}

#[tokio::test]
#[should_panic(
    expected = "the default rule of the limit `default_outside` is outside its own bounds"
)]
async fn a_default_outside_its_own_bounds_stops_the_module_from_being_built() {
    built_with(WrongBounds::DefaultOutside);
}

/// Limits that count distinct elements, around the most a window of them may hold.
#[derive(Debug, Clone, Copy, strum::IntoStaticStr, strum::VariantArray, strum::EnumProperty)]
#[strum(serialize_all = "snake_case")]
enum ManyDistinct {
    /// Exactly the most: fine.
    Thousand,
    /// One more than the most, in the default rule.
    ThousandAndOne,
    /// A default well below the most, but a tenant could raise it above.
    #[strum(props(
        min_at_most = 1,
        max_at_most = 1001,
        min_per_seconds = 60,
        max_per_seconds = 86400
    ))]
    TenantCouldAskForMore,
}

impl Limit for ManyDistinct {
    fn rule(self) -> Rule {
        Rule::WindowOfDistinct {
            at_most: Count::new(match self {
                Self::Thousand => MAX_DISTINCT_ELEMENTS,
                Self::ThousandAndOne => MAX_DISTINCT_ELEMENTS + 1,
                Self::TenantCouldAskForMore => 3,
            }),
            per: Period::hours(1),
        }
    }
}

fn built_with_distinct(limits: &'static [ManyDistinct]) -> Limits {
    // A pool that has not connected to anything: building alone reads every rule.
    let pool = PgPool::connect_lazy("postgres://nobody@localhost/nothing").unwrap();
    Limits::builder()
        .pool(pool)
        .keyed_hash(TestHash::new())
        .audit(common::NoAudit)
        .limits(limits)
        .run_jobs_in_background(|_error| {})
        .build()
}

#[tokio::test]
async fn a_window_of_a_thousand_distinct_elements_is_the_largest_that_builds() {
    let _limits = built_with_distinct(&[ManyDistinct::Thousand]);
}

#[tokio::test]
#[should_panic(
    expected = "the limit `thousand_and_one` could allow 1001 distinct elements; a window of distinct elements holds at most 1000"
)]
async fn a_window_of_more_distinct_elements_stops_the_module_from_being_built() {
    let _limits = built_with_distinct(&[ManyDistinct::ThousandAndOne]);
}

#[tokio::test]
#[should_panic(expected = "the limit `tenant_could_ask_for_more` could allow 1001 distinct")]
async fn bounds_that_let_a_tenant_ask_for_more_distinct_elements_stop_it_too() {
    let _limits = built_with_distinct(&[ManyDistinct::TenantCouldAskForMore]);
}

#[tokio::test]
async fn a_window_of_attempts_has_no_such_cap() {
    #[derive(
        Debug, Clone, Copy, strum::IntoStaticStr, strum::VariantArray, strum::EnumProperty,
    )]
    #[strum(serialize_all = "snake_case")]
    enum Attempts {
        AMillion,
    }
    impl Limit for Attempts {
        fn rule(self) -> Rule {
            Rule::Window {
                at_most: Count::new(1_000_000),
                per: Period::hours(1),
            }
        }
    }
    let pool = PgPool::connect_lazy("postgres://nobody@localhost/nothing").unwrap();
    let _limits = Limits::builder()
        .pool(pool)
        .keyed_hash(TestHash::new())
        .audit(common::NoAudit)
        .limits(Attempts::VARIANTS)
        .run_jobs_in_background(|_error| {})
        .build();
}
