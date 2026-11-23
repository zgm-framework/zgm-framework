-- The schema of the module and its table: the settings that tenants changed.

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

-- A setting one tenant changed: one row per tenant and setting, and only for the settings the
-- tenant changed. No row means the default, which the module of the setting gives when it is
-- built; the defaults are not in the database, so a new default reaches every tenant that has
-- not changed the setting.
--
-- Going back to the default deletes the row. Only the module's code writes this table, so the
-- database keeps only what that code cannot guarantee on its own (one value per tenant and
-- setting) and the dates it sets itself.
CREATE TABLE zgm_settings.tenant_settings (
    -- The tenant, as a plain UUID: the tenants live in another module, so there is no foreign
    -- key, and a tenant deleted there leaves its rows here.
    tenant_id   uuid        NOT NULL,

    -- The name of the setting with the prefix of its module, e.g. 'iam.user_deletion_window'.
    -- The module turns a variant of its enum of settings into this name.
    name        text        NOT NULL,

    -- The value the tenant set, as JSON of any kind: 7776000, true, "text" or
    -- {"at_most": 10, "per_seconds": 3600}. The database checks only that it is JSON; the
    -- module that reads the setting checks that it is what the setting needs.
    value       jsonb       NOT NULL,

    created_at  timestamptz NOT NULL DEFAULT statement_timestamp(),
    updated_at  timestamptz NOT NULL DEFAULT statement_timestamp(),

    -- The tenant first: one setting of a tenant and all the settings of a tenant are both found
    -- through this index. Nothing refers to a row, so there is no id of its own.
    CONSTRAINT tenant_settings_pkey PRIMARY KEY (tenant_id, name)
);

CREATE TRIGGER touch_tenant_settings
    BEFORE UPDATE ON zgm_settings.tenant_settings
    FOR EACH ROW EXECUTE FUNCTION zgm_settings.set_updated_at();
