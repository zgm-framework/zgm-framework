# ADR-0007: We connect to the database through `sqlx`, and all SQL lives in one `repository.rs`

> **Status:** accepted
> **Date:** 2026-11-10
> **Applies to:** `zgm-limits` (`Cargo.toml`, `src/repository.rs`, `examples/tour/main.rs`)

## Context

- **Counting an attempt is an SQL query** ([ADR-0006](0006-counting-in-one-sql-statement.md)), so the code must send it to PostgreSQL.
- **The other crates of the repository use `sqlx` with `tokio`,** with dates from `chrono` and identifiers from `uuid`.

## Options considered

| Option | Pros | Cons |
|---|---|---|
| **A. `sqlx`, `tokio`, `chrono`, `uuid`** | the same crates and versions as in the author's other crates; migrations from the `migrations/` directory | four crates; `sqlx` and `chrono` are before version 1.0 |
| **B. `tokio-postgres`** | a smaller crate | different from the rest of the repository; migrations must be run by hand |

## Decision

**A** (decided by the maintainer on 2026-11-10).

- **The crates, each with an exact version:** `sqlx` 0.9.0, `tokio` 1.54.1, `chrono` 0.4.45, `uuid` 1.26.1.
- **All SQL of the module lives in `src/repository.rs`,** in plain functions that take `&mut PgConnection` and return the module's types. No trait and no state.
- **Migrations are run by the module's `migrate` function** ([ADR-0019](0019-the-module-runs-its-migrations.md)).
- **The database address comes from the `DATABASE_URL` variable.** Without it the program ends with an error; there is no default address.

## Why

- **One database crate in the whole repository** means one way of writing queries and one version to keep track of.
- **No default address,** so the module never creates its schema `zgm_limits` ([ADR-0005](0005-counters-table-and-key.md)) in a database nobody pointed it at.
- **Checked on 2026-11-10:** none of the versions is yanked, OSV knows no vulnerability in any of them, the repositories are not archived. `uuid` 1.29.2 came out a day earlier, so we take the older 1.26.1.

## Consequences

- **`cargo audit` reports RUSTSEC-2026-0459 in `libsqlite3-sys`.** This crate is only listed in `Cargo.lock` by `sqlx` and is not compiled: `cargo tree -i libsqlite3-sys` does not find it in the program.
- **The `sqlx` repository on GitHub redirects from `launchbadge/sqlx` to `transact-rs/sqlx`.**
- **Running the program requires an empty PostgreSQL 18 database.**
- **The counting tests need a database** ([ADR-0010](0010-tests-in-the-tests-folder-on-a-database.md)).
