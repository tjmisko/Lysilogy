-- Append-only reading history and per-paper reading position.

CREATE TABLE reader.history_event (
    id bigserial PRIMARY KEY,
    workspace_id uuid NOT NULL REFERENCES reader.workspace(id) ON DELETE CASCADE,
    occurred_at timestamptz NOT NULL,
    session_id uuid NOT NULL,
    kind text NOT NULL CHECK (kind IN (
        'paper_opened', 'paper_left', 'view_changed', 'section_focused', 'figure_viewed',
        'page_reached', 'search_run', 'question_asked', 'link_followed', 'digest_opened',
        'graph_opened', 'work_inspected', 'list_opened', 'list_changed', 'list_generated',
        'fetch_requested')),
    paper_id text,
    work_id uuid REFERENCES graph.work(id) ON DELETE SET NULL,
    list_id uuid,
    dwell_ms integer CHECK (dwell_ms IS NULL OR dwell_ms >= 0),
    detail jsonb NOT NULL DEFAULT '{}',
    client_seq integer NOT NULL,
    UNIQUE (session_id, client_seq)
);
CREATE INDEX history_recent ON reader.history_event (workspace_id, occurred_at DESC);
CREATE INDEX history_paper ON reader.history_event (paper_id, occurred_at DESC);
CREATE INDEX history_work ON reader.history_event (work_id, occurred_at DESC);

-- The list table arrives with the reading-list migration (0011). Tie the history to it
-- when it exists so this migration also applies on a database that predates lists.
DO $$
BEGIN
    IF to_regclass('reader.reading_list') IS NOT NULL THEN
        ALTER TABLE reader.history_event
            ADD CONSTRAINT history_event_list_id_fkey
            FOREIGN KEY (list_id) REFERENCES reader.reading_list(id) ON DELETE SET NULL;
    END IF;
END $$;

CREATE TABLE reader.reading_position (
    workspace_id uuid NOT NULL REFERENCES reader.workspace(id) ON DELETE CASCADE,
    paper_id text NOT NULL,
    view text NOT NULL CHECK (view IN ('pdf', 'markdown', 'overview', 'section')),
    page integer CHECK (page IS NULL OR page >= 0),
    offset_utf16 integer CHECK (offset_utf16 IS NULL OR offset_utf16 >= 0),
    section_id text,
    scroll_ratio real CHECK (scroll_ratio IS NULL OR (scroll_ratio >= 0 AND scroll_ratio <= 1)),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, paper_id)
);
