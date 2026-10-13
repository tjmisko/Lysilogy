-- 0001_init: Ramp A schema for the literature viewer.
--
-- Organizes tables into purpose-specific schemas:
--   source  — immutable evidence contributed by a source (provider or local extraction)
--   graph   — bibliographic entities, claims, and citation edges
--   reader  — workspace-owned documents and (later) paper objects
--   publish — public lists and ramps (reserved)
--   ops     — jobs and provider connections (reserved)
--
-- Conventions: uuid primary keys via gen_random_uuid(); timestamptz for time;
-- text + CHECK constraints for state fields; jsonb for variable provider values.

CREATE SCHEMA IF NOT EXISTS source;
CREATE SCHEMA IF NOT EXISTS graph;
CREATE SCHEMA IF NOT EXISTS reader;
CREATE SCHEMA IF NOT EXISTS publish;
CREATE SCHEMA IF NOT EXISTS ops;

-- ---------------------------------------------------------------------------
-- graph: bibliographic identity
-- ---------------------------------------------------------------------------

CREATE TABLE graph.work (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    kind text NOT NULL DEFAULT 'article'
        CHECK (kind IN ('article', 'book', 'chapter', 'thesis', 'report', 'dataset', 'other')),
    visibility text NOT NULL DEFAULT 'private'
        CHECK (visibility IN ('private', 'public')),
    display_title text,
    title_state text NOT NULL DEFAULT 'unknown'
        CHECK (title_state IN ('known', 'unknown', 'not_checked', 'conflicting', 'not_applicable')),
    display_year integer,
    year_state text NOT NULL DEFAULT 'unknown'
        CHECK (year_state IN ('known', 'unknown', 'not_checked', 'conflicting', 'not_applicable')),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

-- ---------------------------------------------------------------------------
-- reader: workspace-owned documents
-- ---------------------------------------------------------------------------

CREATE TABLE reader.workspace (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE reader.blob (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    scope text NOT NULL DEFAULT 'private'
        CHECK (scope IN ('private', 'public')),
    storage_key text NOT NULL UNIQUE,
    sha256 text NOT NULL,
    bytes bigint NOT NULL CHECK (bytes >= 0),
    media_type text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX blob_sha256 ON reader.blob (sha256);

CREATE TABLE reader.document (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id uuid NOT NULL REFERENCES reader.workspace(id) ON DELETE CASCADE,
    work_id uuid REFERENCES graph.work(id) ON DELETE SET NULL,
    relative_path text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX document_workspace ON reader.document (workspace_id);
CREATE INDEX document_work ON reader.document (work_id);
CREATE UNIQUE INDEX document_path ON reader.document (workspace_id, relative_path)
    WHERE relative_path IS NOT NULL;

CREATE TABLE reader.document_version (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    document_id uuid NOT NULL REFERENCES reader.document(id) ON DELETE CASCADE,
    blob_id uuid REFERENCES reader.blob(id) ON DELETE SET NULL,
    format text NOT NULL DEFAULT 'pdf',
    sha256 text NOT NULL,
    bytes bigint CHECK (bytes >= 0),
    upload_state text NOT NULL DEFAULT 'ready'
        CHECK (upload_state IN ('pending', 'ready', 'missing', 'failed')),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (document_id, sha256)
);

CREATE INDEX document_version_sha ON reader.document_version (sha256);

-- ---------------------------------------------------------------------------
-- source: immutable evidence
-- ---------------------------------------------------------------------------

CREATE TABLE source.source_record (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    kind text NOT NULL,
    source_key text NOT NULL,
    retrieved_at timestamptz NOT NULL DEFAULT now(),
    blob_hash text,
    coverage text NOT NULL DEFAULT 'unknown'
        CHECK (coverage IN ('unknown', 'partial', 'complete')),
    license_info text,
    payload jsonb,
    UNIQUE (kind, source_key)
);

CREATE TABLE source.bibliography_entry (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    citing_version_id uuid NOT NULL REFERENCES reader.document_version(id) ON DELETE CASCADE,
    entry_key text NOT NULL,
    raw_text text NOT NULL,
    page_or_locator text,
    source_record_id uuid REFERENCES source.source_record(id) ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (citing_version_id, entry_key)
);

-- ---------------------------------------------------------------------------
-- graph: claims and citation edges
-- ---------------------------------------------------------------------------

CREATE TABLE graph.work_identifier (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    work_id uuid NOT NULL REFERENCES graph.work(id) ON DELETE CASCADE,
    scheme text NOT NULL
        CHECK (scheme IN ('doi', 'arxiv', 'openalex', 'semantic_scholar', 'pmid', 'isbn', 'url')),
    value_normalized text NOT NULL,
    source_record_id uuid REFERENCES source.source_record(id) ON DELETE SET NULL,
    status text NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'conflicting', 'retracted')),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (work_id, scheme, value_normalized)
);

CREATE INDEX work_identifier_lookup ON graph.work_identifier (scheme, value_normalized);

-- At most one active claim per exact identifier, so re-ingest is idempotent.
CREATE UNIQUE INDEX work_identifier_active ON graph.work_identifier (scheme, value_normalized)
    WHERE status = 'active';

CREATE TABLE graph.work_version (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    work_id uuid NOT NULL REFERENCES graph.work(id) ON DELETE CASCADE,
    version_kind text NOT NULL DEFAULT 'unknown'
        CHECK (version_kind IN ('preprint', 'published', 'revision', 'unknown')),
    version_label text,
    published_date date,
    date_state text NOT NULL DEFAULT 'unknown'
        CHECK (date_state IN ('known', 'unknown', 'not_checked', 'conflicting', 'not_applicable')),
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX work_version_work ON graph.work_version (work_id);

CREATE TABLE graph.field_claim (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    work_id uuid NOT NULL REFERENCES graph.work(id) ON DELETE CASCADE,
    version_id uuid REFERENCES graph.work_version(id) ON DELETE SET NULL,
    field text NOT NULL
        CHECK (field IN ('title', 'year', 'author', 'venue', 'type', 'abstract', 'identifier', 'other')),
    value jsonb NOT NULL,
    state text NOT NULL DEFAULT 'known'
        CHECK (state IN ('known', 'unknown', 'not_checked', 'conflicting', 'not_applicable')),
    source_record_id uuid REFERENCES source.source_record(id) ON DELETE SET NULL,
    locator text,
    review_state text NOT NULL DEFAULT 'unreviewed'
        CHECK (review_state IN ('unreviewed', 'accepted', 'rejected', 'disputed')),
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX field_claim_work ON graph.field_claim (work_id, field);

CREATE TABLE graph.entry_resolution (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    entry_id uuid NOT NULL REFERENCES source.bibliography_entry(id) ON DELETE CASCADE,
    candidate_work_id uuid NOT NULL REFERENCES graph.work(id) ON DELETE CASCADE,
    status text NOT NULL
        CHECK (status IN ('candidate', 'accepted', 'rejected', 'ambiguous', 'withdrawn')),
    method text NOT NULL,
    confidence double precision CHECK (confidence >= 0 AND confidence <= 1),
    reviewed_by text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (entry_id, candidate_work_id)
);

-- One accepted resolution per bibliography entry.
CREATE UNIQUE INDEX entry_resolution_accepted ON graph.entry_resolution (entry_id)
    WHERE status = 'accepted';

CREATE INDEX entry_resolution_candidate ON graph.entry_resolution (candidate_work_id);

CREATE TABLE graph.citation_assertion (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    citing_work_id uuid NOT NULL REFERENCES graph.work(id) ON DELETE CASCADE,
    cited_work_id uuid NOT NULL REFERENCES graph.work(id) ON DELETE CASCADE,
    entry_id uuid REFERENCES source.bibliography_entry(id) ON DELETE SET NULL,
    source_record_id uuid REFERENCES source.source_record(id) ON DELETE SET NULL,
    status text NOT NULL DEFAULT 'asserted'
        CHECK (status IN ('asserted', 'accepted', 'rejected', 'withdrawn')),
    observed_at timestamptz NOT NULL DEFAULT now(),
    CHECK (citing_work_id <> cited_work_id)
);

CREATE INDEX citation_assertion_citing ON graph.citation_assertion (citing_work_id, cited_work_id);
CREATE INDEX citation_assertion_cited ON graph.citation_assertion (cited_work_id, citing_work_id);

CREATE TABLE graph.citation_mention (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    assertion_id uuid NOT NULL REFERENCES graph.citation_assertion(id) ON DELETE CASCADE,
    document_version_id uuid REFERENCES reader.document_version(id) ON DELETE SET NULL,
    page_or_locator text,
    quote_or_anchor text,
    text_hash text,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX citation_mention_assertion ON graph.citation_mention (assertion_id);

CREATE TABLE graph.relationship_claim (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    assertion_id uuid NOT NULL REFERENCES graph.citation_assertion(id) ON DELETE CASCADE,
    mention_id uuid REFERENCES graph.citation_mention(id) ON DELETE SET NULL,
    label text NOT NULL,
    explanation text,
    origin text NOT NULL DEFAULT 'ai'
        CHECK (origin IN ('ai', 'user', 'imported')),
    review_state text NOT NULL DEFAULT 'unreviewed'
        CHECK (review_state IN ('unreviewed', 'accepted', 'rejected', 'disputed')),
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX relationship_claim_assertion ON graph.relationship_claim (assertion_id);

CREATE TABLE graph.work_redirect (
    old_work_id uuid PRIMARY KEY REFERENCES graph.work(id) ON DELETE CASCADE,
    new_work_id uuid NOT NULL REFERENCES graph.work(id) ON DELETE CASCADE,
    reason text,
    decided_by text,
    decided_at timestamptz NOT NULL DEFAULT now(),
    CHECK (old_work_id <> new_work_id)
);

CREATE TABLE graph.resolution_decision (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    kind text NOT NULL CHECK (kind IN ('merge', 'split', 'distinct', 'correction')),
    subject_ids uuid[] NOT NULL,
    actor_id text,
    reason text,
    created_at timestamptz NOT NULL DEFAULT now()
);
