# ADR-0004: The `migrate` function creates the module's tables, and the migration history lives in the module's schema

> **Status:** accepted
> **Date:** 2026-11-23
> **Applies to:** `zgm-settings` (`src/migrate.rs`, `migrations/`, `tests/migrate.rs`)

## Context

- **An application uses the module as a library** and needs a way to create its tables.
- **By default `sqlx` keeps the migration history in the `_sqlx_migrations` table in the `public` schema,** shared by everybody who uses it in that database. A migrator sees other users' entries there as its own migrations that it does not know, and fails with an error.
- **`zgm-limits` and `zgm-iam` have the same need and solved it** with a `migrate` function and a history table in their own schema.

## Options considered

| Option | Pros | Cons |
|---|---|---|
| **A. A `migrate(pool)` function in the module, with the history in `zgm_settings._sqlx_migrations`** | the application calls one function; the module's migrations and the application's do not see each other; the same as in `zgm-limits` and `zgm-iam` | one database account creates the tables and uses them |
| **B. The application copies the migration file into its own migrations** | the module has no migration code | every change to the module's schema requires manual work in every application |

## Decision

**A** (accepted by the maintainer on 2026-11-23, with the design of the table, which proposed it as in `zgm-iam`).

- **`zgm_settings::migrate(&pool)`** creates or updates the module's tables. The application calls it at deployment or at startup, before it uses anything else of the module.
- **Everything of the module lives in the `zgm_settings` schema,** and the migration history lives in `zgm_settings._sqlx_migrations`. This name cannot change after the first deployment.
- **The migrator creates the `zgm_settings` schema,** so that it has a place to put the history. The migration file has `CREATE SCHEMA IF NOT EXISTS`, so it also works when run directly.
- **A migration error is `SettingsError::Migration`.**

## Why

- **The application has its own migrations.** Without a separate history table, its migrator and the module's migrator would each fail on the other's entries.
- **Checked by tests:** `migrate` on a database in which the application already has its own `_sqlx_migrations` with a migration unknown to the module succeeds and changes nothing in it; with the history in the default table the same test fails. Three calls at once also succeed, the module's history has one entry, both tables are in the module's schema, and nothing of the module lies outside it.

## Consequences

- **The account the application calls `migrate` with must be able to create a schema,** and it becomes the owner of the tables.
- **The module does not check at startup whether all migrations have been applied to the database.**
