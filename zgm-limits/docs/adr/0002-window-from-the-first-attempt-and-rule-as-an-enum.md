# ADR-0002: A limit is counted with a window from the first attempt, and `Rule` is an enum with a variant for each way of counting

> **Status:** accepted
> **Date:** 2026-11-10
> **Applies to:** `zgm-limits` (`Rule`)

## Context

- **The module must know how to count attempts in a limit of "at most N per time T".**
- **A common way to count is a bucket of tokens.** The database holds one date, "when the bucket will be full again", and "5 per hour" lets up to 9 attempts through in the first hour. This is hard to explain, also to an admin in the panel.
- **Our limits are mostly wrong passwords, wrong codes, emails with a link and exports,** that is, a few attempts and then a stop. A limit on every API request is a separate subject, outside this module.

## Options considered

**How to count:**

| Option | Pros | Cons |
|---|---|---|
| **A. Bucket with tokens** | once it is used up the wait is short, for one token (12 minutes with "5 per hour"); an even pace for a client that sends requests all the time | the hardest to explain; "5 per hour" is up to 9 attempts in the first hour |
| **B. Window from the first attempt** | everyone understands it: the first attempt opens a window that lasts T, N attempts pass in it, then the count starts from zero; the database holds the start of the window and the number of attempts | once it is used up the wait lasts until the end of the window; where two windows meet, up to 2N attempts pass in a short time |
| **C. List of the times of every attempt** | exactly N in any period T | N dates per key in the database |

**Where to write down the way of counting, so that the next one can be added:**

| Option | Pros | Cons |
|---|---|---|
| **A. `Rule` as an enum, a variant per way** | every way has its own fields, so a new one can have fields other than N and T | the shared fields repeat in the variants and are read through `match` |
| **B. A `policy` field in a `Rule` struct** | `at_most` and `per` are read without `match` | every way must fit in the same fields or keep its own in a variant of `Policy` |
| **C. A trait with generic types** | a new way without changing the module | the code is much harder to read and gives the same result |

## Decision

**Counting: B. Representation: A** (decided by the maintainer on 2026-11-10).

- **`Rule` is an enum with a variant per way of counting:** `Rule::Window { at_most, per }`, a window from the first attempt; `Rule::WindowOfDistinct { at_most, per }`, the same window filled by distinct elements ([ADR-0022](0022-window-of-distinct-elements.md)); and `Rule::Bucket { capacity, refill_every }`, a bucket with tokens.
- **The fields of a rule are `Count` and `Period`,** types that do not accept zero ([ADR-0016](0016-rules-checked-at-build.md)).
- **Both windows are counted.** The `Bucket` variant can already be written in a limit, but its counting is not implemented.
- **Every way of counting has its own SQL query,** which counts the attempt in the database ([ADR-0006](0006-counting-in-one-sql-statement.md)).

## Why

- **This is how large services count failed sign-ins:** a count of attempts, a time after which the count goes back to zero, and a block. They use buckets for API traffic, where a client cannot stop for an hour.
- **A window can be explained in one sentence,** and it protects against abuse almost as well as a bucket (2N attempts where two windows meet, against 2N − 1 in the first period T).
- **A variant per way, because the ways may have different fields.** The maintainer: "then window and bucket can have different fields, for example".

## Consequences

- **Counting a bucket in the future means its SQL query** and filling in the `Rule::Bucket` arm of the `match`. The window code and the product's limits that stay with the window do not change.
- **The next way of counting is a new variant of `Rule`,** with its own fields.
- **Adding a variant requires a new version of the module,** because the product cannot add its own way of counting.
- **Every way remembers something different about a key.** In the table this lives in one `state` column of type `jsonb` ([ADR-0005](0005-counters-table-and-key.md)).

## When to revisit

- A limit appears where waiting until the end of the window gets in the way of ordinary users.
