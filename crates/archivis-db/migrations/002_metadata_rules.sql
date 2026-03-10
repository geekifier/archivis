-- Metadata rules: extensible policy engine for import/resolution decisions.
--
-- Each rule matches on a `rule_type` + `match_value` pair and produces an
-- `outcome` when matched. The first outcome is `trust_metadata`, which marks
-- books from trusted publishers as `Identified` at import time and skips
-- provider resolution.

CREATE TABLE metadata_rules (
    id          TEXT    PRIMARY KEY NOT NULL,
    rule_type   TEXT    NOT NULL CHECK (rule_type IN ('publisher')),
    match_value TEXT    NOT NULL,
    match_mode  TEXT    NOT NULL DEFAULT 'exact' CHECK (match_mode IN ('exact', 'contains')),
    outcome     TEXT    NOT NULL DEFAULT 'trust_metadata' CHECK (outcome IN ('trust_metadata')),
    enabled     INTEGER NOT NULL DEFAULT 1,
    builtin     INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE UNIQUE INDEX idx_metadata_rules_type_value
    ON metadata_rules(rule_type, match_value COLLATE NOCASE);

-- Pre-seed builtin publisher rules for well-known community publishers.
INSERT INTO metadata_rules (id, rule_type, match_value, match_mode, outcome, enabled, builtin)
VALUES
    ('00000000-0000-0000-0000-000000000001', 'publisher', 'Standard Ebooks', 'exact', 'trust_metadata', 1, 1),
    ('00000000-0000-0000-0000-000000000002', 'publisher', 'Global Grey Ebooks', 'exact', 'trust_metadata', 1, 1),
    ('00000000-0000-0000-0000-000000000003', 'publisher', 'Project Gutenberg', 'contains', 'trust_metadata', 1, 1);
