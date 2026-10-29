-- migrate:up
SET LOCAL ROLE nano_device;
SET LOCAL search_path = nano_device;

-- Plain table `device` of nanoservice `device`. Only this
-- nanoservice's role can reach it (the Directive §2).
-- `created_at` and `updated_at` are added when the plan declares none of
-- its own; a declared one is the plan's, without the default.
CREATE TABLE device (
    id UUID NOT NULL PRIMARY KEY,
    user_id UUID NOT NULL,
    public_key BYTEA NOT NULL,
    platform TEXT NOT NULL,
    push_provider TEXT,
    push_token TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    last_seen_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE INDEX idx_device_user_id ON device (user_id);

-- dbmate records this version in the same transaction, and the role may
-- not write the ledger.
RESET ROLE;

-- migrate:down
SET LOCAL ROLE nano_device;
SET LOCAL search_path = nano_device;
DROP TABLE IF EXISTS device;
RESET ROLE;
