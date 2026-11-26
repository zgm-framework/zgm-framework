# ADR-0002: The defaults and their bounds are built into the modules and written to the database once, when a module first starts; later only a function of the settings module changes them

> **Status:** accepted
> **Date:** 2026-11-24
> **Applies to:** `zgm-settings` (`zgm_settings.default_settings`, `zgm_settings.tenant_settings`), and the modules that have settings

## Context

- **A setting has a default for every tenant and, for some tenants, a value of their own** ([ADR-0001](0001-settings-in-a-module-of-their-own.md)). E.g. a user waits 30 days for deletion, and the tenant Nowak keeps 90.
- **A tenant may set a value only within bounds,** e.g. between a day and a year. The bounds belong to the setting, as the default does.
- **The settings module does not know the settings of other modules,** and a number such as "30 days" is not its choice.
- **`zgm-limits` keeps its defaults in code** and only the changes of tenants in its table, so a new default reaches every tenant that has not changed the rule ([ADR-0017 of `zgm-limits`](../../../zgm-limits/docs/adr/0017-tenant-rules-in-the-database.md)).
- **The maintainer wants the defaults and their bounds in the database,** where they can be changed while the program runs, without a new version of a module.
- **`CONTRIBUTING.md`:** "A module gives mechanisms, not values from standards. It enforces only what makes the mechanism work at all."

## Options considered

**Where the defaults lie:**

| Option | Pros | Cons |
|---|---|---|
| **A. In code, kept in memory; the table holds only what tenants changed** | a new default reaches every tenant that has not changed the setting; every row is under a tenant | the defaults are not in the database; a new default or a new bound needs a new version of the module and a deployment |
| **B. In the database, in a table of their own, written by the module when it first starts and not touched after** | the defaults and their bounds are in the database and change without a deployment; two versions of a program that start side by side both find the row and leave it, so the value does not flip | a row without a tenant; a new number in a module's code does not reach a database that has the row already; the module writes to the database when it starts, and that can fail |
| **C. In the database, written again from the code at every start** | the defaults are in the database; the code stays where a default comes from | a row without a tenant; during a deployment the old and the new version write their own number in turn, so the value flips; a change made in the database is lost at the next start |
| **D. Copied into the rows of a tenant when it is created** | every tenant has all its values in the database | a new default does not reach the tenants that already exist; the settings module would have to learn that `zgm-iam` created a tenant |
| **E. Written by a migration of the program, once** | no Rust code holds the number; a change is a new migration, run once | each program writes every setting of every module it uses; a mistyped name, a wrong value or a forgotten row shows only when the setting is first read, not when the program starts |

**Who gives the default and its bounds:**

| Option | Pros | Cons |
|---|---|---|
| **A. The program, in a required step of the module's builder** | nobody gets a number they did not choose; a forgotten value does not compile | every program states every setting, even when the usual value suits it |
| **B. The module has them built in, and the program may override them in an optional step** | a program states only what it wants different | a program that set nothing gets the module's number without knowing it; a second place a number comes from |
| **C. The module has them built in, and only a tenant changes the value, for itself** | one place a number comes from, besides the row of a tenant; no program states anything | every program starts with the module's numbers |

**Who changes a default or its bounds later:**

| Option | Pros | Cons |
|---|---|---|
| **A. Only a function of the settings module** | the code that checks the value of a tenant checks a new default too, so the rules live in one place | a change needs a program that calls the function |
| **B. Also SQL by hand** | no code is needed for a change | the rules (the kind, the least not above the most) would have to be in the database as well, in a second place, where they drift from the code |

## Decision

**Where: B. Who: C. Who changes later: A** (decided by the maintainer on 2026-11-24, after the defaults had first been designed in memory: "but they were supposed to be in the database. Only when somebody wants to override does a row with `tenant_id` appear, and if they don't override, there has to be a separate default table to keep it in, and min and max go there too"; on what happens after the first start: "with this step min and max get frozen, and if you then change it in the code, nothing happens, because it has already been added to the database"; and on later changes: "only a function of the module changes it". On who gives the default, on 2026-11-23: "underneath we can set the default values up front, and then, when a product built on these modules wants a change, it overrides it, instead of requiring everybody to state it here in the builder"; and on 2026-11-24, when that override was designed as an optional step of the builder of `zgm-iam`: "it should rather not be there at all", "when there is an admin panel every tenant has access to, it will change it there if it wants to").

- **A module that has settings has an enum of them,** e.g. `IamSetting::UserDeletionWindow`, and a built-in default for each. A duration or a number also has both bounds built in, each of them possibly none; a flag or a text has no bounds.
- **The defaults lie in `zgm_settings.default_settings`,** one row per setting ([ADR-0003](0003-the-tenant-settings-table.md)). When a module starts, it gives its defaults to the settings module, which writes the row of every setting that has none. A row that is there is left as it is.
- **A later change of a default or of its bounds goes through a function of the settings module.**
- **A program does not change a default.** A module's builder has no step for it.
- **A tenant that wants its own value has a row in `zgm_settings.tenant_settings`;** no row means the default.

## Why

- **A default a tenant can change enforces nothing,** so it is a mechanism with a starting value, not a value from a standard that the module imposes.
- **In the database,** because the maintainer wants to change a default or a bound of a program that is running, without a new version of the module.
- **Written once and then left:** two versions of a program that run side by side during a deployment both find the row, so neither overwrites the other, and a change made later is not undone at the next start.
- **Only through a function:** the rules of a value are checked in one place, the module's code, and the database keeps only what that code cannot guarantee on its own.
- **A mistake in a name does not compile,** as a name comes from an enum.

## Consequences

- **`default_settings` has no tenant,** as a default is the same for every tenant. It is the one table of this module whose rows are not under a tenant.
- **A new number in a module's code reaches only a database without the row.** A database that has the row keeps the number it was first given, until it is changed through the function.
- **A setting a module no longer has keeps its row.** Nothing deletes it yet.
- **Starting a module writes to the database,** so it can fail when the database does not answer.
