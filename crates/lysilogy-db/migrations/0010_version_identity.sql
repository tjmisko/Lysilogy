-- 0010: separate version identifiers from Work anchors and link held documents
-- to a manifestation.
--
-- DOI, arXiv, PMID, ISBN and URL name a version. OpenAlex and Semantic Scholar
-- ids name a Work. Existing version-level identifiers are moved to a Work's
-- first version before the Work table is constrained.

CREATE TABLE graph.work_version_identifier (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    version_id uuid NOT NULL REFERENCES graph.work_version(id) ON DELETE CASCADE,
    scheme text NOT NULL
        CHECK (scheme IN ('doi', 'arxiv', 'pmid', 'isbn', 'url')),
    value_normalized text NOT NULL,
    source_record_id uuid REFERENCES source.source_record(id) ON DELETE SET NULL,
    status text NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'conflicting', 'retracted')),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (version_id, scheme, value_normalized)
);

CREATE INDEX work_version_identifier_lookup
    ON graph.work_version_identifier (scheme, value_normalized);

CREATE UNIQUE INDEX work_version_identifier_active
    ON graph.work_version_identifier (scheme, value_normalized)
    WHERE status = 'active';

-- Every Work that only has version-level identifiers gets a version to hold them.
INSERT INTO graph.work_version (work_id, version_kind, version_label)
SELECT i.work_id,
       CASE
           WHEN bool_or(i.scheme = 'arxiv') THEN 'preprint'
           WHEN bool_or(i.scheme = 'doi') THEN 'published'
           ELSE 'unknown'
       END,
       'migrated'
FROM graph.work_identifier i
WHERE i.scheme IN ('doi', 'arxiv', 'pmid', 'isbn', 'url')
  AND NOT EXISTS (
      SELECT 1 FROM graph.work_version v WHERE v.work_id = i.work_id
  )
GROUP BY i.work_id;

-- Move version-level identifiers to the Work's first version.
INSERT INTO graph.work_version_identifier
    (version_id, scheme, value_normalized, source_record_id, status)
SELECT (
           SELECT v.id FROM graph.work_version v
           WHERE v.work_id = i.work_id
           ORDER BY v.created_at, v.id
           LIMIT 1
       ),
       i.scheme,
       i.value_normalized,
       i.source_record_id,
       i.status
FROM graph.work_identifier i
WHERE i.scheme IN ('doi', 'arxiv', 'pmid', 'isbn', 'url');

DELETE FROM graph.work_identifier
WHERE scheme IN ('doi', 'arxiv', 'pmid', 'isbn', 'url');

ALTER TABLE graph.work_identifier
    ADD CONSTRAINT work_identifier_work_level
    CHECK (scheme IN ('openalex', 'semantic_scholar'));

ALTER TABLE reader.document
    ADD COLUMN work_version_id uuid REFERENCES graph.work_version(id) ON DELETE SET NULL;

CREATE INDEX document_work_version ON reader.document (work_version_id);
