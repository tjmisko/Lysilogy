-- Edge labels from more than one path (docs/edge-labels.md).
-- `label_path` records which path produced a claim: the core reading's
-- admitted relations (`core_relation`), the dedicated per-edge labeler
-- (`edge_labeler`), or a person (`user`). `engagement` is the labeler's
-- judgement of how much the citing paper does with the work. Both stay null
-- on rows written before this migration that cannot be classified, and
-- readers treat a null path as `edge_labeler` for an AI claim.
ALTER TABLE graph.relationship_claim
    ADD COLUMN label_path text
        CHECK (label_path IS NULL OR label_path IN ('core_relation', 'edge_labeler', 'user')),
    ADD COLUMN engagement text
        CHECK (engagement IS NULL OR engagement IN ('substantive', 'perfunctory', 'not_shown')),
    ADD COLUMN prompt_version integer;

UPDATE graph.relationship_claim SET label_path = 'user'
    WHERE origin = 'user' AND label_path IS NULL;

-- Every AI claim before this migration came from the first labeler prompt.
UPDATE graph.relationship_claim c SET label_path = 'edge_labeler', prompt_version = 1
    FROM source.source_record r
    WHERE c.origin = 'ai' AND c.label_path IS NULL
      AND r.id = c.source_record_id AND r.kind = 'relationship_label';
