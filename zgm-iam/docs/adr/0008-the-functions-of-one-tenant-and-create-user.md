# ADR-0008: The functions of one tenant hang from `Iam::tenant_id`; `create_user` writes a user only under an active tenant

> **Status:** accepted
> **Date:** 2026-11-21
> **Applies to:** `zgm-iam` (`src/iam.rs`, `src/tenant_scope.rs`, `src/user.rs`, `src/repository.rs`, `src/error.rs`)

## Context

- **`zgm-limits` takes the tenant in every call,** e.g. `unblock(tenant_id, id)`.
- **A function of one tenant takes the UUID of the tenant and often the UUID of a row in it,** and both are `Uuid`.
- **Some applications have a single tenant.**
- **Nothing can be done in a tenant that is suspended or waits for deletion** ([ADR-0007](0007-the-users-table-and-the-states.md)).

## Options considered

**How the tenant is given:**

| Option | Pros | Cons |
|---|---|---|
| **A. In every call, as in `zgm-limits`:** `iam.suspend_user(tenant_id, user_id)` | the same pattern as in `zgm-limits` | `iam.suspend_user(user_id, tenant_id)` compiles too |
| **B. Once:** `iam.tenant_id(tenant_id)`, and the functions of the tenant on what it gives | the tenant cannot be left out or swapped with another UUID; an application with one tenant gives it once, when it starts | one more type; a pattern different from `zgm-limits` |

**A tenant that does not exist, or is not active:**

| Option | Pros | Cons |
|---|---|---|
| **A. One error** | one variant | the application cannot tell a mistake of its own from a blocked company |
| **B. Two errors, `NoSuchTenant` and `TenantNotActive` with the state** | the application can tell them apart | two variants |

**A tenant suspended at the same moment as its user is created:**

| Option | Pros | Cons |
|---|---|---|
| **A. Read the state, then write the user** | no lock | the user is written past a suspension under way |
| **B. Lock the row of the tenant with `FOR SHARE` in the transaction that writes the user** | the creation waits for the suspension and then refuses | a change of the tenant waits for a creation under way |

## Decision

**The tenant: B. The errors: B. The same moment: B** (decided by the maintainer on 2026-11-21).

- **`iam.tenant_id(tenant_id: Uuid) -> TenantScope`,** with no query to the database. The maintainer: "it should be `iam.tenant_id(123).create_user`, `.suspend_user` etc., that will be better, and we set `tenant_id` first, and nothing else can be set until the tenant id is given first".
- **`TenantScope` is `Clone` and holds its own copy of `Iam`,** so an application with one tenant makes it once and passes it down. The maintainer: "it would be good if the IAM instance with this tenant were created already in `main.rs` and then only passed down".
- **`create_tenant` stays on `Iam`,** since a new tenant has no id to give first (the maintainer: "yes, that seems logical now").
- **`in_tenant.create_user() -> Result<User, IamError>`** gives back `User { id, tenant_id, state, deletes_at, created_at, updated_at }`. The id, the state, `deletes_at` and the dates come from the database; a new user is `State::Active` with no `deletes_at`.
- **One transaction:** `SELECT state FROM zgm_iam.tenants WHERE id = $1 FOR SHARE`; no row is `IamError::NoSuchTenant`, a state other than `active` is `IamError::TenantNotActive(state)`, and only then `INSERT INTO zgm_iam.users (tenant_id) VALUES ($1)`. On an error nothing is written; a word that is no state is `IamError::UnknownState`.
- **A plain `INSERT ... VALUES` is enough here,** unlike in a function that links two rows of a tenant ([ADR-0007](0007-the-users-table-and-the-states.md)): the one row it links to, the tenant, is checked and locked in the same transaction, and the foreign key backs it.

## Why

- **The tenant once:** a function cannot be called without its tenant, and a UUID of a user cannot take the place of the tenant's.
- **Two errors:** the first is usually a mistake of the application, the second a normal case to tell a person about.
- **`FOR SHARE`, checked on PostgreSQL 18 on 2026-11-21** in a throwaway schema with a foreign key from users to tenants: while another session held an uncommitted suspension of the tenant for 3 seconds, the `INSERT ... SELECT` with `FOR SHARE` waited for it and wrote no user; without `FOR SHARE` it did not wait and wrote one. The foreign key alone does not stop it.
- **The lock first, then the write,** and not a write that writes nothing followed by a query for the reason: between those two the state can change again, and the reason read would not be the one the refusal was made on.

## Consequences

- **A change of a tenant waits for a `create_user` under way in it,** for one short transaction.
- **`iam.tenant_id(id)` with an id of no tenant is no error** until a function is called.
- **The pattern differs from `zgm-limits`,** where the tenant is a parameter of every call.
- **The message of `TenantNotActive` names the state as the Rust type does,** e.g. "the tenant is not active: Suspended".
- **The test of the same moment** holds a suspension open, starts `create_user` and checks in `pg_stat_activity` that a session of its own database waits on a lock, before it commits the suspension.
