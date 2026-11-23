# ADR-0005: `Settings` is built with a `bon` builder; the modules give it defaults of the four kinds of `Value`, which it keeps in memory, shared by its clones; a default given again replaces the one before

> **Status:** accepted
> **Date:** 2026-11-23
> **Applies to:** `zgm-settings` (`src/settings.rs`, `src/value.rs`, `tests/defaults.rs`)

## Context

- **The defaults are built into the modules, and each module gives them to the settings module when it is built** ([ADR-0002](0002-defaults-built-into-the-modules.md)). Nothing of them goes to the database.
- **In the database a value is `jsonb`** ([ADR-0003](0003-the-tenant-settings-table.md)), e.g. `7776000` for 90 days in seconds.
- **A setting is named with the prefix of its module,** e.g. `iam.user_deletion_window`, from an enum of the module through `strum` ([ADR-0003](0003-the-tenant-settings-table.md)).
- **`zgm-limits` and `zgm-iam` are built with `bon` builders,** once when the program starts, and cloned where they are needed.

## Options considered

**What a value is in Rust:**

| Option | Pros | Cons |
|---|---|---|
| **A. Plain JSON (`serde_json::Value`), with "time is always in seconds" agreed in a record** | one type through the whole module; a module turns JSON into its own type in a few lines | nobody checks that the modules keep to the agreement; a screen of settings shows `7776000` and does not know it is time; one more crate |
| **B. The module's own type, which the settings module turns into JSON and back through `serde`** | convenient in the module that reads the setting | the list of all the settings of a company needs one type anyway, so the module would have two ways; `serde` in every type |
| **C. A few kinds the settings module knows: time, a number, yes or no, a text** | the agreement on time is in the code, not on paper; a screen can show 90 days, a box for yes or no, a field for a number; a value of another kind than the default's can be refused | a new kind, e.g. several numbers at once, is a change of the settings module |

**What `add_default` does with a name given before:**

| Option | Pros | Cons |
|---|---|---|
| **A. The same value changes nothing; another value stops the program at startup** | a program stops only when nobody can tell which number is meant, and at once | about four lines and a test more |
| **B. Every second default stops the program, even with the same value** | the simplest check | two copies of one module on one `Settings`, e.g. in tests, do not work at all |
| **C. The second replaces the first** | the least code: one write to a map | with 30 and then 14 days the later wins, and nobody learns of it |

## Decision

**Value: C. A name given again: C** (decided by the maintainer on 2026-11-23: on the value, after "we would need to assume that things of this kind are always in seconds or something", "then 2"; on the name, "let it overwrite, I guess? Whichever is simplest and the least code to implement").

```rust
let settings = Settings::builder().pool(pool).build();
settings.add_default(IamSetting::UserDeletionWindow, Value::Duration(thirty_days));
settings.defaults(); // {"iam.user_deletion_window": Duration(30 days)}
```

- **`Value` has four kinds:** `Duration(std::time::Duration)`, `Number(i64)`, `Flag(bool)` and `Text(String)`. How each lies in the `jsonb` column is in [ADR-0006](0006-get-a-setting-of-a-tenant.md).
- **`add_default(name, value)` takes `impl AsRef<str>`,** so a variant of an enum that derives `strum::AsRefStr` gives its name, and plain text works too. The name is kept exactly as it was given.
- **A default given again under the same name replaces the one before.**
- **`defaults()` gives all the defaults given so far,** as a `BTreeMap` by name, so in the order of the names, without asking the database. `zgm-limits` has the same in `get_defaults()`.
- **The defaults lie in memory, behind an `Arc<RwLock<…>>`,** so the clones of one `Settings` share them: what a module gives through its clone, the program sees through its own. A lock poisoned by a panic of another thread is taken over as it is, since an insert into the map cannot be left half done.
- **The builder takes the pool of connections,** for the functions that read and write the rows of the tenants ([ADR-0006](0006-get-a-setting-of-a-tenant.md)). It took it from the first step on, before any function used it, so that the start of a program did not change when they came.

## Why

- **The kinds:** the maintainer's point that time needs one agreed form is kept best by the code that writes it. The settings module then knows the kind of every setting from its default, which a screen and a check of `set` both need.
- **`std::time::Duration`:** a plain Rust type, with no crate and no negative lengths. `zgm-iam` keeps its dates with `jiff`; turning a `jiff` length into this one is its own work.
- **`i64` for a number:** a whole number covers counts and limits; fractions come when a setting needs them.
- **Replacing:** the least code, and with a prefix of the module and a name from an enum, two different defaults under one name come only from a program that builds the same module twice with different numbers.
- **`bon` 3.10.2 is the version `zgm-limits` uses.** Checked on 2026-11-23: not yanked, OSV knows no vulnerabilities in it, its repository is not archived; 3.10.3 exists, and the repository keeps one version in all its modules. The tests use `strum` 0.28.0, checked the same way.
- **Checked by tests,** without a database (the pool never connects): new settings have no defaults; a default of every kind is kept as it was given; a variant of an enum is named `iam.user_deletion_window`; a name given as plain text, `billing.Invoice Footer`, is kept exactly; a second default of a name replaces the first; the defaults of several modules are all there, in the order of their names; a clone sees what was given through another. Each was broken on purpose one at a time (a clone that copies the defaults, the first default kept, the prefix dropped, another value kept, a default in a new module, the name in lower case), and every time a test went red; the last one needed the test of a name in plain text, which was added for it.

## Consequences

- **Every copy of the program holds the defaults of its own version** ([ADR-0002](0002-defaults-built-into-the-modules.md)).
- **A wrong second default is silent.** If that ever matters, the check of option A can be added without changing a call.
- **A setting of several numbers at once** needs a new kind of `Value` first.
- **Nothing checks that a name has a prefix:** any text is a name.
