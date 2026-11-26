//! The kinds of value a setting can have.

use std::time::Duration;

use crate::repository::StoredValue;

/// The value of a setting, of one of the kinds the module knows. Knowing the kind, the module
/// can refuse a value of another kind for a setting, and an application can show each value
/// the way it should be shown, e.g. 90 days rather than a number of seconds.
///
/// In the database each is plain JSON: a duration is its whole seconds (`7776000` for 90
/// days), a number is the number, a flag is `true` or `false`, and a text is a string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// A length of time, e.g. how long a user waits for deletion.
    Duration(Duration),
    /// A whole number.
    Number(i64),
    /// Yes or no.
    Flag(bool),
    /// A text.
    Text(String),
}

/// A value the way it goes into the table.
pub(crate) enum Json<'a> {
    /// The text of a JSON number or boolean, which the database reads as JSON.
    Literal(String),
    /// A text, which the database turns into a JSON string, so that the module escapes
    /// nothing itself.
    Text(&'a str),
}

impl Value {
    /// The word of the kind in the column `kind` of the defaults.
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Value::Duration(_) => "duration",
            Value::Number(_) => "number",
            Value::Flag(_) => "flag",
            Value::Text(_) => "text",
        }
    }

    /// Whether `self` is of the same kind as `other`, whatever their values.
    pub(crate) fn is_of_the_kind_of(&self, other: &Value) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    /// The value the way it goes into the table, or `None` for a duration with a fraction of
    /// a second, which the table cannot keep.
    pub(crate) fn to_json(&self) -> Option<Json<'_>> {
        match self {
            Value::Duration(duration) if duration.subsec_nanos() != 0 => None,
            _ => Some(self.to_json_in_whole_seconds()),
        }
    }

    /// The value the way it goes into the table, a duration with the fraction of a second cut
    /// off.
    pub(crate) fn to_json_in_whole_seconds(&self) -> Json<'_> {
        match self {
            Value::Duration(duration) => Json::Literal(duration.as_secs().to_string()),
            Value::Number(number) => Json::Literal(number.to_string()),
            Value::Flag(flag) => Json::Literal(flag.to_string()),
            Value::Text(text) => Json::Text(text),
        }
    }

    /// The value a row of the defaults holds, read as the kind its column `kind` names;
    /// `None` for a kind the module does not know, or a value not of the kind named.
    pub(crate) fn read_as(kind: &str, stored: &StoredValue) -> Option<Value> {
        [
            Value::Duration(Duration::ZERO),
            Value::Number(0),
            Value::Flag(false),
            Value::Text(String::new()),
        ]
        .into_iter()
        .find(|like| like.kind() == kind)
        .and_then(|like| like.read_like(stored))
    }

    /// The value a row holds, read as the kind of `self`, the default of its setting; `None`
    /// when the row holds another kind.
    pub(crate) fn read_like(&self, stored: &StoredValue) -> Option<Value> {
        let text = stored.text.as_str();
        match (self, stored.json_type.as_str()) {
            // Whole seconds that are not negative; a fraction or a sign does not parse.
            (Value::Duration(_), "number") => text
                .parse()
                .ok()
                .map(|seconds| Value::Duration(Duration::from_secs(seconds))),
            (Value::Number(_), "number") => text.parse().ok().map(Value::Number),
            (Value::Flag(_), "boolean") => text.parse().ok().map(Value::Flag),
            (Value::Text(_), "string") => Some(Value::Text(text.to_owned())),
            _ => None,
        }
    }
}
