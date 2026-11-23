# Changelog

## 0.1.0 (2026-11-23)

The first version.

- **Defaults from the modules:** a module gives the default of each of its settings when it is built (`Settings::add_default`), by a name with the prefix of the module, e.g. `iam.user_deletion_window`. The defaults stay in memory; `Settings::defaults` lists them.
- **Four kinds of value** (`Value`): a duration in whole seconds, a whole number, yes or no, a text.
- **The settings of one tenant** (`Settings::tenant_id`): `get` the value or the default, `set` a value of the tenant's own, `restore_default`, and `list` all of them with their defaults and whether the tenant changed them.
- **Only what a tenant changed is in the database,** one row per tenant and setting, in a table the module creates itself (`migrate`).
