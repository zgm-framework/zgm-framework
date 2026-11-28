# ADR-0011: The settings of tenants come from the application through a `TenantSettings` trait, required in the builder; `build()` gives the settings a user deletion window of 30 days, from a day to a year, and waits for it to be written

> **Status:** accepted
> **Date:** 2026-11-28
> **Applies to:** `zgm-iam` (`src/tenant_settings.rs`, `src/iam_setting.rs`, `DurationDefault`, `Iam::builder`, `tests/tenant_settings.rs`)

## Context

- **A user waits for deletion for some time, during which the deletion can be cancelled,** and a tenant may want its own length of time. Scheduling a deletion is the next function of the module.
- **The settings of tenants are a module of their own, `zgm-settings`,** which other modules reach through a trait in their builders ([ADR-0001 of `zgm-settings`](../../../zgm-settings/docs/adr/0001-settings-in-a-module-of-their-own.md)). A module has its defaults built in, with the least and the most a tenant may set, and gives them to the settings module when it starts. `zgm-settings` 0.1.0 writes them to its database the first time and leaves them there after that, so giving a default can fail; later only its `change_default` changes them ([ADR-0002](../../../zgm-settings/docs/adr/0002-defaults-built-into-the-modules.md), [ADR-0005](../../../zgm-settings/docs/adr/0005-settings-and-its-defaults.md) and [ADR-0010](../../../zgm-settings/docs/adr/0010-change-a-default.md) of `zgm-settings`). Its `set` refuses a value outside the bounds.
- **The modules of this repository do not depend on each other.** What one needs from outside it takes through a trait the application implements: `Cipher` in this module ([ADR-0004](0004-the-cipher-from-the-application.md)), `KeyedHash` and `Audit` in `zgm-limits`.
- **`zgm-settings` knows four kinds of value** (a length of time, a whole number, a flag, a text) and keeps a length of time as `std::time::Duration`.
- **How long others keep a deleted user:** Microsoft Entra ID 30 days, then the user is gone for good ([Microsoft Learn](https://learn.microsoft.com/en-us/entra/fundamentals/users-restore)); Google Workspace 20 days, 5 before 2017 ([Google Workspace Admin Help](https://knowledge.workspace.google.com/admin/users/restore-a-recently-deleted-user)). AWS keeps a closed account for 90 days ([AWS Account Management](https://docs.aws.amazon.com/accounts/latest/reference/manage-acct-closing.html)), which here is closer to a tenant; AWS IAM deletes a user at once.

## Options considered

**Who connects the module to the settings:**

| Option | Pros | Cons |
|---|---|---|
| **A. A trait in this module; the application writes the glue to `zgm-settings`** | the way of `Cipher`, `KeyedHash` and `Audit`; the modules stay independent; an application without `zgm-settings` can give a simple implementation of its own | every application writes the glue, about 15 lines |
| **B. This module depends on `zgm-settings` and takes its `Settings`** | no glue | breaks the rule that modules do not depend on each other; the module always pulls in `zgm-settings`, and the application must use the very version of it this module uses, or the types do not match |
| **C. A small crate of its own with the trait, which `zgm-settings` implements and this module takes** | no glue; no module depends on another | a third crate; a change of the trait touches three places |

**What the trait offers:**

| Option | Pros | Cons |
|---|---|---|
| **A. Every kind of value: `add_default(name, Value)` and `get(tenant_id, name) -> Value`** | a new kind of setting in this module does not change the trait | a copy of `Value` in this module, and the glue maps one to the other both ways for four kinds, about 30 lines, while the module needs only a length of time |
| **B. A length of time only: `add_default_duration` and `duration`** | the shortest glue; `Duration` is the type of `zgm-settings`, so the glue maps nothing | a setting of another kind is a new function of the trait, which the glue of every application then has to add |

**Where the default is written to the database:**

| Option | Pros | Cons |
|---|---|---|
| **A. In `build()`, which is `async` and gives an error** | the compiler asks for the `.await?`, so it cannot be forgotten; an error shows when the application starts, as with `migrate` | building the module can fail |
| **B. In a function of its own, `iam.start().await?`, after a `build()` that stays as it was** | `build()` cannot fail | an application that forgets it has no row, and its first `delete()` of a user, maybe a week later, is an error |

**The bounds of the user deletion window:**

| Option | Pros | Cons |
|---|---|---|
| **A. From a day to a year** | the numbers the maintainer gave first ("30, 1, 365, something like that"); a tenant cannot set 0 by mistake | 0 days, "delete at once", cannot be set |
| **B. From nothing to a year** | 0 days stays "delete at once", as decided for `delete()` ([ADR-0012](0012-delete-user.md)) | a mistaken 0 deletes the user at the next run of the loop |
| **C. None** | nothing to choose | 9000 years is taken by `set` and refused only by `delete()`, as `DeletionDateOutOfRange` |

**The default of the user deletion window:**

| Option | Pros | Cons |
|---|---|---|
| **A. 30 days** | the same as Microsoft Entra ID | none found |
| **B. 20 days** | the same as Google Workspace | ten days less to notice a mistake than with A |
| **C. Another number, e.g. 14 or 90** | any number the maintainer picks | no service above to follow for a user |

**How an application changes the default for every tenant:**

| Option | Pros | Cons |
|---|---|---|
| **A. An optional step of the builder, `.user_deletion_window(...)`** | one line in the application | a second place a number comes from |
| **B. `settings.add_default(...)` in the application before the module is built** | no code in the module | whichever of the two numbers reaches an empty database first stays, and nobody is told |
| **C. It does not: only a tenant changes its own value** | one place a number comes from; no code | an application that wants another number for every tenant sets it for each one |

## Decision

**Who connects: A. What the trait offers: B. The default: A, 30 days. Changing the default for every tenant: C. Where the default is written: A. The bounds: A** (decided by the maintainer on 2026-11-23 and 2026-11-24, one by one; on the change of the default: "`.user_deletion_window(14 days)` should rather not be there at all", and "when there is an admin panel every tenant has access to, it will change it there if it wants to; we have the option to change it, so where is the problem". On 2026-11-28, on where the default is written, first "I don't understand the question", then, shown the start of an application today and after the change, "well, what do you recommend?" and "ok" to A; on the bounds, "so 30 days, min 1 day, max 365?", and "ok" once told that 0 days can then not be set).

- **`TenantSettings`** has `async fn add_default_duration(&self, name: &str, default: DurationDefault) -> Result<(), TenantSettingsError>` and `async fn duration(&self, tenant_id: Uuid, name: &str) -> Result<Duration, TenantSettingsError>`. `TenantSettingsError` wraps the implementation's error, as `CipherError` does.
- **`DurationDefault { value, min, max }`** has public fields, named so that the two bounds cannot be swapped by their order, as in `DefaultSetting` of `zgm-settings`; `None` is no bound.
- **`Iam::builder().pool(pool).cipher(cipher).settings(settings).build().await?`.** `.settings(...)` is required, like `.cipher(...)`; it takes any `impl TenantSettings`. `build()` gives the defaults and waits for them; an error of the settings is `IamError::Settings`, and there is no module then. A clone of `Iam` shares the settings.
- **`IamSetting { UserDeletionWindow }`** is public and derives `strum::AsRefStr` with `serialize_all = "snake_case"` and `prefix = "iam."`, so `as_ref` gives `iam.user_deletion_window`, the form `zgm-settings` expects. The application changes a setting of a tenant under it, so a typo does not compile.
- **`build()` gives the settings the default of every `IamSetting`, once:** the user deletion window, 30 days, at least a day and at most 365 days, and the tenant deletion window, 90 days, at least 7 days and at most 365 ([ADR-0015](0015-the-state-of-a-tenant.md)). A clone or `iam.tenant_id(...)` gives nothing again.
- **The module has no builder step for a number.** A tenant changes its own value through `zgm-settings`, e.g. later from an admin panel; the application does not change the default.
- **`duration` is read by `delete()` of a user, and of a tenant for its own window,** which takes any length the tenant has, 0 too ([ADR-0012](0012-delete-user.md)); with these bounds `zgm-settings` lets a tenant set no less than a day.

## Why

- **The modules stay independent,** in the way the repository already has three times, and an application that does not use `zgm-settings` is not forced to.
- **Only what the module needs today is in the trait,** so the glue is short and maps nothing.
- **30 days is the window of Microsoft Entra ID,** the longer of the two services above that keep a deleted user.
- **One place a number comes from:** the module or the tenant's own row, nothing in between.
- **The write in `build()`:** a step an application must remember is one it will forget, and an error at the start is found at once, not when a user is deleted.
- **From a day to a year:** the maintainer's numbers. A day is the least a tenant may set, so a 0 typed by mistake does not delete users at the next run of the loop.

## Consequences

- **Every application writes the glue.** It is about 20 lines with `zgm-settings` 0.1.0, checked on 2026-11-28 in a crate of its own on PostgreSQL 18: `build()` writes 30 days from a day to a year, `set` refuses 400 days and 0, `delete()` takes the 90 days the tenant set, and a second start leaves the row. The README of the module will show it.
- **Starting the application writes to the database of the settings,** so `build()` fails when it does not answer.
- **A tenant without a row of its own waits the default of the database:** 30 days, unless somebody changed it through `change_default` of `zgm-settings`, e.g. from a screen; this module has no function for it. A new built-in number reaches only a database without the row.
- **Every test that builds `Iam` gives it settings,** in `tests/common/mod.rs`: they remember what the module gave them and give every tenant the default. Tests in `tests/tenant_settings.rs`: `build()` gives exactly `iam.user_deletion_window`, 30 days from a day to 365 days, once; a clone and `tenant_id` give nothing again; settings that cannot write make `build()` an `IamError::Settings`. They went red first, as the trait and the async `build()` were missing.
