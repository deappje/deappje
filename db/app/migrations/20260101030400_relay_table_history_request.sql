-- migrate:up
SET LOCAL ROLE nano_relay;
SET LOCAL search_path = nano_relay;

-- Plain table `history_request` of nanoservice `relay`. Only this
-- nanoservice's role can reach it (the Directive §2).
-- `created_at` and `updated_at` are added when the plan declares none of
-- its own; a declared one is the plan's, without the default.
-- `through_seq` is the last seq a holder pushed back so far (`from_seq - 1`
-- before any, `to_seq` once complete) and `completed_at` marks a complete
-- request: the row stays for the requester to read until the sweep drops
-- it, so a lost wake-up costs nothing (docs/01-deappje/02-relay.md).
CREATE TABLE history_request (
    id UUID NOT NULL PRIMARY KEY,
    group_id UUID NOT NULL,
    requester_device UUID NOT NULL,
    from_seq BIGINT NOT NULL,
    to_seq BIGINT NOT NULL,
    object_id UUID,
    claimed_by UUID,
    claimed_at TIMESTAMPTZ,
    through_seq BIGINT NOT NULL,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE INDEX idx_history_request_group_id ON history_request (group_id);

-- dbmate records this version in the same transaction, and the role may
-- not write the ledger.
RESET ROLE;

-- migrate:down
SET LOCAL ROLE nano_relay;
SET LOCAL search_path = nano_relay;
DROP TABLE IF EXISTS history_request;
RESET ROLE;
