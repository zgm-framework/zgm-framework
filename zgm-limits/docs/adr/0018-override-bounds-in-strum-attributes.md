# ADR-0018: The product writes the bounds of what a tenant may change a rule to as four `strum` attributes above the limit; all of them or none

> **Status:** accepted
> **Date:** 2026-11-11
> **Applies to:** `zgm-limits` (`src/rule.rs`: `Bounds`, `Limit`; `Limits::builder`, `override_rule`, `repository::hit_window`)

## Context

- **A tenant can override the rule of a limit** ([ADR-0017](0017-tenant-rules-in-the-database.md)). Without bounds, an admin could enter four billion attempts and in practice turn off the password limit.
- **The bounds are different for every limit,** and the product knows them, not the module.
- **The maintainer proposed writing them as `strum` attributes above the enum variant** (2026-11-11).

## Options considered

**Where the product writes the bounds:**

| Option | Pros | Cons |
|---|---|---|
| **A. `strum` attributes above the variant** | the bounds sit right above the limit; always the same four numbers; no second `match` | `strum` does not know the allowed names, so a typo compiles |
| **B. A second method of the `Limit` trait, next to `rule()`** | a typo in a field name does not compile | a second `match` over all limits |

**What a limit without bounds means:**

| Option | Pros | Cons |
|---|---|---|
| **A. The tenant cannot change it** | permission has to be written explicitly; a forgotten limit stays safe | every limit that should be changeable needs four numbers |
| **B. The tenant can change it freely** | less to write | a forgotten limit can be turned off by entering billions |

## Decision

**Notation: A. No bounds: A** (decided by the maintainer on 2026-11-11).

```rust
#[strum(props(min_at_most = 1, max_at_most = 20, min_per_seconds = 60, max_per_seconds = 86400))]
WrongPassword,
```

- **Four attributes:** `min_at_most`, `max_at_most`, `min_per_seconds`, `max_per_seconds`. The `Limit` trait requires `strum::EnumProperty`.
- **All four or none.** A limit with only some of the attributes causes a panic when the module is built, with the name of the limit.
- **At build, the module also checks** that every bound is a number from 1 to `u32::MAX`, that the minimum is not greater than the maximum, and that the default rule itself fits within its bounds. Each of these is a panic at startup.
- **`override_rule` with a rule outside the bounds returns `LimitsError::RuleOutOfBounds`** with the name of the number, its value and the allowed range. For a limit without bounds it returns `LimitsError::RuleCannotBeChanged`.
- **Counting keeps to the bounds too.** The attempt's query clamps the numbers from the tenant's row to the bounds from code; for a limit without bounds it always counts by the default rule. `get_rules` shows the same.
- **`get_defaults` and `get_rules` return the bounds of every limit,** so that an admin panel can show the allowed range.

## Why

- **Every typo ends up on the safe side:**
  - in one name: the module sees three attributes and refuses to build;
  - in all four: the module sees zero, so the limit cannot be changed;
  - an extra, unknown attribute next to four correct ones is ignored.
- **That is why no bounds has to mean "cannot be changed".** With "can be changed freely", typos in all four names would remove the bound silently.
- **The bounds are an addition, not the core of the rule,** so here the attributes do not take away the compiler checking that the `match` in `rule()` gives ([ADR-0016](0016-rules-checked-at-build.md)).
- **Clamping at counting time** makes narrowing or removing the bounds in code take effect at once, also for rows saved earlier. Without it, an old, wide tenant rule would stay in force despite the tighter bounds.
- **The module does not know any standards.** The product decides what bounds a limit has; the code only checks that they are consistent.

## Consequences

- **The product's enum of limits also has `strum::EnumProperty` in its `derive`,** even when no limit has bounds.
- **A mistake in the bounds is a panic at startup, not a `Result`,** just like `Count::new(0)`.
- **A tenant's row outside today's bounds stays in the database unchanged;** only what is counted with and what `get_rules` shows is clamped.
- **`restore_default` always works,** also for a limit that has lost its bounds.
- **Today the bounds apply to the window:** `at_most` and `per`. Other ways of counting will get their own when they are built.
