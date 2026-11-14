//! The rule of a limit.

use std::num::NonZeroU32;
use std::time::Duration;

/// One limit of the product. The product implements it for its enum of limits, and the module
/// then takes any limit of that enum.
///
/// The name of the limit comes from `Into<&'static str>`, which `strum::IntoStaticStr` derives;
/// it is the name the counters are kept under, so it must not change.
///
/// What a tenant may change the rule to comes from four properties over the variant, which
/// `strum::EnumProperty` derives; see [`Bounds`].
pub trait Limit: Copy + Into<&'static str> + strum::EnumProperty {
    /// The rule of the limit: the default, which a tenant may replace within the bounds.
    fn rule(self) -> Rule;
}

/// How far a tenant may move the rule of a limit away from the default.
///
/// The product writes them over the variant of its enum, all four or none:
///
/// ```ignore
/// #[strum(props(min_at_most = 1, max_at_most = 20, min_per_seconds = 60, max_per_seconds = 86400))]
/// WrongPassword,
/// ```
///
/// A limit with none cannot be changed by a tenant at all. Only some of the four, e.g. after
/// a slip in a name, is a panic when the module is built, so that a slip never takes a bound
/// away in silence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    /// The fewest attempts a window may let through.
    pub min_at_most: Count,
    /// The most attempts a window may let through.
    pub max_at_most: Count,
    /// The shortest a window may last.
    pub min_per: Period,
    /// The longest a window may last.
    pub max_per: Period,
}

/// The most distinct elements a [`Rule::WindowOfDistinct`] may allow. The elements of a window
/// lie in the row of its counter, and every new one rewrites that row, so with thousands of
/// them an attempt gets slow. Not a number from any norm: the size up to which this way of
/// counting works.
pub const MAX_DISTINCT_ELEMENTS: u32 = 1000;

/// Checks that a limit which counts distinct elements can never be asked for more than
/// [`MAX_DISTINCT_ELEMENTS`]: neither by its default rule nor, through its bounds, by a tenant.
///
/// # Panics
///
/// When it can. A mistake in the code of the product, found when the module is built.
pub(crate) fn check_distinct_elements(name: &str, rule: Rule, bounds: Option<Bounds>) {
    let Rule::WindowOfDistinct { at_most, .. } = rule else {
        return;
    };
    let highest = bounds.map_or(at_most, |bounds| bounds.max_at_most.max(at_most));
    assert!(
        highest.get() <= MAX_DISTINCT_ELEMENTS,
        "the limit `{name}` could allow {} distinct elements; a window of distinct elements \
         holds at most {MAX_DISTINCT_ELEMENTS}",
        highest.get(),
    );
}

/// The names of the four properties, in the order of the fields of [`Bounds`].
const BOUND_NAMES: [&str; 4] = [
    "min_at_most",
    "max_at_most",
    "min_per_seconds",
    "max_per_seconds",
];

impl Bounds {
    /// The bounds written over `limit`, or `None` when it has none.
    ///
    /// # Panics
    ///
    /// When the limit has only some of the four, when one is not a number from 1 to
    /// `u32::MAX`, when a lowest is above its highest, or when the default rule of the limit is
    /// itself outside them. All are mistakes in the code of the product, found when the
    /// module is built.
    pub(crate) fn of(limit: impl Limit) -> Option<Self> {
        let name: &'static str = limit.into();
        let found = BOUND_NAMES.map(|property| limit.get_int(property));
        let given = found.iter().flatten().count();
        if given == 0 {
            return None;
        }
        assert!(
            given == BOUND_NAMES.len(),
            "the limit `{name}` has only some of the bounds: give all of {BOUND_NAMES:?} or none",
        );
        let [min_at_most, max_at_most, min_per, max_per] = found.map(|value| {
            let value = value.expect("all four are given");
            u32::try_from(value)
                .ok()
                .filter(|&value| value > 0)
                .unwrap_or_else(|| {
                    panic!(
                        "a bound of the limit `{name}` must be from 1 to {}",
                        u32::MAX
                    )
                })
        });
        let bounds = Self {
            min_at_most: Count::new(min_at_most),
            max_at_most: Count::new(max_at_most),
            min_per: Period::seconds(min_per),
            max_per: Period::seconds(max_per),
        };
        assert!(
            bounds.min_at_most <= bounds.max_at_most && bounds.min_per <= bounds.max_per,
            "a lowest bound of the limit `{name}` is above its highest",
        );
        assert!(
            bounds.problem_with(limit.rule()).is_none(),
            "the default rule of the limit `{name}` is outside its own bounds",
        );
        Some(bounds)
    }

    /// Which number of `rule` is outside the bounds, if any: its name, the number, and the
    /// lowest and highest allowed.
    pub(crate) fn problem_with(&self, rule: Rule) -> Option<(&'static str, u32, u32, u32)> {
        let (how_many, how_long) = match rule {
            Rule::Window { at_most, per } | Rule::WindowOfDistinct { at_most, per } => {
                (at_most, per)
            }
            Rule::Bucket {
                capacity,
                refill_every,
            } => (capacity, refill_every),
        };
        if how_many < self.min_at_most || how_many > self.max_at_most {
            Some((
                "at_most",
                how_many.get(),
                self.min_at_most.get(),
                self.max_at_most.get(),
            ))
        } else if how_long < self.min_per || how_long > self.max_per {
            Some((
                "per_seconds",
                how_long.as_seconds(),
                self.min_per.as_seconds(),
                self.max_per.as_seconds(),
            ))
        } else {
            None
        }
    }
}

/// What a limit allows. Every variant is one way of counting, with its own fields.
///
/// Its fields are [`Count`] and [`Period`], which hold no value with which a limit would not
/// limit: no zero attempts and no window of no time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// A window from the first attempt: the first attempt opens a window that lasts `per`,
    /// `at_most` attempts pass in it, and after it the count starts from zero.
    Window {
        /// How many attempts pass in one window.
        at_most: Count,
        /// How long a window lasts.
        per: Period,
    },
    /// The same window, filled by distinct elements instead of attempts: the first attempt
    /// opens a window that lasts `per`, and `at_most` different elements fit in it, e.g. at
    /// most ten different accounts tried from one address. An attempt on an element that is
    /// already in the window always passes; a new element is refused once the window is full.
    /// Counted with [`Attempts::hit_distinct`](crate::Attempts::hit_distinct).
    WindowOfDistinct {
        /// How many different elements fit in one window.
        at_most: Count,
        /// How long a window lasts.
        per: Period,
    },
    /// A bucket with tokens: the bucket holds `capacity` tokens, an attempt takes one, and one
    /// comes back every `refill_every`. An empty bucket refuses. Counting is not implemented yet.
    Bucket {
        /// How many tokens a full bucket holds.
        capacity: Count,
        /// How often one token comes back.
        refill_every: Period,
    },
}

impl Rule {
    /// The name of the way of counting, as the table of the tenants' rules keeps it, with the
    /// two numbers of the rule. `None` for a way of counting that cannot be counted yet.
    pub(crate) fn kind_and_numbers(&self) -> Option<(&'static str, Count, Period)> {
        match *self {
            Self::Window { at_most, per } => Some(("window", at_most, per)),
            Self::WindowOfDistinct { at_most, per } => Some(("window_of_distinct", at_most, per)),
            Self::Bucket { .. } => None,
        }
    }

    /// The rule of the way of counting `kind` with these two numbers; `None` for a name this
    /// version of the module does not know.
    pub(crate) fn of_kind(kind: &str, at_most: Count, per: Period) -> Option<Self> {
        match kind {
            "window" => Some(Self::Window { at_most, per }),
            "window_of_distinct" => Some(Self::WindowOfDistinct { at_most, per }),
            _ => None,
        }
    }
}

/// How many attempts: always at least one.
///
/// Zero cannot be written. In a constant it does not compile:
///
/// ```compile_fail
/// const NONE: zgm_limits::Count = zgm_limits::Count::new(0);
/// ```
///
/// Written straight into `rule()` it panics when the rule is first read, which
/// `Limits::builder().limits(...)` does for every limit when the module is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Count(NonZeroU32);

impl Count {
    /// `count` attempts.
    ///
    /// # Panics
    ///
    /// When `count` is zero.
    pub const fn new(count: u32) -> Self {
        match NonZeroU32::new(count) {
            Some(count) => Self(count),
            None => panic!("a count must be at least 1"),
        }
    }

    /// The number.
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

/// How long: a whole number of seconds, at least one and at most `u32::MAX`, about 136 years.
///
/// Neither no time nor more than that can be written. In a constant it does not compile:
///
/// ```compile_fail
/// const NO_TIME: zgm_limits::Period = zgm_limits::Period::seconds(0);
/// ```
///
/// ```compile_fail
/// const TOO_LONG: zgm_limits::Period = zgm_limits::Period::days(50_000);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Period(NonZeroU32);

impl Period {
    /// `seconds` seconds.
    ///
    /// # Panics
    ///
    /// When `seconds` is zero.
    pub const fn seconds(seconds: u32) -> Self {
        match NonZeroU32::new(seconds) {
            Some(seconds) => Self(seconds),
            None => panic!("a period must be at least 1 second"),
        }
    }

    /// `minutes` minutes.
    ///
    /// # Panics
    ///
    /// When `minutes` is zero or more than fits in a period.
    pub const fn minutes(minutes: u32) -> Self {
        Self::of(minutes, 60)
    }

    /// `hours` hours.
    ///
    /// # Panics
    ///
    /// When `hours` is zero or more than fits in a period.
    pub const fn hours(hours: u32) -> Self {
        Self::of(hours, 60 * 60)
    }

    /// `days` days of 24 hours.
    ///
    /// # Panics
    ///
    /// When `days` is zero or more than fits in a period.
    pub const fn days(days: u32) -> Self {
        Self::of(days, 24 * 60 * 60)
    }

    /// `number` units of `seconds_in_unit` seconds each.
    const fn of(number: u32, seconds_in_unit: u32) -> Self {
        match number.checked_mul(seconds_in_unit) {
            Some(seconds) => Self::seconds(seconds),
            None => panic!("a period must fit in u32::MAX seconds, about 136 years"),
        }
    }

    /// The period in seconds.
    pub const fn as_seconds(self) -> u32 {
        self.0.get()
    }

    /// The period as a [`Duration`].
    pub const fn as_duration(self) -> Duration {
        Duration::from_secs(self.0.get() as u64)
    }
}
