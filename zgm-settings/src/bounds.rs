//! The bounds of a duration or a number, as the table keeps them: the least and the most, a
//! duration in whole seconds, `None` for no bound. A flag or a text has none.

use crate::error::SettingsError;
use crate::value::Value;

/// Refuses a least above the most; a least equal to the most is allowed.
pub(crate) fn in_order(
    name: &str,
    value: &Value,
    min: Option<i64>,
    max: Option<i64>,
) -> Result<(), SettingsError> {
    if let (Some(min), Some(max)) = (min, max)
        && min > max
    {
        return Err(SettingsError::LeastAboveTheMost {
            name: name.to_owned(),
            min: value.bound_like(min),
            max: value.bound_like(max),
        });
    }
    Ok(())
}

/// Refuses a duration or a number below `min` or above `max`, a duration in whole seconds;
/// the bounds themselves are allowed, and a flag or a text has none.
pub(crate) fn within(
    name: &str,
    value: &Value,
    min: Option<i64>,
    max: Option<i64>,
) -> Result<(), SettingsError> {
    let Some(number) = value.as_bounded() else {
        return Ok(());
    };
    if let Some(min) = min
        && number < min
    {
        return Err(SettingsError::BelowTheLeast {
            name: name.to_owned(),
            min: value.bound_like(min),
        });
    }
    if let Some(max) = max
        && number > max
    {
        return Err(SettingsError::AboveTheMost {
            name: name.to_owned(),
            max: value.bound_like(max),
        });
    }
    Ok(())
}
