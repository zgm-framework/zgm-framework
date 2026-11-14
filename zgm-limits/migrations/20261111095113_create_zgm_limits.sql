-- The whole database of the module, in one migration: its schema, the table of counters and
-- the table of the rules tenants set for themselves.

-- Everything of the module lives in its own schema. `zgm_limits::migrate` creates it before
-- this file runs, to keep the history of migrations there; a plain run of the file creates it
-- here.
CREATE SCHEMA IF NOT EXISTS zgm_limits;

-- Sets updated_at on every UPDATE. statement_timestamp(), not now(): now() is the start of the
-- transaction, so a long transaction would write a time earlier than rows created meanwhile.
CREATE FUNCTION zgm_limits.set_updated_at()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    NEW.updated_at := statement_timestamp();
    RETURN NEW;
END;
$$;

-- One row per key in one limit, not per attempt. Only the module's code writes this table, so
-- the database keeps only what the code cannot guarantee on its own: one row per key under
-- concurrent writes. An attempt is one SQL statement that reads the row, counts and writes, so
-- no lock is held between statements; a refused attempt writes nothing and locks nothing.
--
-- Internal state: rows are deleted for good when they mean nothing any more.
CREATE TABLE zgm_limits.counters (
    -- A short name of the row for the list of blocked keys: the page after a given row, and
    -- "unblock this one". The primary key is still the key below, and attempts never look a row
    -- up by id, so id has an index over the blocked rows only (at the end of the file).
    id                bigint      GENERATED ALWAYS AS IDENTITY,

    -- The name of the limit in the product's enum, e.g. 'export'.
    limit_name        text        NOT NULL,

    -- The key: whose counter this is. Every part is NOT NULL and "no such part" has a value of
    -- its own, so that the key is a plain primary key and every lookup is plain equality. With
    -- NULLs the lookup would need IS NOT DISTINCT FROM, which PostgreSQL cannot answer from an
    -- index: every attempt would read the whole table.
    tenant_id         uuid        NOT NULL,
    -- The account the counter is about, e.g. the one somebody tries to sign in to. The nil UUID
    -- (all zeros) when the key has no account.
    subject_id        uuid        NOT NULL,
    -- The hash of a key with a hidden part, e.g. an e-mail or an IP address, which must
    -- not lie here in the open. Empty when the key has no hidden part: such a key needs no
    -- hash at all.
    -- The hash is computed from the limit name, the tenant, the account and the hidden
    -- part together, not from the hidden part alone. So the same e-mail gives a different value
    -- in every limit and every tenant: a copy of the database does not show that two rows are
    -- about the same e-mail, and a row copied under another limit or tenant matches nothing.
    -- Nil subject_id and empty key_hash together: the key is the whole tenant.
    -- A hash starts with a header that holds the version of the secret key it was
    -- computed with, so the rows of one version can be counted: an old version of the secret
    -- key may be deleted once no row holds it.
    key_hash          bytea       NOT NULL,

    -- What the limit's way of counting remembers about the key, in fields of its own. The table
    -- knows no way of counting: a window keeps here how many attempts passed in it, another way
    -- keeps other fields, and a new way needs no migration.
    state             jsonb       NOT NULL,

    -- After this moment the row means nothing and the cleanup may delete it; for a window it is
    -- the end of the window. Every way of counting writes its own moment here, so the cleanup
    -- needs to know no rule of any limit.
    expires_at        timestamptz NOT NULL,
    -- The moment the key got blocked: from then on no attempt passes until expires_at. NULL
    -- when the key is not blocked. Every way of counting writes it its own way; a window
    -- writes it with the attempt that fills the window. It is a column and not a field of
    -- state, so that the list of blocked keys needs to know no way of counting.
    blocked_at        timestamptz,

    created_at        timestamptz NOT NULL DEFAULT statement_timestamp(),
    updated_at        timestamptz NOT NULL DEFAULT statement_timestamp(),

    CONSTRAINT counters_pkey PRIMARY KEY (limit_name, tenant_id, subject_id, key_hash)
);

CREATE TRIGGER touch_counters
    BEFORE UPDATE ON zgm_limits.counters
    FOR EACH ROW EXECUTE FUNCTION zgm_limits.set_updated_at();

-- The cleanup finds the rows that mean nothing any more without reading the whole table.
CREATE INDEX counters_expires_at_idx ON zgm_limits.counters (expires_at);

-- The list of the blocked keys of one tenant, the newest first, page after page. Only the
-- blocked rows are in the index, and they are few among all the counters. id breaks the tie
-- between rows blocked at the same moment, so that a page starts exactly after the last row of
-- the page before.
CREATE INDEX counters_blocked_idx ON zgm_limits.counters (tenant_id, blocked_at, id)
    WHERE blocked_at IS NOT NULL;

-- Unblocking finds one blocked row by its id. Also over the blocked rows only, so an attempt
-- that blocks nobody writes neither of these two indexes.
CREATE UNIQUE INDEX counters_blocked_id_idx ON zgm_limits.counters (id)
    WHERE blocked_at IS NOT NULL;

-- The rules a tenant set for itself in place of the defaults from the code. One row per limit
-- the tenant changed; no row means the default. The defaults themselves are not in the
-- database.
--
-- Only the module's code writes this table. A row is deleted for good when the tenant goes back
-- to the default.
CREATE TABLE zgm_limits.tenant_rules (
    tenant_id   uuid        NOT NULL,
    -- The name of the limit in the product's enum, e.g. 'export'.
    limit_name  text        NOT NULL,

    -- The rule in force in this tenant: its way of counting and that way's own numbers, e.g.
    -- {"kind": "window", "at_most": 10, "per_seconds": 3600}. It is one field, like the state
    -- of a counter, because every way of counting has numbers of its own, and a new way needs
    -- no migration.
    rule        jsonb       NOT NULL,

    created_at  timestamptz NOT NULL DEFAULT statement_timestamp(),
    updated_at  timestamptz NOT NULL DEFAULT statement_timestamp(),

    -- Every attempt reads the row of its tenant and limit by this key, in the statement that
    -- counts the attempt.
    CONSTRAINT tenant_rules_pkey PRIMARY KEY (tenant_id, limit_name)
);

CREATE TRIGGER touch_tenant_rules
    BEFORE UPDATE ON zgm_limits.tenant_rules
    FOR EACH ROW EXECUTE FUNCTION zgm_limits.set_updated_at();
