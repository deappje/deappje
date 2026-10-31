-- migrate:up
SET LOCAL ROLE nano_relay;
SET LOCAL search_path = nano_relay;

-- Plain table `group_head` of nanoservice `relay`. Only this
-- nanoservice's role can reach it (the Directive §2).
-- `created_at` and `updated_at` are added when the plan declares none of
-- its own; a declared one is the plan's, without the default.
-- `restored_through_seq` and `restored_at` are the restore witness: after
-- a complete history restore from seq 1, every row at or below the seq
-- that any holder has is in the log, so a device joining a little later
-- needs no upload; eviction clears it and skips the group for a grace
-- (docs/01-deappje/02-relay.md, "History requests", "Sweeps").
CREATE TABLE group_head (
    group_id UUID NOT NULL PRIMARY KEY,
    head_seq BIGINT NOT NULL,
    evicted_through_seq BIGINT NOT NULL,
    restored_through_seq BIGINT NOT NULL,
    restored_at TIMESTAMPTZ,
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
