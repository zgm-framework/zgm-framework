# ADR-0002: The defaults are built into the modules, given to the settings module when they are built and kept in memory; the database holds only what tenants changed

> **Status:** accepted
> **Date:** 2026-11-23
> **Applies to:** `zgm-settings` (`zgm_settings.tenant_settings`; later `Settings::add_default`), and the builders of the modules that have settings

## Context

- **A setting has a default for every company and, for some companies, a value of their own** ([ADR-0001](0001-settings-in-a-module-of-their-own.md)).
- **The settings module does not know the settings of other modules,** and a number such as "30 days" is not its choice.
- **`zgm-limits` keeps its defaults in code** and only the changes of tenants in its table, so a new default reaches every tenant that has not changed the rule ([ADR-0017 of `zgm-limits`](../../../zgm-limits/docs/adr/0017-tenant-rules-in-the-database.md)).
- **`CONTRIBUTING.md`:** "A module gives mechanisms, not values from standards. It enforces only what makes the mechanism work at all."

## Options considered

**Where the defaults lie:**

| Option | Pros | Cons |
|---|---|---|
| **A. In code; the table holds only what tenants changed** | a new default reaches every company that has not changed the setting; every row is under a tenant | the defaults are not seen in the database |
| **B. In the database, in a table of their own or in rows marked `is_default`, written when the program starts** | the defaults are seen in the database | a row without a tenant; during a deployment the old and the new version of the program both start and write their own default, so the value flips; the code is still where the value comes from |
| **C. Copied into the rows of a company when it is created** | every company has all its values in the database | a new default does not reach the companies that already exist; the settings module would have to learn that `zgm-iam` created a company |
| **D. Written by a migration, once** | no Rust code holds the number; a change is a new migration, run once, so nothing flips | the migration must be the program's, not a module's, since each program picks its own number; a mistyped name, a wrong value or a forgotten row shows only when the setting is first read, e.g. when the first user is deleted, not when the program starts |

**Who gives the default:**

| Option | Pros | Cons |
|---|---|---|
| **A. The program, in a required step of the module's builder** | nobody gets a number they did not choose; a forgotten value does not compile | every program states every setting, even when the usual value suits it |
| **B. The module has it built in, and the program may override it in an optional step** | a program states only what it wants different | a program that set nothing gets the module's number without knowing it; a new built-in number in a new version of the module changes the behaviour of every program that did not override it |

## Decision

**Where: A. Who: B** (decided by the maintainer on 2026-11-23: "what if IAM exposed its own internal enum of settings? It gets filled in when the IAM builder is created, we give the settings module at the same time, and IAM calls `settings.add_default(...)` underneath"; and then: "`user_deletion_window(...)` is not needed in the builder; underneath we can set the default values up front, and then, when a product built on these modules wants a change, it overrides it, instead of requiring everybody to state it here in the builder").

- **A module that has settings has an enum of them,** e.g. `IamSetting::UserDeletionWindow`, and a built-in default for each.
- **The module's `build()` gives its defaults to the settings module** through the trait (`add_default`). The settings module keeps them in memory and writes nothing to the database.
- **A program that wants another default for every company** says so in an optional step of the module's builder.
- **A company that wants its own value has a row in `zgm_settings.tenant_settings`;** no row means the default ([ADR-0003](0003-the-tenant-settings-table.md)).
- **Which number `zgm-iam` builds in** is decided with the function that schedules a deletion.

## Why

- **A default the program can change enforces nothing,** so it is a mechanism with a starting value, not a value from a standard that the module imposes.
- **Defaults in memory leave no row without a tenant,** and the rule of the repository that every row is under a tenant has no exception here.
- **Nothing is written when a program starts,** so two versions running side by side during a deployment do not overwrite each other.
- **A mistake shows at once:** a name comes from an enum, so a typo does not compile, and a value from a builder has a Rust type.

## Consequences

- **A program that set nothing gets the numbers of the modules,** e.g. users deleted after the number of days `zgm-iam` builds in.
- **A built-in default changes only in a new major version of its module, with a note,** because every program that did not override it would behave differently after an update.
- **The defaults are not in the database.** A copy of the database says what companies changed, not what the others have; the settings module answers that.
- **Every copy of the program holds the defaults of its own version.** During a deployment two copies may answer with different defaults for a company without a row, until the old one is gone.
