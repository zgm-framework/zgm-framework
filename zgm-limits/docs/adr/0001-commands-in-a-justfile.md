# ADR-0001: The commands that build and run the crate live in a `justfile`

> **Status:** accepted
> **Date:** 2026-11-10
> **Applies to:** `zgm-limits` (`justfile`)

## Context

- **The crate needs short commands to build and run it.**
- **The repository has no file with commands and no CI yet,** so there is no pattern to follow.

## Options considered

| Option | Pros | Cons |
|---|---|---|
| **A. `just`** | a tool for commands only; `just --list` shows the list; no `.PHONY` and no required tabs | every other person and a future CI must install it |
| **B. `make`** | already present on macOS with the developer tools and on almost every Linux | a tool for building files, so every command needs `.PHONY`, and indentation must be a tab |
| **C. No file, only `cargo`** | no new files; `cargo run` already builds and runs | longer commands will come back when tests with a database arrive (docker and `DATABASE_URL`) |

## Decision

**Option A** (decided by the maintainer on 2026-11-10).

- **The `justfile` lives in `zgm-limits/`.**
- **`just` builds and runs the program, and `just build` only builds it.**

## Why

- **`just` is for commands only,** so the file is shorter and has none of the workarounds that `make` needs.
- **The maintainer already has it installed.**
- **The file will be more useful when tests with a database arrive,** because the commands will be longer then.

## Consequences

- **Every other person and a future CI must have `just`.**
- **The decision covers only `zgm-limits`.**
