-- Reading lists generalize Ramp E ramps: one graph of Work, topic and claim nodes with typed
-- edges, per-Work reading status, and a ledger of agent generations (docs/reading-lists/spec.md).

CREATE TABLE reader.reading_list (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id uuid NOT NULL REFERENCES reader.workspace(id) ON DELETE CASCADE,
    title text NOT NULL CHECK (char_length(title) BETWEEN 1 AND 120 AND title = btrim(title)),
    description text CHECK (description IS NULL OR char_length(description) BETWEEN 1 AND 4000),
    origin text NOT NULL CHECK (origin IN ('manual', 'walk', 'generated')),
    seed_work_id uuid REFERENCES graph.work(id) ON DELETE SET NULL,
    goal text CHECK (goal IS NULL OR char_length(goal) BETWEEN 1 AND 2000),
    archived_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, id)
);
CREATE INDEX reading_list_updated ON reader.reading_list (workspace_id, updated_at DESC);
CREATE INDEX reading_list_seed ON reader.reading_list (seed_work_id) WHERE seed_work_id IS NOT NULL;

-- Provenance of agent runs; list_node points at it, so it is created first.
CREATE TABLE ops.list_generation (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    list_id uuid REFERENCES reader.reading_list(id) ON DELETE SET NULL,
    goal text NOT NULL,
    options jsonb NOT NULL,
    state text NOT NULL CHECK (state IN ('running', 'succeeded', 'failed', 'over_budget')),
    report jsonb,
    spend_usd numeric(10, 5) NOT NULL DEFAULT 0,
    model_key text,
    error text,
    created_at timestamptz NOT NULL DEFAULT now(),
    finished_at timestamptz
);

CREATE TABLE reader.list_node (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    list_id uuid NOT NULL REFERENCES reader.reading_list(id) ON DELETE CASCADE,
    kind text NOT NULL CHECK (kind IN ('work', 'topic', 'claim')),
    work_id uuid REFERENCES graph.work(id) ON DELETE CASCADE,
    label text CHECK (label IS NULL OR char_length(label) BETWEEN 1 AND 80),
    summary text CHECK (summary IS NULL OR char_length(summary) BETWEEN 1 AND 2000),
    claim jsonb,
    claim_state text CHECK (claim_state IN ('unverified', 'ambiguous', 'contradicted', 'pending')),
    candidates jsonb NOT NULL DEFAULT '[]',
    position integer NOT NULL CHECK (position >= 0),
    note text CHECK (note IS NULL OR char_length(note) BETWEEN 1 AND 2000),
    rationale text CHECK (rationale IS NULL OR char_length(rationale) BETWEEN 1 AND 2000),
    context_links jsonb NOT NULL DEFAULT '[]',
    source text NOT NULL CHECK (source IN ('user', 'walk', 'citation', 'agent')),
    review text NOT NULL DEFAULT 'accepted' CHECK (review IN ('accepted', 'proposed')),
    via_work_id uuid REFERENCES graph.work(id) ON DELETE SET NULL,
    path_work_ids uuid[],
    generation_id uuid REFERENCES ops.list_generation(id) ON DELETE SET NULL,
    added_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((kind = 'work') = (work_id IS NOT NULL)),
    CHECK ((kind = 'topic') = (label IS NOT NULL)),
    CHECK ((kind = 'claim') = (claim IS NOT NULL AND claim_state IS NOT NULL)),
    CONSTRAINT list_node_position UNIQUE (list_id, position) DEFERRABLE INITIALLY DEFERRED
);
CREATE UNIQUE INDEX list_node_work ON reader.list_node (list_id, work_id) WHERE kind = 'work';
CREATE UNIQUE INDEX list_node_topic ON reader.list_node (list_id, lower(label)) WHERE kind = 'topic';
CREATE INDEX list_node_by_work ON reader.list_node (work_id) WHERE work_id IS NOT NULL;

CREATE TABLE reader.list_edge (
    list_id uuid NOT NULL REFERENCES reader.reading_list(id) ON DELETE CASCADE,
    from_node uuid NOT NULL REFERENCES reader.list_node(id) ON DELETE CASCADE,
    to_node uuid NOT NULL REFERENCES reader.list_node(id) ON DELETE CASCADE,
    kind text NOT NULL CHECK (kind IN ('prerequisite', 'covers', 'subtopic')),
    source text NOT NULL CHECK (source IN ('user', 'agent')),
    review text NOT NULL DEFAULT 'accepted' CHECK (review IN ('accepted', 'proposed')),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (from_node, to_node, kind),
    CHECK (from_node <> to_node)
);
CREATE INDEX list_edge_list ON reader.list_edge (list_id);
CREATE INDEX list_edge_to ON reader.list_edge (to_node);

CREATE TABLE reader.work_reading_state (
    workspace_id uuid NOT NULL REFERENCES reader.workspace(id) ON DELETE CASCADE,
    work_id uuid NOT NULL REFERENCES graph.work(id) ON DELETE CASCADE,
    status text NOT NULL CHECK (status IN ('unread', 'reading', 'read')),
    changed_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, work_id)
);

-- Data move from Ramp E. Ramp IDs, item positions, paths, notes and timestamps are kept.
INSERT INTO reader.reading_list
    (id, workspace_id, title, origin, seed_work_id, created_at, updated_at)
SELECT id, workspace_id, title, 'walk', seed_work_id, created_at, updated_at
FROM reader.ramp;

INSERT INTO reader.list_node
    (list_id, kind, work_id, position, note, source, via_work_id, path_work_ids,
     added_at, updated_at)
SELECT ramp_id, 'work', work_id, position, note, 'walk', via_work_id, path_work_ids,
       added_at, updated_at
FROM reader.ramp_item;

-- Reading status belongs to the Work: the furthest status any ramp reached (read > reading >
-- unread), stamped with the latest change.
INSERT INTO reader.work_reading_state (workspace_id, work_id, status, changed_at)
SELECT r.workspace_id, i.work_id,
       CASE max(CASE i.status WHEN 'read' THEN 2 WHEN 'reading' THEN 1 ELSE 0 END)
           WHEN 2 THEN 'read' WHEN 1 THEN 'reading' ELSE 'unread' END,
       coalesce(max(i.status_changed_at), max(i.updated_at), now())
FROM reader.ramp_item i
JOIN reader.ramp r ON r.id = i.ramp_id
GROUP BY r.workspace_id, i.work_id;

DROP TABLE reader.ramp_item;
DROP TABLE reader.ramp;
