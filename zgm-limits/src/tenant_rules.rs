//! The rules of the limits: the defaults from the code and what a tenant set for itself.

use crate::rule::{Bounds, Rule};

/// One limit with its default rule, the one from the code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefaultRule {
    /// The name of the limit.
    pub limit_name: &'static str,
    /// Its rule in the code.
    pub rule: Rule,
    /// How far a tenant may move the rule; `None`: a tenant cannot change this limit.
    pub bounds: Option<Bounds>,
}

/// One limit with the rule in force in a tenant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TenantRule {
    /// The name of the limit.
    pub limit_name: &'static str,
    /// The rule attempts in this tenant are counted by.
    pub rule: Rule,
    /// Whether the tenant set this rule itself. `false`: it is the default from the code.
    pub overridden: bool,
    /// How far the tenant may move the rule; `None`: it cannot change this limit.
    pub bounds: Option<Bounds>,
}
