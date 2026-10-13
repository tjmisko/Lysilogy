-- 0003: record how a document came to be linked to a Work.

ALTER TABLE reader.document
    ADD COLUMN work_link_method text
        CHECK (work_link_method IN ('identifier', 'title_match', 'manual')),
    ADD COLUMN work_link_source_record_id uuid
        REFERENCES source.source_record(id) ON DELETE SET NULL,
    ADD COLUMN work_link_detail text,
    ADD COLUMN work_linked_at timestamptz;
