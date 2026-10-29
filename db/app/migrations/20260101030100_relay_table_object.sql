-- migrate:up
SET LOCAL ROLE nano_relay;
SET LOCAL search_path = nano_relay;

-- Plain table `object` of nanoservice `relay`. Only this
-- nanoservice's role can reach it (the Directive §2).
CREATE TABLE object (
    id UUID NOT NULL PRIMARY KEY,
    group_id UUID NOT NULL,
    value BYTEA,
    seq BIGINT NOT NULL,
    deleted BOOLEAN NOT NULL,
    uploaded_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE UNIQUE INDEX idx_object_group_id_seq ON object (group_id, seq);

CREATE INDEX idx_object_uploaded_at ON object (uploaded_at);

-- dbmate records this version in the same transaction, and the role may
-- not write the ledger.
RESET ROLE;

-- migrate:down
SET LOCAL ROLE nano_relay;
SET LOCAL search_path = nano_relay;
DROP TABLE IF EXISTS object;
RESET ROLE;
