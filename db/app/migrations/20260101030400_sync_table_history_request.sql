-- migrate:up
SET LOCAL ROLE nano_sync;
SET LOCAL search_path = nano_sync;

-- Plain table `history_request` of nanoservice `sync`. Only this
-- nanoservice's role can reach it (the Directive §2).
CREATE TABLE history_request (
    id UUID NOT NULL PRIMARY KEY,
    group_id UUID NOT NULL,
    requester_device UUID NOT NULL,
    from_seq BIGINT NOT NULL,
    to_seq BIGINT NOT NULL,
    object_id UUID,
    claimed_by UUID,
    claimed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE INDEX idx_history_request_group_id ON history_request (group_id);

-- dbmate records this version in the same transaction, and the role may
-- not write the ledger.
RESET ROLE;

-- migrate:down
SET LOCAL ROLE nano_sync;
SET LOCAL search_path = nano_sync;
DROP TABLE IF EXISTS history_request;
RESET ROLE;
