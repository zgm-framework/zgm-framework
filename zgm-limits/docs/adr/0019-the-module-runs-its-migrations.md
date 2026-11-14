# ADR-0019: The `migrate` function creates the module's tables, and the migration history lives in the module's schema

> **Status:** accepted
> **Date:** 2026-11-11
> **Applies to:** `zgm-limits` (`src/migrate.rs`, `migrations/`, `examples/tour/main.rs`)

## Context

- **The migration was run by the example's `main.rs`, through `sqlx::migrate!()`.** An application that uses the module as a library had no way to create its tables.
- **By default `sqlx` keeps the migration history in the `_sqlx_migrations` table in the `public` schema,** shared by everybody who uses it in that database. A migrator sees other users' entries there as its own migrations that it does not know, and fails with an error.

## Options considered

| Option | Pros | Cons |
|---|---|---|
| **A. A `migrate(pool)` function in the module, with the history in `zgm_limits._sqlx_migrations`** | the application calls one function; the module's migrations and the application's do not see each other | one database account creates the tables and uses them |
| **B. The same plus a grant of privileges to a separate application account** | the application's account does not own the tables | a second argument, a list of tables to keep up to date with every migration, and a lock while the privileges are granted |
| **C. The application copies the migration file into its own migrations** | the module has no migration code | every change to the module's schema requires manual work in every application |

## Decision

**A** (decided by the maintainer on 2026-11-11: "ok, go ahead", after a recommendation).

- **`zgm_limits::migrate(&pool)`** creates or updates the module's tables. The application calls it at deployment or at startup, before building `Limits`.
- **The migration history lives in `zgm_limits._sqlx_migrations`.** This name cannot change after the first deployment.
- **The migrator creates the `zgm_limits` schema,** so that it has a place to put the history. The migration file has `CREATE SCHEMA IF NOT EXISTS`, so it also works when run directly.
- **A migration error is `LimitsError::Migration`.**

## Why

- **The application has its own migrations.** Without a separate history table, its migrator and the module's migrator would each fail on the other's entries.
- **Checked by a test:** `migrate` on a database in which the application already has its own `_sqlx_migrations` with a migration unknown to the module succeeds and changes nothing in it. Three calls at once also succeed, and the module's history has one entry.

## Consequences

- **The account the application calls `migrate` with must be able to create a schema,** and it becomes the owner of the tables.
- **There is no separate migration account and no granting of privileges.** An application that wants to separate them calls `migrate` on the pool of the migration account and grants the application's account privileges on the tables in `zgm_limits` itself.
- **The module's tests still get their tables from `#[sqlx::test]`,** which applies the same file with the default history in the test database. `migrate` itself is checked by `tests/migrate.rs`.
- **The module does not check at startup whether all migrations have been applied to the database.** A missing table shows up as a database error on the first query.
