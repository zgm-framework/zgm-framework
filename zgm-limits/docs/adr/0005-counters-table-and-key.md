# ADR-0005: One counters table with a primary key of four columns without empty values; the counter's state in `jsonb`; a hash only with a hidden part

> **Status:** accepted
> **Date:** 2026-11-10
> **Applies to:** `zgm-limits` (`migrations/20261111095113_create_zgm_limits.sql`, table `zgm_limits.counters`)

## Context

- **The module keeps the counter of every key in the database,** not in the application's memory, so that it works the same with several servers.
- **The key says whose counter it is.** It has up to three parts: the tenant (always), the account (e.g. the one somebody is trying to sign in to) and the hidden part (an email or an IP address, when there is no account or we count by address).
- **The module must withstand a flood of attempts.** The counter is read on every attempt, so the read must use an index.
- **A hash of the whole key,** also for an account alone, would make every attempt need the secret key, and changing that key would require rewriting and merging rows.

## Options considered

**How many tables:**

| Option | Pros | Cons |
|---|---|---|
| **A. One table for all ways of counting** | one place for counters, one cleanup, one list in the panel | the table must hold what every way remembers |
| **B. A table per way of counting** | every table has only its own columns | the cleanup and the list in the panel go over several tables |

**Where to keep what a way of counting remembers about a key:**

| Option | Pros | Cons |
|---|---|---|
| **A. A `state` column of type `jsonb`** | the table knows no way of counting; each has its own fields under its own names; a new way needs no migration | the database does not check the content; the dates inside have no type of their own; the row is larger (30 MB instead of 18 MB per 200,000 rows) |
| **B. Two columns with neutral names, a date and a number** | plain types, no dependency | the row does not show what the number means; a way that remembers more than a date and a number will not fit |
| **C. Own columns for every way, e.g. `window_started_at`** | plain types and full names | the shared table knows every way of counting; a new way is a migration and empty columns in the rows of the others |

**Parts of the key that are absent:**

| Option | Pros | Cons |
|---|---|---|
| **A. A value of its own: the all-zero UUID, empty bytes** | the key is an ordinary primary key, and the read is an ordinary equality on the index | "absent" is a conventional value that the code and the panel must know |
| **B. `NULL` and a unique index with `NULLS NOT DISTINCT`** | an empty column plainly means "absent" | a read through `IS NOT DISTINCT FROM` does not use the index: on PostgreSQL 18 it reads the whole table on every attempt (checked on 200,000 rows) |
| **C. A hash of the whole key in one column** | a short primary key | the secret key on every attempt, also for an account alone |

**What to compute the hash from:**

| Option | Pros | Cons |
|---|---|---|
| **A. Only when there is a hidden part, from the limit name, the tenant, the account and the hidden part** | a key without a hidden part does not touch cryptography; the same email gives a different hash in every limit and tenant | none |
| **B. Only when there is a hidden part, from the hidden part alone** | the simplest input | a copy of the database shows that the same email is in several tenants and limits; a row can be moved under another limit or tenant |

## Decision

**Tables: A. State: A. Absent parts: A. Hash: A** (decided by the maintainer on 2026-11-10).

- **The table `zgm_limits.counters` has one row per key in one limit,** not per attempt.
- **Primary key:** `(limit_name, tenant_id, subject_id, key_hash)`. All columns are `NOT NULL`.
- **No account is the all-zero UUID, and no hidden part is empty bytes.** Both together mean the key "the whole tenant".
- **The hash is computed only for a key with a hidden part,** from the limit name, the tenant, the account and the hidden part together.
- **The hash is computed by the `KeyedHash` from the application** ([ADR-0013](0013-keyed-hash-from-the-application.md)). The recommended implementation gives a hash that starts with a header holding the version number of the secret key, in the format of `zgm-encryption` (1 byte of format, 4 bytes of version; `zgm-encryption` ADR-0009). The table has no separate column for the version.
- **The `state` column (`jsonb`) holds what the way of counting remembers about the key.** A window writes there the number of attempts and the threshold it opened with (`{"count": 3, "at_most": 5}`; [ADR-0017](0017-tenant-rules-in-the-database.md)). The table holds nothing about any way of counting.
- **The table has no `version` column.** An attempt is one SQL statement, so there is nothing to compare between a read and a write ([ADR-0006](0006-counting-in-one-sql-statement.md)).
- **`expires_at` says from when the row means nothing** and the cleanup may delete it; for a window it is the end of the window. Every way of counting writes its own moment there, so the cleanup does not need to know the rules of any limit. The column has the index `counters_expires_at_idx`.
- **`blocked_at` says since when the key has been blocked;** empty when it is not ([ADR-0012](0012-list-of-blocked-keys.md)).
- **`created_at` and `updated_at` are set by the database,** `updated_at` by a trigger. The trigger function lives in the `zgm_limits` schema.
- **The `id` column is a short name of the row for the list of blocked keys** ([ADR-0012](0012-list-of-blocked-keys.md)). It is not the primary key, and its index covers only blocked rows.

## Why

- **A fast read matters more than a "tidy" `NULL`.** Every attempt gives the four values of the key and hits one row through the primary key index.
- **The primary key guarantees one row per key,** also when two processes insert it at once. Counting in one statement, with no lock held between statements, relies on this.
- **The ways of counting have different fields,** like the variants of `Rule` ([ADR-0002](0002-window-from-the-first-attempt-and-rule-as-an-enum.md)), so the table stores them without guessing how many dates and numbers will be enough.
- **`jsonb` does not slow counting down to a degree that could be measured.** On PostgreSQL 18, with 200,000 rows and 8 connections, the read of the counter and the conditional write were faster once with plain columns and once with `jsonb`, and the differences between runs were larger than those between the variants.
- **The tenant and the account are identifiers, not secrets,** so keeping them directly in the key reveals nothing, and a limit per account does not need the secret key.
- **A hash computed together with the limit name and the tenant** keeps what the hash of the whole key gave: the rows of the same person cannot be linked across tenants and limits, or moved.
- **The version number of the key lives in the row, not in a separate usage counter in the keys module.** The number of rows of a given version counted from the table always agrees with what is in it. A separate counter would need a write to one shared row with every new counter and could drift away from the table.
- **The version is in the header of the hash, not in a separate column,** because a hash from `zgm-encryption` already has it. A column would hold the same information a second time.
- **`id` has no index over the whole table,** because it would be a second index written with every new counter, and an attempt never looks a row up by `id`.

## Consequences

- **The schema is named `zgm_limits`.**
- **`state` is read and written by SQL,** so the window code does not need the `serde_json` crate.
- **The shape of `state` is guarded by tests,** because the database does not check it.
- **The code and the panel must know the conventional values:** the all-zero UUID and empty bytes are not an account or a hash.
- **An account with the all-zero UUID cannot be used as a key.** `limits.attempts(...).subject_id(Uuid::nil())` returns the error `LimitsError::NilSubject` instead of silently counting on the counter of the whole tenant. A code review showed that otherwise three sign-ins to unknown accounts, replaced with the default value, block the whole tenant.
- **Rows past `expires_at` are deleted by the module's loop** ([ADR-0015](0015-cleanup-in-a-job-loop.md)).
- **To do later, without a migration:** a method that says how many rows each key version still has, so that an old version can be removed when it has none left. It reads the version from the header of the hash.

## When to revisit

- The size of the table or the time spent working with `state` starts to show in measurements.
