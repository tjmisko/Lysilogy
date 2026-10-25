-- The expansion ledger gains a direction (D18): references (a Work's outgoing citations) or
-- cited_by (works citing it). Each direction is cached and refreshed on its own, so the key is
-- (work_id, direction). Existing rows are all references.
ALTER TABLE ops.work_expansion
    ADD COLUMN direction text NOT NULL DEFAULT 'references'
        CHECK (direction IN ('references', 'cited_by'));

ALTER TABLE ops.work_expansion DROP CONSTRAINT work_expansion_pkey;
ALTER TABLE ops.work_expansion ADD PRIMARY KEY (work_id, direction);

-- Cited-by only ever comes from the provider; local and existing describe a references fanout.
ALTER TABLE ops.work_expansion
    ADD CONSTRAINT work_expansion_cited_by_method
        CHECK (direction = 'references' OR method IS NULL OR method = 'provider');
