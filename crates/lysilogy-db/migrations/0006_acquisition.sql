-- 0006: open-access acquisition state, and the `acquisition` document link method.

ALTER TABLE reader.document DROP CONSTRAINT document_work_link_method_check;
ALTER TABLE reader.document ADD CONSTRAINT document_work_link_method_check
    CHECK (work_link_method IN ('identifier', 'title_match', 'manual', 'acquisition'));

CREATE TABLE ops.acquisition (
    work_id uuid PRIMARY KEY REFERENCES graph.work(id) ON DELETE CASCADE,
    state text NOT NULL DEFAULT 'unresolved'
        CHECK (state IN ('unresolved', 'identifier_found', 'link_found',
                         'downloaded', 'mapped', 'unavailable')),
    unavailable_reason text
        CHECK (unavailable_reason IN ('no_identifier', 'no_oa_location', 'all_links_failed')),
    identifiers jsonb,
    candidates jsonb NOT NULL DEFAULT '[]',
    trials jsonb NOT NULL DEFAULT '[]',
    source text,
    pdf_url text,
    final_url text,
    sha256 text,
    bytes bigint CHECK (bytes >= 0),
    blob_id uuid REFERENCES reader.blob(id) ON DELETE SET NULL,
    document_id uuid REFERENCES reader.document(id) ON DELETE SET NULL,
    document_version_id uuid REFERENCES reader.document_version(id) ON DELETE SET NULL,
    attempts integer NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    last_error text,
    checked_at timestamptz,
    lease_until timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((state = 'unavailable') = (unavailable_reason IS NOT NULL))
);
CREATE INDEX acquisition_state ON ops.acquisition (state);
