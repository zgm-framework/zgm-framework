# ADR-0002: The module keeps the tenants; a tenant is a UUID v7, an encrypted name and two dates

> **Status:** accepted
> **Date:** 2026-11-15
> **Applies to:** `zgm-iam` (`migrations/`, the `zgm_iam.tenants` table, `Iam::create_tenant`)

## Context

- **Everything in the framework is multi-tenant.** The maintainer: "everything we build is multi-tenant, every module; all relations must be under the tenant, the tenant is the root of the whole application."
- **`zgm-limits` takes the tenant as a bare UUID** from the application and keeps no list of tenants.
- **This module keeps users under tenants** ([ADR-0007](0007-the-users-table-and-the-states.md)), and a user must belong to a tenant that exists.
- **In AWS, accounts are not created by IAM** but by a separate service, AWS Organizations: "Using Organizations, you can create accounts and allocate resources, group accounts to organize your workflows, apply policies for governance, and simplify billing by using a single payment method for all of your accounts."

## Options considered

**Who keeps the tenants:**

| Option | Pros | Cons |
|---|---|---|
| **A. The application; the module takes a bare UUID,** as `zgm-limits` does | no table and no functions here | nothing checks that a user's tenant exists; removing a tenant is a separate "delete everything of tenant X", not a cascade; every application builds the same list |
| **B. A separate module for tenants** | one job per module | it would be a single table; modules do not reference each other's tables, so the database still could not check a user's tenant |
| **C. A `tenants` table in this module** | the database checks that a user's tenant exists; one place gives out the identifiers of tenants | one more table and its functions here; an application with its own table of companies keeps the tenant's UUID next to each company |

**The name of a tenant:**

| Option | Pros | Cons |
|---|---|---|
| **A. No name** | the module needs no cipher from the application | every application needs a table of its own only to name its tenants |
| **B. A readable name** | search and sorting by name in SQL | a copy of the database names every tenant ([ADR-0001](0001-nothing-says-whose-uuid-it-is.md)) |
| **C. A keyed hash of the name** | a copy names nobody | a hash cannot be turned back into the name, so there is nothing to show |
| **D. The name encrypted by a cipher from the application** | the name can be shown; a copy names nobody | no search or sorting by name in SQL; the application must give a cipher |

**The key:**

| Option | Pros | Cons |
|---|---|---|
| **A. A `bigint` from a sequence, and a UUID next to it for the outside** | a reference takes 8 bytes | two identifiers of one row |
| **B. A UUID v4 alone** | one identifier | random, so every new row lands in a different place of the index |
| **C. A UUID v7 alone** | one identifier; starts with the time, so new rows go to the end of the index | a reference takes 16 bytes |

## Decision

**Who keeps the tenants: C. The name: D. The key: C** (decided by the maintainer on 2026-11-15).

- **`zgm_iam.tenants` has six columns:** `id uuid PRIMARY KEY DEFAULT uuidv7()`, `name bytea NOT NULL`, `state` and `deletes_at` ([ADR-0007](0007-the-users-table-and-the-states.md)), and `created_at` and `updated_at`, both `timestamptz NOT NULL DEFAULT statement_timestamp()`. A trigger sets `updated_at` on every change.
- **The database gives a tenant its id and both dates;** the Rust code creates no UUIDs, as in `zgm-limits`. The maintainer: "we have no company number. We expose a `create_tenant` method and that's it, we generate the uuidv7 ourselves."
- **`name` holds the bytes the application's cipher made of the name.** The application gives the module the methods that encrypt and decrypt, through the `Cipher` trait ([ADR-0004](0004-the-cipher-from-the-application.md)). The module holds no key and knows no version of it. The maintainer: "this module should not deal with encryption."
- **Every tenant has a name:** the column is `NOT NULL`, and `create_tenant` refuses an empty one ([ADR-0005](0005-iam-with-a-bon-builder.md)).
- **The table has no `CHECK` constraints.** Only the module's code writes it, so the database keeps what that code cannot guarantee on its own and the values the database sets itself.

## Why

- **Keeping tenants is a job every application has anyway, and users depend on tenants** (the maintainer's argument for C). The cost of C, a second list to keep in step with the application's own, is the same cost the application carries for users.
- **A separate module would add a crate and remove the one gain,** the check in the database: "there is no point in another module like this one that deals with tenants alone."
- **An AWS account carries billing and the rules of a whole organization.** A tenant here is an identifier, a name and two dates, which is why this module departs from AWS in this one place.
- **The module needs no notion of key versions.** A ciphertext is only decrypted, never compared, so the application's method can tell on its own which key the bytes were encrypted with. A hash is compared, which is why `zgm-limits` must ask for the hashes of all versions.
- **A UUID v7 behaves like a number from a sequence.** Measured on PostgreSQL 18.6 with 5 million rows, on one laptop, so the numbers are orders of magnitude:

  | | `bigint` | UUID v7 | UUID v4 |
  |---|---|---|---|
  | the 100,000 newest rows by primary key | 85–105 ms | 92–119 ms | 502–864 ms |
  | appending 100,000 rows to the full table | 89 ms | 155 ms | 1,665 ms |
  | the primary-key index | 107 MB | 150 MB | 194 MB |

## Consequences

- **`zgm-limits` still takes the tenant as a bare UUID** and checks nothing against this table, because modules do not reference each other's tables.
- **The name cannot be searched or sorted in SQL,** and showing a list of tenants means decrypting every name on it.
- **A tenant is written by `Iam::create_tenant`** ([ADR-0005](0005-iam-with-a-bon-builder.md)), so far the only function that writes this table.
- **A tenant is deleted only after a window, and its users go with it** ([ADR-0007](0007-the-users-table-and-the-states.md)). `iam.tenant_id(id).delete()` puts a tenant in `pending_deletion` with a date the window of the tenant away, 90 days unless it set otherwise ([ADR-0015](0015-the-state-of-a-tenant.md)); the loop of the module deletes it after that date, its users with it through the cascade ([ADR-0014](0014-the-loop-that-deletes-users.md)).
- **Not decided yet:** whether a name can be changed.
