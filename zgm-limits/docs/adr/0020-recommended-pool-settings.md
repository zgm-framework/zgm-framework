# ADR-0020: We recommend a pool that does not test a connection before use and has a short idle timeout

> **Status:** accepted
> **Date:** 2026-11-11
> **Applies to:** `zgm-limits` (`README.md`, `examples/tour/main.rs`)

## Context

- **The module borrows connections from the application's pool,** one for each call.
- **By default the `sqlx` pool tests a connection before handing it out** (`test_before_acquire`), which means an extra round trip to the database before every query.
- **A performance review measured this through the module's Rust code:** without the test, refusals were faster in six pairs out of six, by 10–65%. The measurement went through Docker port forwarding, where a round trip is expensive.

## Options considered

| Option | Pros | Cons |
|---|---|---|
| **A. A recommendation in the README: `test_before_acquire(false)` and `idle_timeout(120 s)`** | one round trip fewer on every call; connections are closed before the network drops them | a dead connection shows up only at the query, as an error of one attempt |
| **B. Recommend nothing** | the default behavior of `sqlx`, with no errors after a dropped connection | two round trips on every call, felt most under a flood of refusals |
| **C. The module's own pool with its own settings** | the module does not depend on the application's settings | a second set of connections to the database; the application loses control over their number |

## Decision

**A** (decided by the maintainer on 2026-11-11).

- **The README has a "Connection pool" section** with the recommended code and an explanation of what each of the two settings gives and what is lost.
- **The example in `examples/tour/main.rs` builds its pool the same way.**
- **The module enforces nothing:** it accepts any pool.

## Why

- **With a stable database, the test is almost always a wasted round trip.**
- **Connections die mostly from idleness,** when a firewall or a load balancer drops an unused connection. A short `idle_timeout` closes them earlier, so it replaces the test where the test was needed most often.
- **The gain is where it matters:** under a flood of refusals. In an ordinary sign-in, checking the password takes tens of milliseconds and the saved round trip makes no difference.

## Consequences

- **After a database restart or failover, one request per dead connection gets `LimitsError::Database`.** The application should then refuse the action.
- **120 seconds is a starting value.** It should be shorter than the time after which the network of a given deployment drops unused connections.
- **The setting applies to the whole pool,** including the application's queries.
