-- 0002: make citation assertions idempotent on their evidence source.
--
-- A canonical edge groups assertions from different sources. Each provider
-- response and each local bibliography entry may contribute at most one
-- assertion for a given (citing, cited) pair.

CREATE UNIQUE INDEX citation_assertion_source_key
    ON graph.citation_assertion (citing_work_id, cited_work_id, source_record_id)
    WHERE source_record_id IS NOT NULL;

CREATE UNIQUE INDEX citation_assertion_entry_key
    ON graph.citation_assertion (citing_work_id, cited_work_id, entry_id)
    WHERE entry_id IS NOT NULL;
