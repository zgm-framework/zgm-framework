# ADR-0003: Limits are an enum of the product, and `strum` gives their names and their list

> **Status:** accepted
> **Date:** 2026-11-10
> **Applies to:** `zgm-limits` (the product's enum of limits, `Cargo.toml`)

## Context

- **The module knows no limit.** The product names the limits, in its own enum, and gives the rule of each in one `match`.
- **The module needs two things from that enum:** the text name of every limit (it goes to the database) and the list of all limits (the panel shows it).

## Options considered

| Option | Pros | Cons |
|---|---|---|
| **A. `strum` (`IntoStaticStr`, `VariantArray`)** | the product adds only a variant and its rule; the names and the list come by themselves | a new dependency, `strum` 0.28.0 (before version 1.0), with the `syn` and `quote` macro crates; the product must have it in the same version as the module |
| **B. Hand-written methods** | no dependency | the name and the list are written by hand for every limit, and a variant can be left off the list |

## Decision

**Option A** (decided by the maintainer on 2026-11-10).

- **The product's enum of limits has:** `#[derive(Clone, Copy, strum::IntoStaticStr, strum::VariantArray)]` and `#[strum(serialize_all = "snake_case")]`.
- **`strum` is pinned to version `=0.28.0`.**

## Why

- **The list cannot be broken:** a new variant is on it at once.

## Consequences

- **The product is responsible for `serialize_all = "snake_case"`.** Without that line the code compiles, and the names have capital letters, e.g. `WrongCode`.
- **The name of a limit goes to the database,** so a limit in use cannot be renamed later without losing its counters.
