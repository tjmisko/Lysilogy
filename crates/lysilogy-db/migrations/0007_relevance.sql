-- 0007: relevance, marks, and tags (Ramp D).
-- Tags and marks are private, workspace-owned curation. `work_prominence` is a
-- derived cache that is safe to truncate; it is recomputed on read.

CREATE TABLE reader.tag (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id uuid NOT NULL REFERENCES reader.workspace(id) ON DELETE CASCADE,
    name text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 48 AND name = btrim(name)),
    color text NOT NULL CHECK (color ~ '^#[0-9a-f]{6}$'),
    kind text NOT NULL DEFAULT 'topic' CHECK (kind IN ('topic', 'status')),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, id)
);

CREATE UNIQUE INDEX tag_name ON reader.tag (workspace_id, lower(name));

CREATE TABLE reader.work_tag (
    workspace_id uuid NOT NULL,
    work_id uuid NOT NULL REFERENCES graph.work(id) ON DELETE CASCADE,
    tag_id uuid NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, work_id, tag_id),
    FOREIGN KEY (workspace_id, tag_id) REFERENCES reader.tag (workspace_id, id) ON DELETE CASCADE
);

CREATE INDEX work_tag_tag ON reader.work_tag (tag_id);

CREATE TABLE reader.work_mark (
    workspace_id uuid NOT NULL REFERENCES reader.workspace(id) ON DELETE CASCADE,
    work_id uuid NOT NULL REFERENCES graph.work(id) ON DELETE CASCADE,
    mark text NOT NULL CHECK (mark IN ('pinned', 'downplayed', 'hidden')),
    note text CHECK (note IS NULL OR (char_length(note) BETWEEN 1 AND 2000)),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, work_id)
);

-- Derived cache: safe to truncate; recomputed on read when missing or stale.
CREATE TABLE reader.work_prominence (
    workspace_id uuid NOT NULL REFERENCES reader.workspace(id) ON DELETE CASCADE,
    work_id uuid NOT NULL REFERENCES graph.work(id) ON DELETE CASCADE,
    signals_version smallint NOT NULL,
    citing_works_count integer NOT NULL CHECK (citing_works_count >= 0),
    cited_works_count integer NOT NULL CHECK (cited_works_count >= 0),
    cited_by_global_count bigint CHECK (cited_by_global_count >= 0),
    display_year integer,
    local_available boolean NOT NULL,
    mention_count integer NOT NULL CHECK (mention_count >= 0),
    method_citation_count integer NOT NULL CHECK (method_citation_count >= 0),
    background_citation_count integer NOT NULL CHECK (background_citation_count >= 0),
    score double precision NOT NULL CHECK (score >= 0 AND score <= 1),
    computed_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, work_id)
);

CREATE INDEX work_prominence_score ON reader.work_prominence (workspace_id, score DESC);
