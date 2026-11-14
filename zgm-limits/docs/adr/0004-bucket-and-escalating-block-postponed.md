# ADR-0004: Counting a bucket and a rule with an escalating block are postponed; the window comes first

> **Status:** accepted
> **Date:** 2026-11-10
> **Applies to:** `zgm-limits` (`Rule`)

## Context

- **`Rule` is an enum with a variant for each way of counting** ([ADR-0002](0002-window-from-the-first-attempt-and-rule-as-an-enum.md)). For now only the window is counted.
- **We already know two ways that will be needed:**
  - **a bucket with tokens,** for a client that sends requests all the time and cannot stop until the end of a window;
  - **an escalating block,** as when unlocking a phone: a few wrong attempts are free, and then every next one gives a longer block, e.g. 1 minute, 5 minutes, 15 minutes, an hour. The count goes back to zero on a successful attempt, not with the passing of time.

## Options considered

| Option | Pros | Cons |
|---|---|---|
| **A. All three ways at once** | complete from the start | three ways of counting at once, before the answer, the state, the database and the tests exist for even one |
| **B. The window first, the rest later** | the whole skeleton is built on the simplest way; the next ones arrive as variants | until then a limit with a bucket does not work, and an escalating block cannot be written |

## Decision

**Option B** (decided by the maintainer on 2026-11-10).

- **Bucket:** the variant `Rule::Bucket { capacity, refill_every }` already exists and can be written in a limit, but its counting answers "not implemented".
- **Escalating block:** there is no variant yet. Working shape: `Rule::Escalating { free_attempts, blocks }`, that is, the number of free attempts and the list of successive block times.

## Why

- **The window is the simplest,** so it is the easiest one on which to understand and arrange the rest of the module.
- **Postponing costs nothing:** a new way is a new variant and a file with its counting, with no changes to the window.

## Consequences

- **An escalating block remembers something different from a window:** the number of failures in a row and the end of the block. Its state, and how the caller reports a successful attempt, will be settled when we come back to it.
- **For now the product should not use `Rule::Bucket`** outside the example.

## When to revisit

- The window works from the definition to the write in the database.
- Or earlier, when the product needs one of these ways.
