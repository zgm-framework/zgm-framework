# ADR-0023: Waiting for a held row is bounded by a database setting, not by the module

> **Status:** accepted
> **Date:** 2026-11-11
> **Applies to:** `zgm-limits` (`README.md`)

## Context

- **An attempt that passes writes the counter's row.** When another transaction holds that row, the attempt waits for it without a limit and holds a connection from the pool for that time.
- **A review under a flood of requests measured this:** behind a row held for 3 seconds an attempt waited 1.8 seconds; a refusal on the same row answered in 17 ms, because it writes nothing.
- **Many attempts on one held key occupy many connections,** so the pool can run empty for everybody.
- **The module itself holds rows briefly:** a rule change deletes nothing ([ADR-0017](0017-tenant-rules-in-the-database.md)), and a cleanup batch is 100 rows for a fraction of a second ([ADR-0015](0015-cleanup-in-a-job-loop.md)). Only a transaction from outside the module, e.g. one opened by hand, can hold a row for longer.

## Options considered

| Option | Pros | Cons |
|---|---|---|
| **A. The module sets a wait limit on every attempt** | the safeguard works without the application doing anything | an extra statement on every attempt or a transaction around it; a new error in the API; the time would be a constant of the module |
| **B. A recommendation in the README: `lock_timeout` and `idle_in_transaction_session_timeout` in the database** | no code; it also protects the application's queries; the second setting removes the cause, not only the effect | whoever does not set them has no safeguard |
| **C. Nothing** | nothing to do | one long transaction on a counter's row can occupy the whole pool |

## Decision

**B** (decided by the maintainer on 2026-11-11).

- **The README, in the "Connection pool" section, recommends both settings** with starting values of 2 s and 60 s, and explains that the first bounds the one who waits and the second the one who holds.
- **The module sets nothing and checks nothing.**

## Why

- **These are database settings that apply to every query of the application,** not only to limits. They fit with the recommendations for the pool ([ADR-0020](0020-recommended-pool-settings.md)).
- **The cause lies outside the module,** so that is where the remedy is too: `idle_in_transaction_session_timeout` disconnects a session that holds a row and does nothing.
- **The values depend on the deployment,** so they are set by whoever configures the system.

## Consequences

- **With `lock_timeout`, an attempt that waited too long ends with `LimitsError::Database`.** The application should then refuse the action.
- **Without these settings the module works, but waiting has no limit.**
- **The settings apply to all sessions of the database,** including migrations and manual work by an administrator.

## When to revisit

- When a deployment cannot change database settings and the pool is at times occupied by waiting attempts.
