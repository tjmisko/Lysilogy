CREATE TABLE entity_assertions (
    entity_id TEXT NOT NULL REFERENCES allocations(id),
    observation_id TEXT PRIMARY KEY REFERENCES observations(id),
    revision TEXT NOT NULL, body TEXT NOT NULL CHECK(json_valid(body))
) STRICT;
CREATE INDEX entity_assertions_entity ON entity_assertions(entity_id, observation_id);
ALTER TABLE projection_state ADD COLUMN journal_stamp TEXT NOT NULL DEFAULT '';
CREATE TABLE entity_identifiers (
    entity_id TEXT NOT NULL REFERENCES allocations(id),
    identifier TEXT NOT NULL, PRIMARY KEY(entity_id, identifier)
) STRICT;
CREATE INDEX entity_identifiers_lookup ON entity_identifiers(identifier, entity_id);
ALTER TABLE projection_state ADD COLUMN projection_version INTEGER NOT NULL DEFAULT 0;
