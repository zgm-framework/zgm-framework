//! The default of a setting, as the module of the setting gives it.

use std::time::Duration;

use crate::value::Value;

/// The default of a setting, as the module of the setting gives it to
/// [`Settings::add_default`](crate::Settings::add_default): the value of every tenant that has
/// not changed the setting, and for a duration or a number the least and the most a tenant
/// may set.
///
/// Both bounds must be given, and `None` is no bound:
///
/// ```
/// use std::time::Duration;
/// use zgm_settings::DefaultSetting;
///
/// const DAY: Duration = Duration::from_secs(24 * 60 * 60);
///
/// // At least a day, and no most.
/// let window = DefaultSetting::Duration { value: DAY * 30, min: Some(DAY), max: None };
/// let self_sign_up = DefaultSetting::Flag(false);
/// ```
///
/// Nothing is checked: a least above the most, or a value outside its own bounds, is written
/// as it is given. A duration is written in whole seconds, the fraction of a second cut off,
/// and a bound of more seconds than a `bigint` holds as the most it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefaultSetting {
    /// A length of time, e.g. how long a user waits for deletion.
    Duration {
        /// The default.
        value: Duration,
        /// The shortest a tenant may set, or `None` for no bound.
        min: Option<Duration>,
        /// The longest a tenant may set, or `None` for no bound.
        max: Option<Duration>,
    },
    /// A whole number.
    Number {
        /// The default.
        value: i64,
        /// The least a tenant may set, or `None` for no bound.
        min: Option<i64>,
        /// The most a tenant may set, or `None` for no bound.
        max: Option<i64>,
    },
    /// Yes or no. It has no bounds.
    Flag(bool),
    /// A text. It has no bounds.
    Text(String),
}

impl DefaultSetting {
    /// The default as a [`Value`].
    pub(crate) fn value(&self) -> Value {
        match self {
            DefaultSetting::Duration { value, .. } => Value::Duration(*value),
            DefaultSetting::Number { value, .. } => Value::Number(*value),
            DefaultSetting::Flag(flag) => Value::Flag(*flag),
            DefaultSetting::Text(text) => Value::Text(text.clone()),
        }
    }

    /// The least and the most, as the table keeps them: a duration in whole seconds.
    pub(crate) fn bounds(&self) -> (Option<i64>, Option<i64>) {
        match self {
            DefaultSetting::Duration { min, max, .. } => {
                (min.map(whole_seconds), max.map(whole_seconds))
            }
            DefaultSetting::Number { min, max, .. } => (*min, *max),
            DefaultSetting::Flag(_) | DefaultSetting::Text(_) => (None, None),
        }
    }
}

/// The whole seconds of a duration as a `bigint`: the fraction cut off, and more than a
/// `bigint` holds, about 292 billion years, as the most it holds.
fn whole_seconds(duration: Duration) -> i64 {
    i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
}
