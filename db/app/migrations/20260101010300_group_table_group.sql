-- migrate:up
SET LOCAL ROLE nano_group;
SET LOCAL search_path = nano_group;

-- Plain table `group` of nanoservice `group`. Only this
-- nanoservice's role can reach it (the Directive §2).
CREATE TABLE group (
    id UUID NOT NULL PRIMARY KEY,
    parent_group_id UUID,
    created_by UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE INDEX idx_group_parent_group_id ON group (parent_group_id);

-- dbmate records this version in the same transaction, and the role may
-- not write the ledger.
RESET ROLE;

-- migrate:down
SET LOCAL ROLE nano_group;
SET LOCAL search_path = nano_group;
DROP TABLE IF EXISTS group;
RESET ROLE;
