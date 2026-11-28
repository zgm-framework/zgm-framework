-- The schema of the module and its tables: the tenants and their users.

-- Everything of the module lives in its own schema. `zgm_iam::migrate` creates it before this
-- file runs, to keep the history of migrations there; a plain run of the file creates it here.
CREATE SCHEMA IF NOT EXISTS zgm_iam;

-- Sets updated_at on every UPDATE. statement_timestamp(), not now(): now() is the start of the
-- transaction, so a long transaction would write a time earlier than rows created meanwhile.
CREATE FUNCTION zgm_iam.set_updated_at()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    NEW.updated_at := statement_timestamp();
    RETURN NEW;
END;
$$;

-- A tenant: the root of everything the module keeps.
--
-- A row says that a tenant exists, since when and in which state, and nothing about who it
-- is: a copy of the database without the application and its keys names no company.
--
-- Only the module's code writes this table, so the database keeps only what that code cannot
-- guarantee on its own and the values the database sets itself: the id, the dates and the
-- state of a new tenant.
CREATE TABLE zgm_iam.tenants (
    -- The identifier of the tenant, and the only one. A UUID v7 starts with the time, so new
    -- rows go to the end of the index, the way numbers from a sequence do.
    id          uuid        PRIMARY KEY DEFAULT uuidv7(),

    -- The name of the tenant, as the bytes the application's cipher made of it. The module has
    -- no key and no cipher of its own, so here the name cannot be read, searched or sorted.
    name        bytea       NOT NULL,

    -- One of: active, suspended, pending_deletion. The list lives in the State enum in Rust;
    -- only the module's code writes the column, so the database does not check the word.
    state       text        NOT NULL DEFAULT 'active',

    -- When the row will be deleted: set together with pending_deletion, NULL in the other
    -- states. The date of deletion and not the moment of closing, so a later change of the
    -- window does not move the date of anyone already waiting.
    deletes_at  timestamptz,

    created_at  timestamptz NOT NULL DEFAULT statement_timestamp(),
    updated_at  timestamptz NOT NULL DEFAULT statement_timestamp()
);

CREATE TRIGGER touch_tenants
    BEFORE UPDATE ON zgm_iam.tenants
    FOR EACH ROW EXECUTE FUNCTION zgm_iam.set_updated_at();

-- The tenants waiting for deletion, by their date, for the loop that deletes those past it.
-- Only the tenants in pending_deletion are in it, so a pass with nothing to delete reads a few
-- index pages, whatever the number of tenants.
CREATE INDEX tenants_pending_deletion_idx ON zgm_iam.tenants (deletes_at)
    WHERE state = 'pending_deletion';

-- A user of a tenant: someone, or something, that acts under it.
--
-- A row says that a user exists, under which tenant, since when and in which state. The module
-- keeps nothing that says who the user is: the application keeps the name and the e-mail, and
-- the login module what the user signs in with.
--
-- Only the module's code writes this table. That code also keeps every link between rows
-- inside one tenant; the database does not check it.
CREATE TABLE zgm_iam.users (
    -- The identifier of the user, and the only one, a UUID v7 as for tenants.
    id          uuid        PRIMARY KEY DEFAULT uuidv7(),

    -- CASCADE: a tenant is deleted only after its window for deletion, and its users go with
    -- it, so nothing of a deleted tenant stays behind.
    tenant_id   uuid        NOT NULL REFERENCES zgm_iam.tenants (id) ON DELETE CASCADE,

    -- One of: active, suspended, pending_deletion, as for tenants.
    state       text        NOT NULL DEFAULT 'active',

    -- When the row will be deleted, as for tenants.
    deletes_at  timestamptz,

    created_at  timestamptz NOT NULL DEFAULT statement_timestamp(),
    updated_at  timestamptz NOT NULL DEFAULT statement_timestamp()
);

-- The list of the users of a tenant, a page at a time in the order of the ids, from either
-- end. It is also the index of the foreign key, which PostgreSQL does not make: without it,
-- deleting a tenant reads the whole table of users to find its users.
CREATE INDEX users_tenant_listing_idx ON zgm_iam.users (tenant_id, id);

-- The same list with the users of one state only. Without it, a page of the 3 suspended users
-- of a tenant with 50 000 reads all 50 000.
CREATE INDEX users_tenant_state_listing_idx ON zgm_iam.users (tenant_id, state, id);

-- The users waiting for deletion, by their date, for the loop that deletes those past it, in
-- every tenant. Only the users in pending_deletion are in it, so a pass with nothing to delete
-- reads a few index pages, whatever the number of users.
CREATE INDEX users_pending_deletion_idx ON zgm_iam.users (deletes_at)
    WHERE state = 'pending_deletion';

CREATE TRIGGER touch_users
    BEFORE UPDATE ON zgm_iam.users
    FOR EACH ROW EXECUTE FUNCTION zgm_iam.set_updated_at();
