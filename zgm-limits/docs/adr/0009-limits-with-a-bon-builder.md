# ADR-0009: The entry point of the module is the `Limits` struct, built with a `bon` builder

> **Status:** accepted
> **Date:** 2026-11-10
> **Applies to:** `zgm-limits` (`src/limits.rs`, `src/error.rs`, `Cargo.toml`)

## Context

- **The application called `repository::hit_window` directly** and unpacked the rule of the limit itself. Choosing the query for a way of counting belongs to the module, not to the application.
- **The module will get more settings:** the hash function, the clock, the audit receiver, background jobs.

## Options considered

| Option | Pros | Cons |
|---|---|---|
| **A. `Limits` with a `bon` builder** | the entry point keeps its shape when settings are added; the modules in the rest of the repository are built this way | the `bon` crate; with one field the builder gives nothing yet |
| **B. `Limits::new(pool)`** | no crate | every new setting changes the call in every application |
| **C. Functions only, no struct** | the least code | the connection pool and every setting must be passed with every attempt |

## Decision

**A** (decided by the maintainer on 2026-11-10: "let's do it with a builder through bon").

- **`Limits::builder().pool(pool).keyed_hash(hmac).audit(audit).limits(AppLimit::VARIANTS).run_jobs_in_background(on_error).build()`.** The settings are the connection pool, the required list of limits whose rules `build()` reads at startup ([ADR-0016](0016-rules-checked-at-build.md)), the required `KeyedHash` ([ADR-0013](0013-keyed-hash-from-the-application.md)), the required receiver of events `Audit` ([ADR-0021](0021-events-instead-of-logs.md)), the required step of the cleanup loop ([ADR-0015](0015-cleanup-in-a-job-loop.md)) and, only in tests, the clock ([ADR-0011](0011-clock-from-the-builder.md)).
- **Attempts are counted by the `Attempts` object,** which the application builds with `limits.attempts(AppLimit::Export).tenant_id(...)` ([ADR-0014](0014-attempts-object-hit-reset.md)). Its `hit()` picks the query of the way of counting in one `match` on `Rule`; for a window it calls `repository::hit_window`. The zeros in place of the account and the empty hash are filled in by the module, not by the caller.
- **`attempts` takes any type with the `Limit` trait** (`src/rule.rs`). The trait has one method, `rule()`, and requires `Copy` and `Into<&'static str>`, that is, the name that `strum` gives ([ADR-0003](0003-limit-names-and-list-through-strum.md)). The product writes `impl Limit for AppLimit` with one `match` (decided by the maintainer on 2026-11-10).
- **Every method takes a connection from the pool and reads the module's clock.**
- **For `Rule::Bucket`, `hit()` returns the error `LimitsError::BucketNotImplemented`** and does not panic.
- **`LimitsError` gets `Display`, `Error` and `From` from the `thiserror` crate** (2.0.21, the same version as in the rest of the repository; decided by the maintainer on 2026-11-10). A new error is one variant with a message, not three places to fill in.
- **The `Limits` struct itself has no generic types;** only the `attempts` method is generic.

## Why

- **There will be more settings,** and a builder lets us add them without changing the existing calls.
- **One `match` on `Rule` in the module** is the only place to extend when bucket counting is added ([ADR-0002](0002-window-from-the-first-attempt-and-rule-as-an-enum.md)).
- **`bon` 3.10.2 and `thiserror` 2.0.21 are the versions used in the rest of the repository.** Checked on 2026-11-10: they are not yanked, OSV knows no vulnerabilities in them, and their repositories are not archived.

## Consequences

- **The name of one limit cannot be given with the rule of another,** because `attempts` takes both from the same value of the enum.
- **The trait is the only way for the module to call `rule()` on the product's enum,** which it does not know. A plain method `AppLimit::rule` could be called only by the product's code.
- **Migrations are run by `zgm_limits::migrate`,** separately from the builder ([ADR-0019](0019-the-module-runs-its-migrations.md)).
