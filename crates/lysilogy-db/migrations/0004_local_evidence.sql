-- 0004: local evidence (bibliography entries, in-text mentions) and title keys.

ALTER TABLE graph.work ADD COLUMN title_key text;
CREATE INDEX work_title_key ON graph.work (title_key) WHERE title_key IS NOT NULL;

ALTER TABLE source.bibliography_entry
    ADD COLUMN ordinal integer CHECK (ordinal >= 0),
    ADD COLUMN printed_key text,
    ADD COLUMN fields jsonb NOT NULL DEFAULT '{}'::jsonb,   -- BibliographicFields
    ADD COLUMN page integer,
    ADD COLUMN text_start integer,
    ADD COLUMN text_end integer,
    ADD COLUMN member_spans jsonb,                           -- [{start,end}]
    ADD COLUMN region jsonb;                                 -- {x_min,y_min,x_max,y_max}
CREATE INDEX bibliography_entry_record ON source.bibliography_entry (source_record_id);

ALTER TABLE graph.citation_mention
    ADD COLUMN entry_id uuid REFERENCES source.bibliography_entry(id) ON DELETE CASCADE,
    ADD COLUMN style text CHECK (style IN ('numeric', 'superscript', 'author_year')),
    ADD COLUMN marker_text text,
    ADD COLUMN page integer,
    ADD COLUMN marker_start integer,
    ADD COLUMN marker_end integer,
    ADD COLUMN passage_start integer,
    ADD COLUMN passage_end integer,
    ADD COLUMN rects jsonb;
-- quote_or_anchor holds the whitespace-collapsed passage; text_hash = sha256(passage);
-- page_or_locator = 'p.<page>'.
CREATE UNIQUE INDEX citation_mention_occurrence
    ON graph.citation_mention (entry_id, marker_start, page) WHERE entry_id IS NOT NULL;
CREATE INDEX citation_mention_entry ON graph.citation_mention (entry_id);
CREATE INDEX citation_assertion_entry ON graph.citation_assertion (entry_id)
    WHERE entry_id IS NOT NULL;
