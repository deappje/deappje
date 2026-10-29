-- migrate:up

-- Processing-object type `group_purge` (key 2) of nanoservice
-- `group`: the registry row, the envelope partition INSIDE this
-- nanoservice's schema, and the typed spec/status tables hanging off the
-- partition by composite foreign key. The envelope owns identity,
-- generation, scheduling, deletion intent and claim authority; these tables
-- carry domain columns only (the Directive §3 invariant 1).
--
-- The registry row and the partition need the envelope parent's owner, the
-- `app` login this migration runs as, so they come first; the partition is
-- then handed to the nanoservice role, whose typed tables follow under
-- SET LOCAL ROLE. Every framework statement targets the partition, and the
-- typed tables reference the partition, so the role needs nothing on the
-- parent. RESET ROLE at the end: dbmate records this version in the same
-- transaction, and the role may not write the ledger.

INSERT INTO basable.processing_object_type (key, name) VALUES (2, 'group_purge');

CREATE TABLE nano_group.processing_object_group_purge
    PARTITION OF basable.processing_object FOR VALUES IN (2)
    WITH (fillfactor = 90);
ALTER TABLE nano_group.processing_object_group_purge OWNER TO nano_group;

SET LOCAL ROLE nano_group;
SET LOCAL search_path = nano_group;

-- Desired state only.
CREATE TABLE group_purge_spec (
    id UUID PRIMARY KEY,
    processing_object_type_key SMALLINT NOT NULL DEFAULT 2
        CHECK (processing_object_type_key = 2),
    group_id UUID NOT NULL, -- immutable (trigger below)
    parent_purge_id UUID, -- immutable (trigger below)
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    FOREIGN KEY (processing_object_type_key, id)
        REFERENCES processing_object_group_purge (processing_object_type_key, id)
        ON DELETE CASCADE
);

-- Identity columns can never be retargeted, whatever the query.
CREATE FUNCTION reject_group_purge_spec_identity_change()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.group_id IS DISTINCT FROM OLD.group_id
       OR NEW.parent_purge_id IS DISTINCT FROM OLD.parent_purge_id THEN
        RAISE EXCEPTION 'group_purge identity is immutable (attempted change on %)', OLD.id
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_group_purge_spec_identity_immutable
    BEFORE UPDATE ON group_purge_spec
    FOR EACH ROW EXECUTE FUNCTION reject_group_purge_spec_identity_change();

-- Observed state only, written exclusively under exact claim authority
-- (fenced completions and Claim::write_status). No unfenced writer exists.
CREATE TABLE group_purge_status (
    id UUID PRIMARY KEY,
    processing_object_type_key SMALLINT NOT NULL DEFAULT 2
        CHECK (processing_object_type_key = 2),
    phase TEXT NOT NULL,
    subgroups_remaining INTEGER NOT NULL,
    objects_remaining BIGINT,
    blobs_remaining BIGINT,
    keys_remaining BIGINT,
    -- Declared-effect slot (uncomment when an adapter uses Strategy::Declared):
    -- effect_operation VARCHAR(64),
    -- effect_key TEXT,
    -- effect_declared_at TIMESTAMPTZ,
    -- effect_detail JSONB,
    -- CHECK ((effect_operation IS NULL) = (effect_key IS NULL) AND (effect_operation IS NULL) = (effect_declared_at IS NULL)),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    FOREIGN KEY (processing_object_type_key, id)
        REFERENCES processing_object_group_purge (processing_object_type_key, id)
        ON DELETE CASCADE
);

RESET ROLE;

-- migrate:down
SET LOCAL ROLE nano_group;
SET LOCAL search_path = nano_group;
DROP TABLE IF EXISTS group_purge_status;
DROP TABLE IF EXISTS group_purge_spec;
DROP FUNCTION IF EXISTS reject_group_purge_spec_identity_change();
RESET ROLE;
DROP TABLE IF EXISTS nano_group.processing_object_group_purge;
DELETE FROM basable.processing_object_type WHERE key = 2 AND name = 'group_purge';
