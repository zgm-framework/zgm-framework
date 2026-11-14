# ADR-0012: A row remembers in the `blocked_at` column since when the key has been blocked; `get_blocked` lists blocked keys in pages, and `unblock` deletes the counter

> **Status:** accepted
> **Date:** 2026-11-10
> **Applies to:** `zgm-limits` (migration, `repository::hit_window`, `repository::list_blocked`, `repository::delete_blocked`, `Limits::get_blocked`, `Limits::unblock`, `src/blocked.rs`)

## Context

- **An operator wants to see who will not pass a limit right now,** and also those whose block is already over.
- **The row alone does not show whether a key is blocked.** The row holds the number of attempts, and only the code (`Rule`) knows how many are allowed.
- **There may be millions of counters, and only a handful of them are blocked.**
- **An operator also wants to unblock a chosen key,** and the list may be longer than one page.

## Options considered

**How we know who is blocked:**

| Option | Pros | Cons |
|---|---|---|
| **A. Computed when listing:** the method gives the query the rules of the limits, and the database compares the number of attempts with `at_most` | no new column | reads all live rows; the result changes when somebody changes a rule in the code |
| **B. A `blocked_at` column,** written by the attempt that fills the window | the list is a read on a small index; the table knows no rule; we know since when the block has lasted | one more line in the attempt query; one more column and one more index |
| **C. A `blocked_at` field in `state`** | no new column | the list must know a field inside `state`, and every way of counting must name it the same; the date as text |

**The word:** "cut off" (`cut_off`) or "blocked" (`blocked`).

**How to point at a row, to unblock it and to start the next page:**

| Option | Pros | Cons |
|---|---|---|
| **A. By the four values of the key** | nothing in the table | every "unblock" button carries four fields, including the bytes of the hash; the cursor has five values |
| **B. By an `id` column with an index on blocked rows only** | `unblock(id)`; the cursor is `blocked_at` and `id` | 8 bytes in every row |
| **C. By an `id` column with an index on the whole table** | as B | a second index written with every new counter |

## Decision

**How we know: B. The word: "blocked". Pointing at a row: B** (decided by the maintainer on 2026-11-10).

- **A blocked key is a key of which no attempt will pass right now,** whatever the reason. Today the only reason is a full window.
- **`blocked_at timestamptz`, null when the key is not blocked.** The attempt that fills the window writes its moment there. A new window starts without a block.
- **A refusal still writes nothing.** The column is set by the last attempt that passed.
- **The end of a window's block is `expires_at`.** There is no separate column for the end.
- **`id bigint GENERATED ALWAYS AS IDENTITY`.** The primary key stays the same ([ADR-0005](0005-counters-table-and-key.md)).
- **Two indexes, both on blocked rows only** (`WHERE blocked_at IS NOT NULL`): `counters_blocked_idx (tenant_id, blocked_at, id)` for the list of one tenant and `counters_blocked_id_idx (id)` for unblocking.
- **`limits.get_blocked().tenant_id(...).call()` returns one page of the blocked keys of one tenant,** at most 100, from the newest block. The tenant is required: without it the code does not compile, and there is no list of all tenants at once. A page has `items` and `next_cursor`; no `next_cursor` means the end of the list.
- **Filters are given by name, through the method's `bon` builder:** `.limit_name(AppLimit::Export)`, `.whole_tenant(true)`, `.include_expired(true)` and `.after(cursor)` or `.maybe_after(page.next_cursor)`. There is no filter struct (decided by the maintainer on 2026-11-10).
- **The cursor is `blocked_at` and `id` of the last row.** `id` decides between rows blocked at the same moment.
- **The query writes a missing cursor as "from infinity",** not as "the cursor is null or the row is after it". With the "or", the database did not use the cursor to jump along the index, and a far page read all the rows before it: with 200,000 blocked keys, 150,000 rows instead of 101.
- **An item of the list is `Blocked`:** `id`, the limit name, the tenant, the account, the hash, `whole_tenant`, `blocked_at`, `blocked_until` and `expired`.
- **`whole_tenant` says that the counter is of the whole tenant,** that is, the key has neither an account nor a hidden part, so the block stops everybody in the tenant. The filter `.whole_tenant(true)` shows only such blocks, and `.whole_tenant(false)` only the others (decided by the maintainer on 2026-11-10).
- **`limits.unblock(tenant_id, id)` deletes the row of the counter,** so the key starts from zero. It returns `true` when this tenant had such a blocked key. An `id` of another tenant deletes nothing and gives `false`, and so does a row that is not blocked.
- **The tenant is in every query of the module** (decided by the maintainer on 2026-11-11: "this whole module works per tenant, so the tenant must be present in every query to the database"). The only exception is the cleanup ([ADR-0015](0015-cleanup-in-a-job-loop.md)).

## Why

- **The rule from [ADR-0005](0005-counters-table-and-key.md):** `state` is read only by the query of the given way of counting, and whatever is needed by somebody who does not know the way of counting is a column, like `expires_at`.
- **One word for "will not pass"** is enough also when the escalating block is added ([ADR-0004](0004-bucket-and-escalating-block-postponed.md)).
- **The index of the list starts with `tenant_id`,** so the list reads only the blocked rows of its tenant, not the blocks of all tenants.
- **Filters by name instead of a struct,** because the caller gives only what it wants, without `Some`, `None` and default values, and a new filter does not change the existing calls.
- **Clearing `blocked_at` alone does not unblock,** because the counter still shows a full window. Deleting the row works the same for every way of counting.
- **An ordinary attempt writes neither of the two indexes,** because a row that blocks nobody is in neither.
- **There is a test for another tenant:** `unblock` with the `id` of a key of another tenant returns `false` and leaves the block, and the list of a tenant does not show the keys of others.
- **The tests detect both bugs that I checked on purpose:** a cursor without `id` loses rows with the same moment of the block, and `unblock` without the condition deletes a counter that is not blocked.

## Consequences

- **The `id` numbers are sequential and shared by all tenants,** so they can be guessed. That is why `unblock` checks the tenant in the query itself and does not leave it to the panel. A code review reproduced lifting another tenant's block by giving consecutive numbers when this condition was missing.
- **Every attempt that passes uses up one value of the `id` sequence,** also when it only changes an existing row. `bigint` is enough for that with a large margin.
- **A filter by limit goes through the blocked rows of the tenant one by one.** There are few of them, so there is no separate index.
- **The list agrees with the counting also after a rule changes,** because a window remembers the threshold it opened with, and is counted and blocked by it ([ADR-0017](0017-tenant-rules-in-the-database.md)).
- **Expired blocks are visible until the row is cleaned up,** that is, for an hour after their end ([ADR-0015](0015-cleanup-in-a-job-loop.md)).
- **The list cannot name keys with a hidden part:** it shows the hash, not the e-mail.
