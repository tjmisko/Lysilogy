-- Grounded relationship labels (Ramp B, U5).
-- `label` holds the relation code and `explanation` holds the reason.
ALTER TABLE graph.relationship_claim
    ADD COLUMN quote text,
    ADD COLUMN model text,
    ADD COLUMN source_record_id uuid REFERENCES source.source_record(id) ON DELETE SET NULL,
    ADD COLUMN superseded_by uuid REFERENCES graph.relationship_claim(id) ON DELETE SET NULL,
    ADD COLUMN updated_at timestamptz NOT NULL DEFAULT now();

CREATE UNIQUE INDEX relationship_claim_generation
    ON graph.relationship_claim (assertion_id, source_record_id)
    WHERE source_record_id IS NOT NULL;

CREATE INDEX relationship_claim_current
    ON graph.relationship_claim (assertion_id) WHERE superseded_by IS NULL;
