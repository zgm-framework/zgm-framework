# Changelog

## 0.1.0 (not released yet)

The first version.

- **Defaults from the modules, in the database:** a module gives the default of each of its settings when it starts (`Settings::add_default`, `DefaultSetting`), by a name with the prefix of the module, e.g. `iam.user_deletion_window`. The first start writes it to `default_settings`, later starts leave it; `Settings::defaults` reads them all.
- **Bounds:** a duration or a number may have a least and a most, given with its default; `set` refuses a value outside them.
- **Changing a default** (`Settings::change_default`): the default and its bounds, refusing another kind, a least above the most and a default outside its bounds.
- **Four kinds of value** (`Value`): a duration in whole seconds, a whole number, yes or no, a text.
- **The settings of one tenant** (`Settings::tenant_id`): `get` the value or the default, `set` a value of the tenant's own, `restore_default`, and `list` all of them with their defaults, bounds and whether the tenant changed them.
- **Two tables** the module creates itself (`migrate`): the defaults, and the values the tenants changed, one row per tenant and setting.
