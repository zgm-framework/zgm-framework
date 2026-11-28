# ADR-0005: The entry point of the module is the `Iam` struct, built with a `bon` builder; `create_tenant` refuses an empty name and gives back the whole tenant

> **Status:** accepted
> **Date:** 2026-11-20
> **Applies to:** `zgm-iam` (`src/iam.rs`, `src/tenant.rs`, `src/error.rs`, `src/repository.rs`, `Cargo.toml`)

## Context

- **Every function that writes or shows a tenant needs the pool of connections and the cipher** ([ADR-0004](0004-the-cipher-from-the-application.md)). Users, roles and permissions come later, and the audit of events with them.
- **`zgm-limits` has one struct, `Limits`, built with a `bon` builder,** and every function of the module is its method.
- **A tenant is created by the module, and the database gives it its id** ([ADR-0002](0002-the-tenants-table.md)). The application has no number of its own to give.

## Options considered

**The entry point:**

| Option | Pros | Cons |
|---|---|---|
| **A. `Iam` with a `bon` builder** | the entry point keeps its shape when settings are added; the same as in `zgm-limits` | the `bon` crate; with two settings the builder gives little yet |
| **B. `Iam::new(pool, cipher)`** | no crate | every new setting changes the call in every application |
| **C. Functions only, no struct** | the least code | the pool and the cipher are passed with every call |

**What `create_tenant` gives back:**

| Option | Pros | Cons |
|---|---|---|
| **A. The whole tenant: the id, the name, the state, the two dates** | the application can show the new tenant without asking again; the same struct the function that shows a tenant will give | none found |
| **B. The id alone** | less to build | the application asks again for what the database has just set |

**An empty name:**

| Option | Pros | Cons |
|---|---|---|
| **A. An error** | "every tenant has a name" holds | none found |
| **B. Written like any name** | nothing to check | empty bytes encrypted are bytes too, so `NOT NULL` passes them, and the tenant is in practice without a name |

## Decision

**The entry point: A. What it gives back: A. An empty name: A** (decided by the maintainer: the entry point and what it gives back on 2026-11-15 and 2026-11-20, the empty name on 2026-11-16: "come on, an error, A").

- **`Iam::builder().pool(pool).cipher(cipher).settings(settings).run_jobs_in_background(on_error).build().await?`.** All four are required; `build()` gives the settings the defaults of the module and waits for them, fails only when the settings do ([ADR-0011](0011-tenant-settings-from-the-application.md)), and then starts the loop that deletes the users past their date ([ADR-0014](0014-the-loop-that-deletes-users.md)). `Iam` is `Clone`, and a clone shares the pool, the cipher and the settings.
- **`Iam` has only the functions above the tenants,** such as `create_tenant`. The functions of one tenant hang from `iam.tenant_id(id)` ([ADR-0008](0008-the-functions-of-one-tenant-and-create-user.md)).
- **`iam.create_tenant(name: &str) -> Result<Tenant, IamError>`** creates a tenant and gives back `Tenant { id, name, state, deletes_at, created_at, updated_at }`. The id, the state, `deletes_at` and the dates come from the database ([ADR-0007](0007-the-users-table-and-the-states.md)); `name` is the one given.
- **An empty name is the error `IamError::EmptyName`.** The cipher is not called, and nothing is written.
- **Nothing else about the name is checked:** not its length, not its characters, not spaces at its ends. That is the application's business.
- **All SQL is in `src/repository.rs`,** in plain functions that take a connection, as in `zgm-limits`. `create_tenant` takes a connection from the pool for one `INSERT ... RETURNING`.
- **`IamError` gets `Display`, `Error` and `From` from `thiserror`.** A database error is `IamError::Database`.
- **No events are reported yet.** The audit comes later, for all the functions there are by then (decided by the maintainer on 2026-11-20); the write of a tenant then goes into one transaction with its event, and the call of `create_tenant` stays the same.

## Why

- **More settings will come,** the audit first, and a builder adds them without changing the calls that exist.
- **`bon` 3.10.2 is the version `zgm-limits` uses.** Checked on 2026-11-20: it is not yanked, OSV knows no vulnerabilities in it, and its repository is not archived. The newest version is 3.10.3; the repository keeps one version of each crate.
- **The empty name is checked before the cipher,** so that an application whose key store is down still gets the error about the name, and the store is not asked for nothing.

## Consequences

- **Two tenants may have the same name.** The module cannot check it: the table has only ciphertext, and a cipher may make different bytes of the same name each time.
- **A name of spaces alone is a name.**
- **Until the audit comes, nothing but the returned `Tenant` says that a tenant was created.**
