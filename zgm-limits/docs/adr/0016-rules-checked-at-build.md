# ADR-0016: The rule of a limit is made of the types `Count` and `Period`, which accept no bad values, and the builder reads the rule of every limit at startup

> **Status:** accepted
> **Date:** 2026-11-11
> **Applies to:** `zgm-limits` (`src/rule.rs`: `Rule`, `Count`, `Period`; `Limits::builder().limits(...)`; `LimitsError::UnknownLimit`)

## Context

- **The rule of a limit is written by the product, in a `match` next to its enum** ([ADR-0002](0002-window-from-the-first-attempt-and-rule-as-an-enum.md)). The fields of a rule were a bare `u32` and `Duration`, so the module accepted any value.
- **A code review reproduced three values with which a limit silently does not work:** `at_most: 0` let one attempt through per window, a `per` of zero let everything through, and a huge `per` caused a panic at the first attempt.

## Options considered

**Where to write the rule:**

| Option | Pros | Cons |
|---|---|---|
| **A. A `match` in `rule()`** | a forgotten limit and a typo in a field name do not compile; every way of counting has its own fields | the rule is in a separate block below the enum |
| **B. `strum` attributes over the variant,** e.g. `#[strum(props(at_most = 3, per_seconds = 900))]` | the rule is right above the limit; no `impl Limit` | a forgotten limit and a typo in a name show up only at startup; another way of counting means more loose names |

**How to keep a bad value out:**

| Option | Pros | Cons |
|---|---|---|
| **A. Types that do not accept a bad value** | nothing to check afterwards; the bounds follow from the type, not from invented constants | a rule says `Count::new(3)` instead of `3`; a zero written straight into `rule()` is a panic, not an error |
| **B. Bare `u32` and `Duration`, checked by a function when the module is built** | ordinary types; a bad value is an ordinary error from `build()` | a separate validating function, an error variant and bounds invented by the module (a microsecond, a thousand years) |
| **C. No checking** | the least code | a limit that looks set does not protect |

## Decision

**Writing: A. Values: A** (decided by the maintainer on 2026-11-11: "wouldn't a type that accepts only positive numbers be better (...) or our own type, a VO with min/max validation").

```rust
Self::WrongPassword => Rule::Window {
    at_most: Count::new(3),
    per: Period::minutes(15),
},
```

- **`Count` is a number of attempts, always at least 1.** Inside it is a `NonZeroU32`.
- **`Period` is whole seconds, at least 1 and at most `u32::MAX`,** that is, about 136 years. It is built with `Period::seconds`, `minutes`, `hours` and `days`.
- **The shortest time is a second.** A shorter window makes no sense in this module.
- **A bad value in a constant or in a `const { }` block does not compile.** Written straight into `rule()`, it causes a panic with a message when the rule is read.
- **The builder has a required step `.limits(AppLimit::VARIANTS)`** and reads the rule of every limit when it builds. So the panic happens when the application starts, not at the first attempt in that limit.
- **`build()` returns no error.**
- **An attempt in a limit that was not on the list is the error `LimitsError::UnknownLimit`.**

## Why

- **A value that cannot be written does not need to be checked.** The validating function, its constants and the error variant are gone.
- **The bounds follow from the type.** The maximum is the size of `u32`, not a number chosen by the module.
- **136 years fit in a date with room to spare,** so the end of a window can always be computed.
- **It is better not to start than to run with a limit that does not limit.** Hence reading all the rules when the module is built.
- **The compiler checks more with a `match` than with attributes.**
- **Without `UnknownLimit`, the reading at startup could be bypassed** by giving the builder only some of the limits or by using a second enum.
- **The module knows no norms.** Whether "5 per hour" is a good limit is for the product to decide.

## Consequences

- **A bad value written straight into `rule()` is a panic, not a `Result`.** It is a mistake in a constant in the product's code, like a hard-coded division by zero.
- **`compile_fail` tests in the documentation of `Count` and `Period`** make sure that zero and a time that is too long do not compile in a constant.
- **An application with two enums of limits gives both on the list,** as one common type.
- **The database still gets `u32` and `Duration`:** `Attempts::hit` unpacks the types before the query.
