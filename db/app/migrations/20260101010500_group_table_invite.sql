-- migrate:up
SET LOCAL ROLE nano_group;
SET LOCAL search_path = nano_group;

-- Plain table `invite` of nanoservice `group`. Only this
-- nanoservice's role can reach it (the Directive §2).
CREATE TABLE invite (
    id UUID NOT NULL PRIMARY KEY,
    group_id UUID NOT NULL,
    code TEXT NOT NULL UNIQUE,
    role TEXT NOT NULL,
    created_by UUID NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    max_uses INTEGER NOT NULL,
    uses INTEGER NOT NULL,
    revoked BOOLEAN NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE INDEX idx_invite_group_id ON invite (group_id);

-- dbmate records this version in the same transaction, and the role may
-- not write the ledger.
RESET ROLE;

-- migrate:down
SET LOCAL ROLE nano_group;
SET LOCAL search_path = nano_group;
DROP TABLE IF EXISTS invite;
RESET ROLE;
