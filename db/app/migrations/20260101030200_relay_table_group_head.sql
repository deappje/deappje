-- migrate:up
SET LOCAL ROLE nano_relay;
SET LOCAL search_path = nano_relay;

-- Plain table `group_head` of nanoservice `relay`. Only this
-- nanoservice's role can reach it (the Directive §2).
-- `created_at` and `updated_at` are added when the plan declares none of
-- its own; a declared one is the plan's, without the default.
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
SET LOCAL ROLE nano_relay;
SET LOCAL search_path = nano_relay;
DROP TABLE IF EXISTS group_head;
RESET ROLE;
