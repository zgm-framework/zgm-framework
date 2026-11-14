# ADR-0006: An attempt is counted by one SQL statement, which reads the row first and writes nothing on a refusal

> **Status:** accepted
> **Date:** 2026-11-10
> **Applies to:** `zgm-limits` (counting the window, table `zgm_limits.counters`)

## Context

- **Every attempt must check the counter and, if there is room, increase it.** Two attempts at once must not both count on a copy of the same row.
- **The module must withstand a flood of attempts.** In a flood almost all attempts end in a refusal, so it is the refusal that must be cheap.
- **The usual way is `BEGIN`, `SELECT … FOR UPDATE`, a write and `COMMIT` on every attempt.** Then every attempt, also a refused one, locks the row and waits in a queue behind the others.

## Options considered

| Option | Pros | Cons |
|---|---|---|
| **A. A read, counting in Rust, a write conditional on `version`** | counting is a pure function that is tested without a database | two queries for an attempt that passes; retrying after a lost write; an extra `version` column |
| **B. One statement `INSERT … ON CONFLICT DO UPDATE … WHERE`** | one query | `ON CONFLICT DO UPDATE` locks the row also when `WHERE` is false, so refusals stand in a queue: about 2,400–2,900 attempts per second with 8 connections and 1,500 with 32 |
| **C. One statement that reads the row first and skips the write when the window is full** | one query; a refusal locks nothing and writes nothing: 48,000 attempts per second with 8 connections and 83,000 with 32 | the window logic is in SQL, so its tests need a database; every way of counting has its own query |

Measurements: PostgreSQL 18 in Docker on a laptop, a flood of refusals on one key. A plain read of the row gave 64,000 and 167,000 per second in the same runs. What matters is the ratio, not the numbers themselves.

## Decision

**C** (decided by the maintainer on 2026-11-10: "remove version and do it in SQL").

- **One attempt is one query to the database,** with no transaction opened by the code.
- **The window query has three parts:**
  1. it reads the tenant's rule, if the tenant has one, and the row of the key, without a lock;
  2. unless the window is both open and full, it inserts the row or changes it: in a window that is still open it increases the number of attempts, and after the end of the window it opens a new one with the number 1;
  3. it returns "passed" or "refused until the end of the window".
- **The condition "there is room" also stands at the write itself,** so when several attempts at once see free room, the database lets through only as many as there is room for.
- **A refusal changes nothing,** including the end of the window.
- **The code gives the moment of the attempt and the default rule (`at_most`, `per`) as parameters.** The query does not read the database clock. When the tenant has its own rule for the limit, the query takes it from the `tenant_rules` table in the same statement ([ADR-0017](0017-tenant-rules-in-the-database.md)).
- **The `state` of a window is the number of attempts and the threshold the window opened with,** and the end of the window is `expires_at`.

## Why

- **Performance in a flood matters most here,** and only C brings a refusal close to the cost of a plain read.
- **There is no `version` column.** The database guards correctness inside one statement.
- **Checked on a database:** 7 attempts one after another with a limit of 5 gave 5 passed and 2 refusals with the same end of the window; 40 attempts at once on a new key gave exactly 5 passed.

## Consequences

- **There is no function that counts the window in Rust.** The Rust code assembles the parameters and reads the answer.
- **The counting tests run on a real database.**
- **The bucket and the escalating block will get their own queries** ([ADR-0004](0004-bucket-and-escalating-block-postponed.md)).
- **In rare cases the query may give no answer.** This happens when what part 1 read stopped being true before part 2 reached the write: another attempt opened or filled the window, or `reset` deleted the row and attempts filled a new window. `Attempts::hit` then repeats the query, which reads the row as it is now.
- **There are at most ten repeats.** When none gives an answer, `hit` returns the error `LimitsError::NoAnswer` instead of spinning without end. A code review did not find a state in which this happens; the bound is there so that the loop does not depend on that assumption.
- **A refusal comes only from a window that part 1 read as full.** Earlier, after a `reset` and a refill between the read and the write, the refusal gave the end of a window that no longer existed; a code review reproduced this, and after the fix the same interleaving gives no answer and the correct end on the repeat.
- **The query lives in `repository.rs`,** in the function `hit_window` ([ADR-0007](0007-database-through-sqlx.md)).
- **The query searches by a list of hashes** ([ADR-0008](0008-changing-the-secret-key.md)).

## When to revisit

- A way of counting turns out to be too hard to write and test in SQL.
