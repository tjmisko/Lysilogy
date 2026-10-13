CREATE TABLE reader.ramp (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id uuid NOT NULL REFERENCES reader.workspace(id) ON DELETE CASCADE,
    seed_work_id uuid NOT NULL REFERENCES graph.work(id) ON DELETE CASCADE,
    title text NOT NULL CHECK (char_length(title) BETWEEN 1 AND 120 AND title = btrim(title)),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, id)
);
CREATE INDEX ramp_workspace_updated ON reader.ramp (workspace_id, updated_at DESC);
CREATE INDEX ramp_seed ON reader.ramp (seed_work_id);

CREATE TABLE reader.ramp_item (
    ramp_id uuid NOT NULL REFERENCES reader.ramp(id) ON DELETE CASCADE,
    work_id uuid NOT NULL REFERENCES graph.work(id) ON DELETE CASCADE,
    position integer NOT NULL CHECK (position >= 0),
    via_work_id uuid REFERENCES graph.work(id) ON DELETE SET NULL,
    path_work_ids uuid[] NOT NULL CHECK (
        cardinality(path_work_ids) BETWEEN 1 AND 9
        AND path_work_ids[cardinality(path_work_ids)] = work_id),
    status text NOT NULL DEFAULT 'unread' CHECK (status IN ('unread', 'reading', 'read')),
    note text CHECK (note IS NULL OR char_length(note) BETWEEN 1 AND 2000),
    added_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    status_changed_at timestamptz,
    PRIMARY KEY (ramp_id, work_id),
    CONSTRAINT ramp_item_position UNIQUE (ramp_id, position) DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX ramp_item_work ON reader.ramp_item (work_id);

-- Ledger of Works whose outgoing fanout has been made known. The neighborhood itself is
-- graph.citation_assertion; deleting a row only makes the next expand rerun the flow.
CREATE TABLE ops.work_expansion (
    work_id uuid PRIMARY KEY REFERENCES graph.work(id) ON DELETE CASCADE,
    state text NOT NULL CHECK (state IN ('expanded', 'unavailable')),
    method text CHECK (method IN ('provider', 'local', 'existing')),
    unavailable_reason text CHECK (unavailable_reason IN
        ('no_provider_identifier', 'provider_not_found', 'identifier_conflict')),
    provider_key text,
    source_record_id uuid REFERENCES source.source_record(id) ON DELETE SET NULL,
    references_count integer NOT NULL DEFAULT 0 CHECK (references_count >= 0),
    new_works integer NOT NULL DEFAULT 0 CHECK (new_works >= 0),
    warnings jsonb NOT NULL DEFAULT '[]',
    attempts integer NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    last_error text,
    expanded_at timestamptz NOT NULL DEFAULT now(),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((state = 'unavailable') = (unavailable_reason IS NOT NULL)),
    CHECK ((state = 'expanded') = (method IS NOT NULL))
);
