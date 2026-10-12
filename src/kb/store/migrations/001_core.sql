CREATE TABLE allocations (
    id TEXT PRIMARY KEY, kind TEXT NOT NULL CHECK(kind IN ('work', 'person')),
    origin TEXT NOT NULL, UNIQUE(kind, origin)
) STRICT;
CREATE TABLE works (
    id TEXT PRIMARY KEY REFERENCES allocations(id), title TEXT NOT NULL,
    title_key TEXT NOT NULL, body TEXT NOT NULL CHECK(json_valid(body))
) STRICT;
CREATE TABLE persons (
    id TEXT PRIMARY KEY REFERENCES allocations(id), name TEXT NOT NULL,
    body TEXT NOT NULL CHECK(json_valid(body))
) STRICT;
CREATE TABLE observations (
    id TEXT PRIMARY KEY, entity_id TEXT NOT NULL REFERENCES allocations(id),
    origin TEXT NOT NULL, revision TEXT NOT NULL,
    body TEXT NOT NULL CHECK(json_valid(body)), admitted_sequence INTEGER NOT NULL
) STRICT;
CREATE INDEX observations_entity ON observations(entity_id, id);
CREATE TABLE work_versions (
    work_id TEXT NOT NULL REFERENCES works(id), id TEXT NOT NULL,
    body TEXT NOT NULL CHECK(json_valid(body)), PRIMARY KEY(work_id, id)
) STRICT;
CREATE TABLE local_copies (
    work_id TEXT NOT NULL REFERENCES works(id), paper_id TEXT NOT NULL,
    content_hash TEXT NOT NULL, version_id TEXT,
    body TEXT NOT NULL CHECK(json_valid(body)), PRIMARY KEY(work_id, paper_id),
    FOREIGN KEY(work_id, version_id) REFERENCES work_versions(work_id, id)
) STRICT;
CREATE INDEX local_copies_hash ON local_copies(content_hash, work_id);
CREATE TABLE authorships (
    work_id TEXT NOT NULL REFERENCES works(id), person_id TEXT NOT NULL REFERENCES persons(id),
    position INTEGER NOT NULL CHECK(position >= 0), raw_name TEXT NOT NULL,
    observation_id TEXT NOT NULL REFERENCES observations(id),
    PRIMARY KEY(work_id, person_id, position, observation_id)
) STRICT;
CREATE INDEX authorships_person ON authorships(person_id, work_id);
CREATE TABLE citations (
    citing TEXT NOT NULL REFERENCES works(id), cited TEXT NOT NULL REFERENCES works(id),
    PRIMARY KEY(citing, cited)
) STRICT;
CREATE INDEX citations_incoming ON citations(cited, citing);
CREATE TABLE citation_evidence (
    citing TEXT NOT NULL, cited TEXT NOT NULL, id TEXT NOT NULL,
    observation_id TEXT NOT NULL REFERENCES observations(id),
    body TEXT NOT NULL CHECK(json_valid(body)), PRIMARY KEY(citing, cited, id, observation_id),
    FOREIGN KEY(citing, cited) REFERENCES citations(citing, cited)
) STRICT;
CREATE INDEX citation_evidence_observation ON citation_evidence(observation_id, citing, cited);
CREATE INDEX authorships_observation ON authorships(observation_id);
CREATE TABLE aliases (
    absorbed TEXT PRIMARY KEY REFERENCES allocations(id),
    surviving TEXT NOT NULL REFERENCES allocations(id), CHECK(absorbed != surviving)
) STRICT;
CREATE TABLE distinct_entities (
    left_id TEXT NOT NULL REFERENCES allocations(id), right_id TEXT NOT NULL REFERENCES allocations(id),
    PRIMARY KEY(left_id, right_id), CHECK(left_id < right_id)
) STRICT;
CREATE TABLE decisions (id TEXT PRIMARY KEY, body TEXT NOT NULL CHECK(json_valid(body))) STRICT;
CREATE TABLE canonical_records (
    sequence INTEGER PRIMARY KEY, sha256 TEXT NOT NULL, body TEXT NOT NULL CHECK(json_valid(body))
) STRICT;
CREATE TABLE reading_lists (id TEXT PRIMARY KEY, body TEXT NOT NULL CHECK(json_valid(body))) STRICT;
CREATE TABLE projection_state (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    sequence INTEGER NOT NULL, journal_bytes INTEGER NOT NULL, sha256 TEXT NOT NULL
) STRICT;
INSERT INTO projection_state VALUES(1, 0, 0, '');
