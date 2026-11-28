# ADR-0003: The `migrate` function creates the module's tables, and the migration history lives in the module's schema

> **Status:** accepted
> **Date:** 2026-11-15
> **Applies to:** `zgm-iam` (`src/migrate.rs`, `migrations/`)

## Context

- **An application uses the module as a library** and needs a way to create its tables.
- **By default `sqlx` keeps the migration history in the `_sqlx_migrations` table in the `public` schema,** shared by everybody who uses it in that database. A migrator sees other users' entries there as its own migrations that it does not know, and fails with an error.
- **`zgm-limits` has the same need and solved it** with a `migrate` function and a history table in its own schema.

## Options considered

| Option | Pros | Cons |
|---|---|---|
| **A. A `migrate(pool)` function in the module, with the history in `zgm_iam._sqlx_migrations`** | the application calls one function; the module's migrations and the application's do not see each other; the same as in `zgm-limits` | one database account creates the tables and uses them |
| **B. The same plus a grant of privileges to a separate application account** | the application's account does not own the tables | a second argument, a list of tables to keep up to date with every migration, and a lock while the privileges are granted |
| **C. The application copies the migration file into its own migrations** | the module has no migration code | every change to the module's schema requires manual work in every application |

## Decision

**A** (decided by the maintainer on 2026-11-15: the same as in `zgm-limits`).

- **`zgm_iam::migrate(&pool)`** creates or updates the module's tables. The application calls it at deployment or at startup, before it uses anything else of the module.
- **Everything of the module lives in the `zgm_iam` schema,** and the migration history lives in `zgm_iam._sqlx_migrations`. This name cannot change after the first deployment.
- **The migrator creates the `zgm_iam` schema,** so that it has a place to put the history. The migration file has `CREATE SCHEMA IF NOT EXISTS`, so it also works when run directly.
- **A migration error is `IamError::Migration`.**

## Why

- **The application has its own migrations.** Without a separate history table, its migrator and the module's migrator would each fail on the other's entries.
- **Checked by a test:** `migrate` on a database in which the application already has its own `_sqlx_migrations` with a migration unknown to the module succeeds and changes nothing in it; with the history in the default table the same test fails with `VersionMissing`. Three calls at once also succeed, the module's history has one entry, and nothing of the module lies outside its schema.

## Consequences

- **The account the application calls `migrate` with must be able to create a schema,** and it becomes the owner of the tables.
- **There is no separate migration account and no granting of privileges.** An application that wants to separate them calls `migrate` on the pool of the migration account and grants the application's account privileges on the tables in `zgm_iam` itself.
- **The module does not check at startup whether all migrations have been applied to the database.**
