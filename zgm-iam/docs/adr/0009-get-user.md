# ADR-0009: `user_id(id).get()` gives back a user of an active tenant in any state of the user, or `None`; a tenant that is not active refuses reading too

> **Status:** accepted
> **Date:** 2026-11-22
> **Applies to:** `zgm-iam` (`src/tenant_scope.rs`, `src/repository.rs`)

## Context

- **The functions of one tenant hang from `iam.tenant_id(id)`,** and `create_user` refuses a tenant that does not exist or is not active ([ADR-0008](0008-the-functions-of-one-tenant-and-create-user.md)).
- **A tenant and a user each have their own state:** `active`, `suspended` or `pending_deletion` ([ADR-0007](0007-the-users-table-and-the-states.md)).
- **An id the application asks about may be of no user, or of a user of another tenant.** Both are UUIDs, and nothing in the id says which tenant it belongs to.

## Options considered

**A user the tenant does not have:**

| Option | Pros | Cons |
|---|---|---|
| **A. `Ok(None)`** | an error means that something broke; the application decides whether a missing user is an error for it, as with `HashMap::get` | the application has to handle `None` |
| **B. An error, e.g. `NoSuchUser`** | `?` passes it on | a normal case looks like a failure |

**A tenant that is suspended or waits for deletion:**

| Option | Pros | Cons |
|---|---|---|
| **A. Reading always works;** only the functions that change something refuse | the application can show who is in a blocked company | "nothing works in it" does not hold for reading |
| **B. Reading refuses too, with `TenantNotActive`, as `create_user` does** | "nothing works in it" holds for every function | the application does not see the users of a blocked company until it is unblocked |

## Decision

**A user the tenant does not have: A. A tenant that is not active: B** (decided by the maintainer: the first on 2026-11-21, "then A"; the second on 2026-11-21, "yes, refuse with an error too"; the whole design on 2026-11-22, "ok").

- **`in_tenant.user_id(user_id).get() -> Result<Option<User>, IamError>`;** `user_id` reads nothing ([ADR-0013](0013-the-functions-of-one-user-and-set-state.md)).
- **A user of the tenant is given back in whatever state the user is in.** In an active tenant, a suspended user is `Some(user)` with `State::Suspended`, and a user waiting for deletion has its `deletes_at`. The state of the tenant decides whether reading works; the state of the user is only shown.
- **No such user, and a user of another tenant, are `Ok(None)`.**
- **A tenant that does not exist is `IamError::NoSuchTenant`, and one that is not active `IamError::TenantNotActive(state)`,** as in `create_user`, even for an id of no user. A word that is no state, of the tenant or of the user, is `IamError::UnknownState`.
- **One query, with no transaction and no lock:**

  ```sql
  SELECT t.state AS tenant_state, u.id, u.state, u.deletes_at, u.created_at, u.updated_at
  FROM zgm_iam.tenants t
  LEFT JOIN zgm_iam.users u ON u.tenant_id = t.id AND u.id = $2
  WHERE t.id = $1
  ```

  `$1` is the tenant and `$2` the user. No row is no tenant; a row with no `u.id` is a tenant without such a user.
- **The check of the tenant is one function, shared with `create_user`,** so the two cannot drift apart.

## Why

- **`None` for a user of another tenant,** and not a different answer than for an id of no user: the application learns nothing about the other tenants, not even that the id exists there.
- **Reading refuses in a tenant that is not active,** because the maintainer wants nothing to work in it (option B).
- **`NoSuchTenant` and not `None` for a missing tenant:** a missing tenant is usually a mistake of the application, and `create_user` reports it the same way.
- **No lock:** the query writes nothing, and one statement sees the tenant and the user as they were at one moment. A suspension under way at the same time is either seen or not, and either answer was true a moment ago.
- **One query and not two:** one round trip, and no moment between the check of the tenant and the read of the user.

## Consequences

- **The application does not see the users of a suspended tenant, or of one waiting for deletion,** until the tenant is active again.
- **A suspended user is visible.** Whether such a user may do anything is decided later, by the permissions and by the login module.
- **`get` takes a connection from the pool for one query,** like `create_tenant`.
