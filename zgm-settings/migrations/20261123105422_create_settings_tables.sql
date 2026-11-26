-- The schema of the module and its tables: the default of every setting, and the settings
-- that tenants changed.

-- Everything of the module lives in its own schema. `zgm_settings::migrate` creates it before
-- this file runs, to keep the history of migrations there; a plain run of the file creates it
-- here.
CREATE SCHEMA IF NOT EXISTS zgm_settings;

-- Sets updated_at on every UPDATE. statement_timestamp(), not now(): now() is the start of the
-- transaction, so a long transaction would write a time earlier than rows created meanwhile.
CREATE FUNCTION zgm_settings.set_updated_at()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    NEW.updated_at := statement_timestamp();
    RETURN NEW;
END;
$$;

-- The default of a setting, for every tenant that has not changed it: one row per setting and
-- no tenant, as a default is the same for all of them. The module of the setting writes the row
-- when it first starts and does not touch it after that, so the row keeps the value and the
-- bounds it was first given, and a new number in the module's code does not change it. A later
-- change goes through a function of this module.
--
-- Only the module's code writes this table, so the database keeps only what that code cannot
-- guarantee on its own (one row per setting) and the dates it sets itself. The kind, the order
-- of the bounds and the lack of bounds on a flag or a text are the code's to check.
CREATE TABLE zgm_settings.default_settings (
    -- The name of the setting with the prefix of its module, e.g. 'iam.user_deletion_window'.
    -- The module turns a variant of its enum of settings into this name.
    name        text        NOT NULL,

    -- 'duration', 'number', 'flag' or 'text'; the Rust enum `Value` holds the list. A number of
    -- seconds and a plain number are the same JSON, so only this column tells them apart.
    kind        text        NOT NULL,

    -- The default as JSON: 2592000 (30 days in seconds), 10, false or "Welcome".
    value       jsonb       NOT NULL,

    -- The least and the most a tenant may set, in seconds for a duration; NULL is no bound, and
    -- always for a flag or a text. Whole numbers, as a duration has whole seconds.
    min         bigint,
    max         bigint,

    created_at  timestamptz NOT NULL DEFAULT statement_timestamp(),
    updated_at  timestamptz NOT NULL DEFAULT statement_timestamp(),

    -- A setting is always found by its name, so the name is the key and there is no id.
    CONSTRAINT default_settings_pkey PRIMARY KEY (name)
);

CREATE TRIGGER touch_default_settings
    BEFORE UPDATE ON zgm_settings.default_settings
    FOR EACH ROW EXECUTE FUNCTION zgm_settings.set_updated_at();

-- A setting one tenant changed: one row per tenant and setting, and only for the settings the
-- tenant changed. No row means the default, in `default_settings`.
--
-- Going back to the default deletes the row. Only the module's code writes this table, so the
-- database keeps only what that code cannot guarantee on its own (one value per tenant and
-- setting) and the dates it sets itself.
CREATE TABLE zgm_settings.tenant_settings (
    -- The tenant, as a plain UUID: the tenants live in another module, so there is no foreign
    -- key, and a tenant deleted there leaves its rows here.
    tenant_id   uuid        NOT NULL,

    -- The name of the setting, e.g. 'iam.user_deletion_window', whose default is in
    -- `default_settings`: the foreign key below.
    name        text        NOT NULL,

    -- The value the tenant set, as JSON of any kind: 7776000, true, "text" or
    -- {"at_most": 10, "per_seconds": 3600}. The database checks only that it is JSON; the
    -- module that reads the setting checks that it is what the setting needs.
    value       jsonb       NOT NULL,

    created_at  timestamptz NOT NULL DEFAULT statement_timestamp(),
    updated_at  timestamptz NOT NULL DEFAULT statement_timestamp(),

    -- The tenant first: one setting of a tenant and all the settings of a tenant are both found
    -- through this index. Nothing refers to a row, so there is no id of its own.
    CONSTRAINT tenant_settings_pkey PRIMARY KEY (tenant_id, name),

    -- A tenant has no value of a setting without a default, and a default a tenant uses is not
    -- deleted. No cascade: one DELETE of a default would take the values of every tenant that
    -- changed it.
    CONSTRAINT tenant_settings_name_fkey FOREIGN KEY (name)
        REFERENCES zgm_settings.default_settings (name) ON DELETE RESTRICT
);

-- The key begins with the tenant, so it does not find the rows of one setting; this does,
-- when the foreign key checks a default that is deleted. Without it, that check reads the
-- whole table.
CREATE INDEX tenant_settings_name_idx ON zgm_settings.tenant_settings (name);

CREATE TRIGGER touch_tenant_settings
    BEFORE UPDATE ON zgm_settings.tenant_settings
    FOR EACH ROW EXECUTE FUNCTION zgm_settings.set_updated_at();
