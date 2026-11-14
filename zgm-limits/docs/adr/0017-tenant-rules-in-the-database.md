# ADR-0017: Rules in code are the defaults, a tenant overrides them in the `tenant_rules` table, and an open window is counted to its end by the rule it opened with

> **Status:** accepted
> **Date:** 2026-11-11
> **Applies to:** `zgm-limits` (`migrations/20261111095113_create_zgm_limits.sql`, `src/tenant_rules.rs`, `Limits::get_defaults`, `get_rules`, `override_rule`, `restore_default`, `repository::hit_window`)

## Context

- **The rules of the limits were only in code,** so changing a value required a deployment and applied to all tenants at once.
- **The maintainer wants the values in code to be defaults that a tenant can override,** together with the way of counting, once there is more than one (2026-11-11).
- **The list of blocked keys relies on `blocked_at`,** which an attempt writes according to the rule in force at that moment ([ADR-0012](0012-list-of-blocked-keys.md)). After a rule changed in the middle of a window, the list and the counting could disagree: once the threshold was lowered, somebody was refused but was not on the list.

## Options considered

**Where an attempt learns the tenant's rule:**

| Option | Pros | Cons |
|---|---|---|
| **A. In the same query as the counting** | a change takes effect from the next attempt on every server; still one query per attempt | the attempt's query reads one more table |
| **B. A cache in the application** | the attempt's query does not change | a change reaches the servers with a delay; state in memory that can be stale |
| **C. A separate query before the attempt** | simple queries | two round trips to the database per attempt |

**What happens to the counters that already exist when a rule changes:**

| Option | Pros | Cons |
|---|---|---|
| **A. A change deletes the counters of that limit in the tenant** | the new rule applies to everybody at once; the list of blocked keys agrees with the counting | with very many counters the delete takes seconds and holds all those rows for that time, so attempts on them wait and occupy connections; it lifts blocks that are in force |
| **B. The counters stay and are compared with the new rule** | a change is one write | after the threshold changes, the list of blocked keys is wrong until the end of the window: somebody is refused but is not on the list |
| **C. The window remembers the threshold it opened with** | a change is one write and holds nothing; the list of blocked keys always agrees with the counting; one rule: "a window is counted the way it started" | a change does not take effect at once for keys with an open window |
| **D. Versions of the rule, and the counter remembers the version number** | like C, plus a history of rules in the database | a second read for an attempt on an open window; old versions have to be stored and cleaned up |

## Decision

**Reading: A. Counters: C** (decided by the maintainer on 2026-11-11: "when a counter is created, the rule is copied and kept next to the counter (...), and later, when we want a new one, we set a new one and new counters count by the new one").

- **The `zgm_limits.tenant_rules` table:** one row per tenant and limit pair, only for the rules the tenant has changed. The primary key is `(tenant_id, limit_name)`.
- **The `rule` column (`jsonb`) holds the way of counting and its numbers,** e.g. `{"kind": "window", "at_most": 10, "per_seconds": 3600}`. So the tenant also chooses the way of counting.
- **No row means the default rule from code.** The default values are not in the database.
- **Four methods:**
  - `limits.get_defaults()` returns all limits with the rules from code, without a database query;
  - `limits.get_rules().tenant_id(...)` returns all limits with the rule in force in the tenant and an `overridden` flag;
  - `limits.override_rule(limit).tenant_id(...).rule(...)` saves the tenant's rule;
  - `limits.restore_default(limit).tenant_id(...)` deletes the tenant's row.
- **The attempt's query reads the tenant's row by primary key** when it opens a new window; when there is no row, it takes the default values passed in from code.
- **When a window opens, the `at_most` threshold is copied into the counter,** next to the number of attempts: `{"count": 1, "at_most": 3}`. The length of the window is already in the row as its end (`expires_at`). A window in progress is counted by its own copy.
- **Changing a rule deletes nothing and lifts no block.** The new rule is for windows opened from that moment on.
- **Saving the same rule a second time changes nothing.** `override_rule` and `restore_default` return `true` only when something changed.
- **`Rule::Window` and `Rule::WindowOfDistinct` can be saved,** but only of the same kind as the limit's default rule; another kind is `LimitsError::RuleOfAnotherKind` ([ADR-0022](0022-window-of-distinct-elements.md)). `Rule::Bucket` returns `LimitsError::BucketNotImplemented`, so that a tenant cannot set a rule on which every attempt fails.
- **A limit that is not on the builder's list is `LimitsError::UnknownLimit`.**

## Why

- **A change has to take effect at once and everywhere.** Hence the read in the attempt's query, with no cache.
- **Reading the tenant's rule costs nothing that can be measured.** On PostgreSQL 18, with 100,000 counters and 20,000 `tenant_rules` rows, 8 connections and refusals only, six alternating runs gave 43,000–46,000 queries per second without the read and 41,000–46,000 with it. A first, shorter measurement showed a loss of 20–40%; it was machine noise, which disappeared when the measurement was repeated.
- **The copy of the threshold in the counter closes the gap between the list of blocked keys and the counting:** a window and its block are counted by the same number.
- **The first version of this decision deleted the counters on every change.** A review under a flood of requests showed the cost: deleting 100,000 counters took up to 2.5 seconds, and the attempts waiting for those rows occupied every connection in the pool, so refusals and other tenants stalled too. Deleting in batches would have required a loop, a background job, and rules for counters created in the meantime.
- **A copy instead of versions,** because an attempt then reads one row and has everything in it, and the copy disappears together with the counter. The history of rule changes comes from the `RuleChanged` events.
- **A bad rule cannot be saved,** because `override_rule` takes a `Rule` built from `Count` and `Period` ([ADR-0016](0016-rules-checked-at-build.md)).

## Consequences

- **A change does not take effect at once for keys with an open window.** Tightening a limit does not stop somebody who is halfway through a window; loosening it does not unblock those already blocked. This lasts at most as long as the window.
- **The admin unblocks blocked keys from the list** ([ADR-0012](0012-list-of-blocked-keys.md)). After unblocking, the key opens a window by the new rule.
- **The application decides who may call `override_rule` and `restore_default`.** What a rule may be changed to is set by the limit's bounds in code; a limit without bounds cannot be changed ([ADR-0018](0018-override-bounds-in-strum-attributes.md)).
- **The bounds from code are applied when a window opens.** A window opened before the bounds were narrowed runs to its end with its own threshold.
- **A new default value in code works the same way as a change by a tenant:** it reaches the tenants without a rule of their own, from the next window.
- **The row of a limit that is no longer in code stays in the database,** and `get_rules` skips it.
- **A tenant's rule equal to the default is a row too:** the tenant keeps it when the default value in code changes.
- **A tenant does not change the kind of counting.** The `rule` column stores the kind so that the query takes only a rule of its own kind, and whether a limit counts attempts or elements is decided by the code that calls `hit` or `hit_distinct` ([ADR-0022](0022-window-of-distinct-elements.md)).
- **A rule change reports a `RuleChanged` event** ([ADR-0021](0021-events-instead-of-logs.md)).
- **There is no "delete the counters of this limit" operation.** If immediate tightening turns out to be needed, it will be a separate method that is called deliberately.
