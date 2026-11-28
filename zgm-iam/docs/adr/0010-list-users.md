# ADR-0010: `list_users` gives the users of an active tenant in pages of at most 100, the newest or the oldest first, of every state or of one

> **Status:** accepted
> **Date:** 2026-11-22
> **Applies to:** `zgm-iam` (`src/tenant_scope.rs`, `src/repository.rs`, `src/order.rs`, `src/user.rs`, `src/state.rs`, `migrations/20261115183923_create_tables.sql`)

## Context

- **`user_id(id).get()` gives one user by id** ([ADR-0009](0009-get-user.md)). An application also needs the users of a tenant, and a tenant can have tens of thousands.
- **`get_blocked` in `zgm-limits` gives pages of at most 100 with a `next_cursor`,** the newest first.
- **A user has no name, and the name of a tenant is encrypted** ([ADR-0002](0002-the-tenants-table.md), [ADR-0007](0007-the-users-table-and-the-states.md)), so the database has no name to sort by.
- **Nothing works in a tenant that is not active,** reading included ([ADR-0009](0009-get-user.md)), but for `set_state` and `delete` of the tenant itself, the way out of those states ([ADR-0015](0015-the-state-of-a-tenant.md)).

## Options considered

**All users at once, or in pages:**

| Option | Pros | Cons |
|---|---|---|
| **A. Pages of at most 100, with `next_cursor`, as in `zgm-limits`** | a tenant with 50 000 users never comes into memory at once | an application that wants all users calls in a loop |
| **B. All at once, `Vec<User>`** | the simplest call | the whole tenant in memory; going over to pages later changes the call in every application |

**The order:**

| Option | Pros | Cons |
|---|---|---|
| **A. One order, the newest first, as in `zgm-limits`** | nothing to choose | the application cannot start from the oldest |
| **B. `Order { NewestFirst, OldestFirst }`, by `id`** | both ends of the list, from one index | only the order of creation |
| **C. A column to sort by, as with `ORDER BY` in SQL: `created_at`, `updated_at`** | more orders | an index for each column; a user whose `updated_at` changes moves on the list while the application is going through its pages; there is no name to sort by |

**A list without `.order(...)`:**

| Option | Pros | Cons |
|---|---|---|
| **A. The newest first, as in `zgm-limits`** | the shortest call gives the usual order | the order is not written in the call |
| **B. `.order(...)` required** | the order is always written in the call | every call is longer |

**A filter by the state of the user:**

| Option | Pros | Cons |
|---|---|---|
| **A. None** | a simpler function | to find the 3 suspended users among 50 000, the application reads 500 pages |
| **B. `.state(State::...)`, optional** | the application gets those 3 at once | one more index on the table of users |

## Decision

**Pages: A. The order: B. Without `.order(...)`: A. The filter: B** (decided by the maintainer on 2026-11-22: the pages, "ok A"; the order, first "then add an order by filter", then "it should be order by, by created at, by name etc., and not newest, oldest first", and, after C was shown with its costs, "then maybe leave that `order(Order::OldestFirst)`"; the default, "ok"; the filter, "add a state filter too"; the whole design, "well, ok").

- **A `bon` builder ending in `.call()`,** as `get_blocked` in `zgm-limits`:

  ```rust
  let page = in_kowalski.list_users().call().await?; // the newest first, every state
  let next = in_kowalski
      .list_users()
      .order(Order::OldestFirst)
      .state(State::Suspended)
      .maybe_after(page.next_cursor)
      .call()
      .await?;
  ```

- **`UsersPage { items: Vec<User>, next_cursor: Option<Uuid> }`.** A page holds at most 100 users. `next_cursor` is the `id` of the last user of the page, to be given to `after` with the same order and state; it is `None` on the last page. The module reads 101 rows to know whether there is a next page, as `zgm-limits` does.
- **`Order { NewestFirst, OldestFirst }`, with `NewestFirst` the default,** sorts by `id`. Every user is listed in whatever state it is in, unless `state` asks for one.
- **A tenant without users, or without users in that state, gives an empty page.** A tenant that does not exist is `IamError::NoSuchTenant`, one that is not active `IamError::TenantNotActive(state)`, and a word that is no state `IamError::UnknownState`, as in `get_user`.
- **One query, with no transaction and no lock,** in four constant versions: two orders, without a state and with one. The newest first with a state:

  ```sql
  SELECT t.state AS tenant_state, u.id, u.state, u.deletes_at, u.created_at, u.updated_at
  FROM zgm_iam.tenants t
  LEFT JOIN LATERAL (
      SELECT id, state, deletes_at, created_at, updated_at FROM zgm_iam.users
      WHERE tenant_id = $1 AND state = $3 AND (tenant_id, state, id) < ($1, $3, $2)
      ORDER BY id DESC LIMIT $4) u ON true
  WHERE t.id = $1
  ORDER BY u.id DESC
  ```

  `$1` is the tenant, `$2` the id the page starts after, `$3` the state and `$4` 101. No row is no tenant; one row with no `u.id` is a tenant without such users.
- **The id to start after is always given:** on the first page, the largest UUID for the newest first and the nil UUID for the oldest first. A UUID v7 is never either, so no user is left out.
- **The table of users has two indexes for the list:** `(tenant_id, id)`, which is also the index of the foreign key to the tenants, and `(tenant_id, state, id)`.

## Why

- **Pages:** the memory of the application does not grow with the tenant, and the call will not have to change when tenants grow.
- **`id` and not `created_at`:** a UUID v7 starts with the moment it was made, so it is the order of creation, and no two users share one, so the place where a page ended is one value and a list over several pages never skips or repeats a user. Sorting by `created_at` would need the `id` too, and its own index.
- **No order by name:** there is none the database can read.
- **The id compared together with the tenant, and the state, as one row,** and given on the first page too. Measured on 2026-11-22 on 500 000 users in 11 tenants, one with 3 users and ten with 50 000, written one of each in turn, with the plan PostgreSQL makes once for any values: in this shape every page, of either order, with a state or without, started at its place in an index of the list and read only its rows, in 0.01–0.6 ms. Written as `id < $2`, or with no id on the first page, the same query for the tenant with 3 users went through the primary key instead, all 500 000 rows, in 23–24 ms, a time that grows with the table.
- **The index of the state:** without it, the page of the 3 suspended users of a tenant with 50 000 read the 50 000 users of the tenant and dropped 49 997 of them, in 6.7 ms; with it, 0.02–0.03 ms.
- **No lock:** as in `user_id(id).get()`, the query writes nothing and sees the tenant and its users as they were at one moment.
- **`ORDER BY` outside the join too:** SQL gives no order to the rows of a join without it. The sort of at most 101 rows is in the times above.

## Consequences

- **An application that wants every user calls in a loop,** a page at a time.
- **The list is not a snapshot.** A user created while the application goes through the pages appears on a later page when the oldest come first, and not at all when the newest come first. A user deleted meanwhile is gone, and one whose state changes may leave or join a list of one state. A `next_cursor` stays valid when its user is deleted, since it only marks a place.
- **A `next_cursor` given with another order or state** gives a page from that place in the other list, and no error.
- **Two users created at almost the same moment on two connections** may be listed in an order other than that of their `created_at`: the `id` is made a moment after the statement starts, and `created_at` is the start of the statement.
- **Every write of a user updates three indexes:** the primary key and the two of the list.
