# ADR-0001: The settings of tenants are a module of their own, which other modules reach through a trait; `zgm-limits` keeps its own table

> **Status:** accepted
> **Date:** 2026-11-23
> **Applies to:** `zgm-settings` (the whole crate), and the modules that will read settings through it

## Context

- **`zgm-iam` needs a number for its next function:** how many days a user waits for deletion before the row is gone. One company may want the default, another a number of its own.
- **`zgm-limits` already lets a tenant change its rules,** in its own `tenant_rules` table: the defaults are in code, a row holds only what a tenant changed, and the attempt reads that row in the same SQL statement that counts it ([ADR-0017 of `zgm-limits`](../../../zgm-limits/docs/adr/0017-tenant-rules-in-the-database.md)).
- **The modules of this repository do not depend on each other.** What one module needs from outside, such as a cipher or a hash, it takes through a trait in its builder, and the application gives the implementation.

## Options considered

| Option | Pros | Cons |
|---|---|---|
| **A. The number only in the builder of `zgm-iam`** | nothing new to build; the call that deletes a user is the same in every option | one number for every company |
| **B. A table of tenant settings inside `zgm-iam`, like `tenant_rules`** | a company has its own number; one SQL statement can read it | every module that needs settings builds the same table and the same functions again |
| **C. A module of settings, which other modules reach through a trait** | the functions that show, change and restore a setting are written once; all the settings of a company are in one place, e.g. for one screen of the application; the modules still do not depend on each other | a module to plan and build before `zgm-iam` goes on; a module reads a setting with a query of its own, not inside its own statement |

**Whether `zgm-limits` moves to this module too:**

| Option | Pros | Cons |
|---|---|---|
| **A. It keeps `tenant_rules`** | an attempt stays one statement; a change of a rule works at once on every server | two ways of keeping tenant settings in the repository |
| **B. It reads its rules through the trait** | one way of keeping tenant settings | one more round trip to the database on every attempt, or a cache in memory, in which a change does not work at once everywhere; a change of the API of `zgm-limits` |

## Decision

**C, and `zgm-limits` keeps its table (A)** (decided by the maintainer on 2026-11-23: "let's build these settings for now, and later they will come in handy in other modules too"; and on `zgm-limits`: "so settings do not fit limits, because there we bet heavily on performance").

- **`zgm-settings` keeps the settings of tenants** in its own schema, `zgm_settings`.
- **A module that has settings takes the settings module through a trait in its builder,** the way `zgm-iam` takes a `Cipher`. What the trait looks like is decided when `zgm-iam` first reads a setting.
- **`zgm-limits` reads its rules as before,** in the statement that counts the attempt.

## Why

- **Settings will be needed by more than one module,** and each would otherwise repeat the table, the functions and their tests.
- **For `zgm-limits` the cost is not worth it.** In the measurements in the README of `zgm-limits`, one round trip to the database takes 0.38 ms and a whole refused attempt 0.63 ms, so a second query would make a refused attempt over half as long again. That is an estimate, not a measurement. The same kind of change was measured there: one round trip fewer on every call (`test_before_acquire(false)`) gave 10–65% more refusals per second during a flood of attempts, and withstanding such a flood is what `zgm-limits` is for. A cache would avoid the round trip, but [ADR-0017 of `zgm-limits`](../../../zgm-limits/docs/adr/0017-tenant-rules-in-the-database.md) rejected it, because a change of a rule must work at once on every server.
- **For `zgm-iam` the round trip costs nothing that matters:** a user is deleted rarely.

## Consequences

- **The repository has two ways of keeping tenant settings:** this module, and `tenant_rules` in `zgm-limits`.
- **A module that reads its settings through the trait does so in a query of its own,** before or next to its own statements.
- **`zgm-iam` waits for this module** before the function that schedules the deletion of a user.
