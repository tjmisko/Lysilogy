-- 0009: claim provenance (origin, extractor, confidence), curator overrides, and the
-- selected author list. Tier ranking lives in Rust (lysilogy_core::provenance).

ALTER TABLE graph.field_claim
    ADD COLUMN origin text NOT NULL DEFAULT 'library_metadata'
        CHECK (origin IN ('user', 'publisher', 'paper', 'openalex',
                          'library_metadata', 'citing_bibliography')),
    ADD COLUMN extractor text NOT NULL DEFAULT 'legacy'
        CHECK (char_length(extractor) BETWEEN 1 AND 64),
    ADD COLUMN confidence text NOT NULL DEFAULT 'heuristic'
        CHECK (confidence IN ('explicit', 'heuristic', 'model')),
    ADD COLUMN entry_id uuid REFERENCES source.bibliography_entry(id) ON DELETE SET NULL,
    ADD COLUMN retracted_at timestamptz,
    ADD CONSTRAINT field_claim_retraction_user_only
        CHECK (retracted_at IS NULL OR origin = 'user');

-- Legacy rows: provider records, local bibliographies, and the paper's catalog hints.
-- Each statement only touches rows still carrying the legacy extractor, so the
-- block can be replayed safely (the backfill test does).
UPDATE graph.field_claim c SET origin = 'openalex', extractor = 'openalex', confidence = 'explicit'
  FROM source.source_record s
 WHERE s.id = c.source_record_id AND c.extractor = 'legacy'
   AND s.kind IN ('openalex_work', 'openalex_work_summary');
UPDATE graph.field_claim c SET origin = 'citing_bibliography', extractor = 'bib-v1',
       confidence = 'heuristic',
       entry_id = (SELECT e.id FROM source.bibliography_entry e
                     JOIN graph.entry_resolution r ON r.entry_id = e.id AND r.status = 'accepted'
                    WHERE e.source_record_id = c.source_record_id
                      AND r.candidate_work_id = c.work_id
                    ORDER BY e.created_at, e.id LIMIT 1)
  FROM source.source_record s
 WHERE s.id = c.source_record_id AND c.extractor = 'legacy' AND s.kind = 'local_bibliography';
UPDATE graph.field_claim SET extractor = 'catalog-metadata'
 WHERE source_record_id IS NULL AND extractor = 'legacy';

-- New writers must classify every claim.
ALTER TABLE graph.field_claim
    ALTER COLUMN origin DROP DEFAULT,
    ALTER COLUMN extractor DROP DEFAULT,
    ALTER COLUMN confidence DROP DEFAULT;

CREATE INDEX field_claim_entry ON graph.field_claim (entry_id) WHERE entry_id IS NOT NULL;
-- At most one active curator override per Work field.
CREATE UNIQUE INDEX field_claim_active_override ON graph.field_claim (work_id, field)
    WHERE origin = 'user' AND retracted_at IS NULL;

ALTER TABLE graph.work
    ADD COLUMN display_authors text[]
        CHECK (display_authors IS NULL OR cardinality(display_authors) >= 1),
    ADD COLUMN authors_state text NOT NULL DEFAULT 'unknown'
        CHECK (authors_state IN ('known', 'unknown', 'not_checked', 'conflicting', 'not_applicable'));
