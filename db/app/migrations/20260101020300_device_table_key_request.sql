-- migrate:up
SET LOCAL ROLE nano_device;
SET LOCAL search_path = nano_device;

-- Plain table `key_request` of nanoservice `device`. Only this
-- nanoservice's role can reach it (the Directive §2).
-- `created_at` and `updated_at` are added when the plan declares none of
-- its own; a declared one is the plan's, without the default.
CREATE TABLE key_request (
    id UUID NOT NULL PRIMARY KEY,
    group_id UUID NOT NULL,
    device_id UUID NOT NULL,
    invite_id UUID,
    invite_tag BYTEA,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE UNIQUE INDEX idx_key_request_group_id_device_id ON key_request (group_id, device_id);

-- dbmate records this version in the same transaction, and the role may
-- not write the ledger.
RESET ROLE;

-- migrate:down
SET LOCAL ROLE nano_device;
SET LOCAL search_path = nano_device;
DROP TABLE IF EXISTS key_request;
RESET ROLE;
