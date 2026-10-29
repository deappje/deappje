-- migrate:up
SET LOCAL ROLE nano_storage;
SET LOCAL search_path = nano_storage;

-- Plain table `upload_usage` of nanoservice `storage`. Only this
-- nanoservice's role can reach it (the Directive §2).
-- `created_at` and `updated_at` are added when the plan declares none of
-- its own; a declared one is the plan's, without the default.
CREATE TABLE upload_usage (
    group_id UUID NOT NULL,
    hour TIMESTAMPTZ NOT NULL,
    bytes BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE UNIQUE INDEX idx_upload_usage_group_id_hour ON upload_usage (group_id, hour);

-- dbmate records this version in the same transaction, and the role may
-- not write the ledger.
RESET ROLE;

-- migrate:down
SET LOCAL ROLE nano_storage;
SET LOCAL search_path = nano_storage;
DROP TABLE IF EXISTS upload_usage;
RESET ROLE;
