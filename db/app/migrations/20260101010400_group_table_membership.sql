-- migrate:up
SET LOCAL ROLE nano_group;
SET LOCAL search_path = nano_group;

-- Plain table `membership` of nanoservice `group`. Only this
-- nanoservice's role can reach it (the Directive §2).
-- `created_at` and `updated_at` are added when the plan declares none of
-- its own; a declared one is the plan's, without the default.
CREATE TABLE membership (
    id UUID NOT NULL PRIMARY KEY,
    group_id UUID NOT NULL,
    user_id UUID NOT NULL,
    role TEXT NOT NULL,
    joined_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE UNIQUE INDEX idx_membership_group_id_user_id ON membership (group_id, user_id);

CREATE INDEX idx_membership_user_id ON membership (user_id);

-- dbmate records this version in the same transaction, and the role may
-- not write the ledger.
RESET ROLE;

-- migrate:down
SET LOCAL ROLE nano_group;
SET LOCAL search_path = nano_group;
DROP TABLE IF EXISTS membership;
RESET ROLE;
