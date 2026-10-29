-- migrate:up
SET LOCAL ROLE nano_sync;
SET LOCAL search_path = nano_sync;

-- Plain table `group_head` of nanoservice `sync`. Only this
-- nanoservice's role can reach it (the Directive §2).
CREATE TABLE group_head (
    group_id UUID NOT NULL PRIMARY KEY,
    head_seq BIGINT NOT NULL,
    evicted_through_seq BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

-- dbmate records this version in the same transaction, and the role may
-- not write the ledger.
RESET ROLE;

-- migrate:down
SET LOCAL ROLE nano_sync;
SET LOCAL search_path = nano_sync;
DROP TABLE IF EXISTS group_head;
RESET ROLE;
