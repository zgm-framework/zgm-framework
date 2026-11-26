//! The side of a module that has settings, the way `zgm-iam` will have them: an enum of its
//! settings, named through `strum` with the prefix of the module, and the defaults it gives
//! when it starts. And how a screen of a program might show a value.

use std::time::Duration;

use zgm_settings::{DefaultSetting, Settings, SettingsError, Value};

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

/// A module with settings. It gives its defaults when it starts: the first start writes them
/// to the database, and a later one, also of a version with other numbers, leaves them there.
pub struct ExampleIam;

impl ExampleIam {
    pub async fn start(settings: &Settings) -> Result<Self, SettingsError> {
        let defaults = [
            (
                IamSetting::UserDeletionWindow,
                DefaultSetting::Duration {
                    value: days(30),
                    min: Some(days(1)),
                    max: Some(days(365)),
                },
            ),
            (
                IamSetting::MaxUsers,
                DefaultSetting::Number {
                    value: 10,
                    min: Some(1),
                    max: None,
                },
            ),
            (IamSetting::SelfSignUp, DefaultSetting::Flag(false)),
            (
                IamSetting::WelcomeText,
                DefaultSetting::Text("Welcome".to_owned()),
            ),
        ];
        for (setting, default) in defaults {
            settings.add_default(setting, default).await?;
        }
        Ok(Self)
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
