-- migrate:up
SET LOCAL ROLE nano_device;
SET LOCAL search_path = nano_device;

-- Plain table `wrapped_key` of nanoservice `device`. Only this
-- nanoservice's role can reach it (the Directive §2).
CREATE TABLE wrapped_key (
    id UUID NOT NULL PRIMARY KEY,
    group_id UUID NOT NULL,
    key_id INTEGER NOT NULL,
    device_id UUID NOT NULL,
    wrapped BYTEA NOT NULL,
    wrapped_by_device UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE INDEX idx_wrapped_key_group_id_key_id_device_id ON wrapped_key (group_id, key_id, device_id);

CREATE INDEX idx_wrapped_key_device_id ON wrapped_key (device_id);

-- dbmate records this version in the same transaction, and the role may
-- not write the ledger.
RESET ROLE;

-- migrate:down
SET LOCAL ROLE nano_device;
SET LOCAL search_path = nano_device;
DROP TABLE IF EXISTS wrapped_key;
RESET ROLE;
