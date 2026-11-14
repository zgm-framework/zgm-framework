# Changelog

## 0.1.0 (2026-11-14)

The first version.

- **Counting attempts** in a window from the first attempt, one statement per attempt: `Attempts::hit`, `Attempts::reset`.
- **Counting distinct elements** in a window, at most 1000: `Attempts::hit_distinct`.
- **Keys** by tenant, account and a hidden value, which reaches the database only as a hash with a key from the application (`KeyedHash`).
- **Rules:** defaults in code (`Limit::rule`), overridable by a tenant within bounds from the code: `get_rules`, `override_rule`, `restore_default`.
- **Blocked keys:** `get_blocked` in pages of 100, and `unblock`.
- **Events** to the application's `Audit`: changes in the transaction of the change, attempts after the fact.
- **Cleanup** of old counters in the module's own loop, and **migrations** run by the module (`migrate`).
