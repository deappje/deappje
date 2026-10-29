-- migrate:up
SET LOCAL ROLE nano_sync;
SET LOCAL search_path = nano_sync;

-- Plain table `group_device` of nanoservice `sync`. Only this
-- nanoservice's role can reach it (the Directive §2).
CREATE TABLE group_device (
    group_id UUID NOT NULL,
    device_id UUID NOT NULL,
    acked_seq BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE UNIQUE INDEX idx_group_device_group_id_device_id ON group_device (group_id, device_id);

CREATE INDEX idx_group_device_device_id ON group_device (device_id);

-- dbmate records this version in the same transaction, and the role may
-- not write the ledger.
RESET ROLE;

-- migrate:down
SET LOCAL ROLE nano_sync;
SET LOCAL search_path = nano_sync;
DROP TABLE IF EXISTS group_device;
RESET ROLE;
