# ADR-0006: The module's dates are `jiff::Timestamp`, read from the database through `jiff-sqlx`

> **Status:** accepted
> **Date:** 2026-11-20
> **Applies to:** `zgm-iam` (`src/tenant.rs`, `src/repository.rs`, `Cargo.toml`)

## Context

- **`Tenant` gives back two dates,** `created_at` and `updated_at` ([ADR-0005](0005-iam-with-a-bon-builder.md)), so a type of dates is part of the module's API.
- **`sqlx` 0.9.0 reads `timestamptz` into the types of two crates,** `chrono` and `time`. `zgm-limits` uses `chrono`.
- **`chrono` is marked unmaintained.** RustSec, RUSTSEC-2026-0543 of 2026-09-14: "The chrono crate is unmaintained; all versions are affected." It recommends `jiff` and `time`, and the maintainer of `chrono` wrote that `jiff` is "an alternative I feel comfortable recommending".

## Options considered

| Option | Pros | Cons |
|---|---|---|
| **A. `chrono` 0.4.45, as in `zgm-limits`** | one type of dates in the repository; `sqlx` reads it itself | nobody fixes the next vulnerability found in it |
| **B. `jiff` 1.0.1 with `jiff-sqlx` 1.0.0** | maintained; the alternative the maintainer of `chrono` recommends; `jiff-sqlx` comes from the same repository as `jiff` | version 1.0 is days old; two types of dates in the repository |
| **C. `time` 0.3.55** | maintained; `sqlx` reads it itself | two types of dates in the repository |

## Decision

**B** (decided by the maintainer on 2026-11-20: "then maybe let's do B already").

- **The dates of `Tenant` are `jiff::Timestamp`,** a moment in UTC.
- **`repository.rs` reads `timestamptz` as `jiff_sqlx::Timestamp`** and turns it into `jiff::Timestamp` with `to_jiff()`.
- **`jiff` comes without its default features, with `std` only:** no reading of the system's time zone and no database of time zones. The module keeps moments, not local times.
- **`jiff-sqlx` comes with `postgres` only.**

## Why

- **The risk of `chrono` is not a known vulnerability but the lack of a fix for the next one.** OSV knows one vulnerability in its history, RUSTSEC-2020-0159, in reading the local time zone, fixed in 0.4.20. The maintainer chose not to wait: "wait, if chrono is abandoned, it may have some vulnerabilities".
- **`jiff` over `time`:** it is the alternative the maintainer of `chrono` recommends.
- **Checked on 2026-11-20:** neither version is yanked, OSV knows no vulnerabilities in either, the repository is not archived, has 69 contributors and was last changed on 2026-11-18. `cargo audit` reports nothing new after they were added.

## Consequences

- **The versions are very new:** `jiff` 1.0.1 was published on 2026-11-18 and `jiff-sqlx` 1.0.0 on 2026-11-15. The maintainer accepted it.
- **`zgm-limits` still uses `chrono`,** so the repository has two types of dates, and an application that uses both modules gets both crates.
- **An application that reads the dates of a tenant needs `jiff` too,** or converts them, e.g. through `as_second()`.
- **Six crates came into `Cargo.lock`:** `jiff`, `jiff-core`, `jiff-sqlx`, `jiff-static`, `portable-atomic` and `portable-atomic-util`.
