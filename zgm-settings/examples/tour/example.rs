//! The side of a module that has settings, the way `zgm-iam` will have them: an enum of its
//! settings, named through `strum` with the prefix of the module, and the defaults it gives
//! when it is built. And how a screen of a program might show a value.

use std::time::Duration;

use zgm_settings::{Settings, Value};

/// The settings of the module, one of every kind of [`Value`]. `strum` makes the name: the
/// prefix of the module, then the variant in snake case, e.g. `iam.user_deletion_window`.
#[derive(Clone, Copy, Debug, strum::AsRefStr)]
#[strum(serialize_all = "snake_case", prefix = "iam.")]
pub enum IamSetting {
    /// How long a user waits for deletion.
    UserDeletionWindow,
    /// How many users a company may have.
    MaxUsers,
    /// Whether people may sign up to a company by themselves.
    SelfSignUp,
    /// What a new user sees first.
    WelcomeText,
}

const DAY: u64 = 24 * 60 * 60;

pub fn days(count: u64) -> Duration {
    Duration::from_secs(count * DAY)
}

/// A module with settings. It gives its defaults when it is built; a program that wants
/// other defaults for every company would say so in the module's builder.
pub struct ExampleIam;

impl ExampleIam {
    pub fn build(settings: &Settings) -> Self {
        settings.add_default(IamSetting::UserDeletionWindow, Value::Duration(days(30)));
        settings.add_default(IamSetting::MaxUsers, Value::Number(10));
        settings.add_default(IamSetting::SelfSignUp, Value::Flag(false));
        settings.add_default(IamSetting::WelcomeText, Value::Text("Welcome".to_owned()));
        Self
    }
}

/// A value the way a screen of settings might show it: knowing the kind, it shows 90 days
/// rather than 7776000.
pub fn shown(value: &Value) -> String {
    match value {
        Value::Duration(duration) if duration.as_secs() % DAY == 0 => {
            format!("{} days", duration.as_secs() / DAY)
        }
        Value::Duration(duration) => format!("{} s", duration.as_secs()),
        Value::Number(number) => number.to_string(),
        Value::Flag(true) => "yes".to_owned(),
        Value::Flag(false) => "no".to_owned(),
        Value::Text(text) => format!("{text:?}"),
    }
}
