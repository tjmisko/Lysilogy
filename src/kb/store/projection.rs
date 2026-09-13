use super::{
    Result, StoreError, digest,
    journal::{self, EntityProjection, Event, Record},
    read_entity,
};
use crate::kb::{AcquisitionState, Decision, Person, PersonId, Work, WorkId, WorkType};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub(super) fn apply(connection: &Connection, directory: &Path, record: &Record) -> Result<()> {
    match &record.event {
        Event::Allocate { id, kind, origin } => allocate(connection, id, kind, origin),
        Event::Admit { revision } => admit(connection, directory, revision, record.sequence),
        Event::Decision { decision } => decision(connection, directory, decision),
    }
}

fn allocate(connection: &Connection, id: &str, kind: &str, origin: &str) -> Result<()> {
    journal::validate_token(origin, "allocation origin", 4096)?;
    match kind {
        "work" => {
            id.parse::<WorkId>().map_err(StoreError::Invalid)?;
        }
        "person" => {
            id.parse::<PersonId>().map_err(StoreError::Invalid)?;
        }
        _ => return Err(StoreError::Invalid("unknown allocation kind".into())),
    }
    connection.execute(
        "INSERT INTO allocations VALUES(?1,?2,?3)",
        params![id, kind, origin],
    )?;
    if kind == "work" {
        upsert_work(connection, &empty_work(id)?)?;
    } else {
        upsert_person(connection, &empty_person(id)?)?;
    }
    Ok(())
}

fn empty_work(id: &str) -> Result<Work> {
    Ok(Work {
        id: id.parse().map_err(StoreError::Invalid)?,
        identifiers: BTreeSet::new(),
        title: None,
        title_key: None,
        year: None,
        venue: None,
        kind: WorkType::Unknown,
        versions: vec![],
        local_copies: vec![],
        acquisition: AcquisitionState::Unresolved,
    })
}
fn empty_person(id: &str) -> Result<Person> {
    Ok(Person {
        id: id.parse().map_err(StoreError::Invalid)?,
        display_name: String::new(),
        family_name: None,
        given_names: vec![],
        particles: vec![],
        suffix: None,
        name_variants: vec![],
        identifiers: BTreeSet::new(),
    })
}

fn admit(connection: &Connection, directory: &Path, revision: &str, sequence: u64) -> Result<()> {
    let admitted = journal::admission(directory, revision)?;
    let observation = &admitted.observation;
    journal::validate_token(&observation.id, "observation ID", 4096)?;
    let origin = serde_json::to_string(&observation.source)?;
    let supplied_id = resolve(connection, admitted.entity.id())?;
    let existing: Option<(String, String)> = connection
        .query_row(
            "SELECT entity_id,origin FROM observations WHERE id=?1",
            [&observation.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let id = if let Some((bound, previous_origin)) = existing {
        if previous_origin != origin {
            return Err(StoreError::Invalid("observation origin changed".into()));
        }
        // A canonical split changes this binding independently of a producer's
        // cached entity ID. Later payload revisions retain the decided binding.
        if bound.starts_with('W') != supplied_id.starts_with('W') {
            return Err(StoreError::Invalid(
                "observation entity kind changed".into(),
            ));
        }
        bound
    } else {
        supplied_id
    };
    let source_entity = admitted.entity.id().to_owned();
    connection.execute("INSERT INTO observations VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,body=excluded.body,admitted_sequence=excluded.admitted_sequence", params![observation.id,id,origin,revision,serde_json::to_string(observation)?,sequence])?;
    match admitted.entity {
        EntityProjection::Work(mut work) => {
            work.id = id.parse().map_err(StoreError::Invalid)?;
            upsert_work(connection, &work)?;
        }
        EntityProjection::Person(mut person) => {
            person.id = id.parse().map_err(StoreError::Invalid)?;
            upsert_person(connection, &person)?;
        }
    }
    connection.execute(
        "DELETE FROM authorships WHERE observation_id=?1",
        [&observation.id],
    )?;
    let old_edges = connection
        .prepare("SELECT DISTINCT citing,cited FROM citation_evidence WHERE observation_id=?1")?
        .query_map([&observation.id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    connection.execute(
        "DELETE FROM citation_evidence WHERE observation_id=?1",
        [&observation.id],
    )?;
    for author in admitted.authorships {
        let work = bound_edge_id(connection, author.work_id.as_str(), &source_entity, &id)?;
        let person = bound_edge_id(connection, author.person_id.as_str(), &source_entity, &id)?;
        connection.execute(
            "INSERT INTO authorships VALUES(?1,?2,?3,?4,?5)",
            params![
                work,
                person,
                author.position,
                author.raw_name,
                observation.id
            ],
        )?;
    }
    for citation in admitted.citations {
        if citation.evidence.is_empty() {
            return Err(StoreError::Invalid("citation needs source evidence".into()));
        }
        let citing = bound_edge_id(connection, citation.citing.as_str(), &source_entity, &id)?;
        let cited = bound_edge_id(connection, citation.cited.as_str(), &source_entity, &id)?;
        connection.execute(
            "INSERT OR IGNORE INTO citations VALUES(?1,?2)",
            params![citing, cited],
        )?;
        for evidence in citation.evidence {
            let body = serde_json::to_string(&evidence)?;
            connection.execute(
                "INSERT OR IGNORE INTO citation_evidence VALUES(?1,?2,?3,?4,?5)",
                params![citing, cited, digest(body.as_bytes()), observation.id, body],
            )?;
        }
    }
    for (citing, cited) in old_edges {
        connection.execute("DELETE FROM citations WHERE citing=?1 AND cited=?2 AND NOT EXISTS(SELECT 1 FROM citation_evidence WHERE citing=?1 AND cited=?2)",params![citing,cited])?;
    }
    Ok(())
}

pub(super) fn upsert_work(connection: &Connection, work: &Work) -> Result<()> {
    for identifier in &work.identifiers {
        let parsed = crate::kb::Identifier::parse(&identifier.key())
            .map_err(|error| StoreError::Invalid(error.message))?;
        if parsed != *identifier {
            return Err(StoreError::Invalid("noncanonical Work identifier".into()));
        }
    }
    let title = work.title.as_deref().unwrap_or("");
    let key = work.title_key.as_deref().unwrap_or(title);
    connection.execute("INSERT INTO works VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET title=excluded.title,title_key=excluded.title_key,body=excluded.body", params![work.id.as_str(),title,key,serde_json::to_string(work)?])?;
    connection.execute(
        "DELETE FROM local_copies WHERE work_id=?1",
        [work.id.as_str()],
    )?;
    connection.execute(
        "DELETE FROM work_versions WHERE work_id=?1",
        [work.id.as_str()],
    )?;
    for version in &work.versions {
        journal::validate_token(&version.id, "version ID", 128)?;
        connection.execute(
            "INSERT INTO work_versions VALUES(?1,?2,?3)",
            params![
                work.id.as_str(),
                version.id,
                serde_json::to_string(version)?
            ],
        )?;
    }
    for copy in &work.local_copies {
        if copy.content_hash.len() != 64
            || !copy
                .content_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(StoreError::Invalid("invalid local-copy SHA-256".into()));
        }
        if !copy
            .paper_id
            .as_str()
            .parse::<crate::domain::PaperId>()
            .is_ok_and(|parsed| parsed == copy.paper_id)
        {
            return Err(StoreError::Invalid("invalid local-copy PaperId".into()));
        }
        connection.execute(
            "INSERT INTO local_copies VALUES(?1,?2,?3,?4,?5)",
            params![
                work.id.as_str(),
                copy.paper_id.as_str(),
                copy.content_hash,
                copy.version_id,
                serde_json::to_string(copy)?
            ],
        )?;
    }
    Ok(())
}

fn upsert_person(connection: &Connection, person: &Person) -> Result<()> {
    connection.execute("INSERT INTO persons VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET name=excluded.name,body=excluded.body",params![person.id.as_str(),person.display_name,serde_json::to_string(person)?])?;
    Ok(())
}

pub(super) fn resolve(connection: &Connection, id: &str) -> Result<String> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM allocations WHERE id=?1)",
        [id],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(StoreError::Invalid(format!("unallocated entity {id}")));
    }
    let mut seen = BTreeSet::new();
    let mut current = id.to_owned();
    while seen.insert(current.clone()) {
        let next: Option<String> = connection
            .query_row(
                "SELECT surviving FROM aliases WHERE absorbed=?1",
                [&current],
                |row| row.get(0),
            )
            .optional()?;
        match next {
            Some(next) => current = next,
            None => return Ok(current),
        }
    }
    Err(StoreError::Invalid("alias cycle".into()))
}

fn decision(connection: &Connection, directory: &Path, decision: &Decision) -> Result<()> {
    journal::validate_token(&decision.id, "decision ID", 128)?;
    journal::validate_token(&decision.rationale, "decision rationale", 16384)?;
    let action = serde_json::to_value(&decision.action)?;
    let kind = action["entity"]
        .as_str()
        .ok_or_else(|| StoreError::Invalid("decision entity missing".into()))?;
    // Convert the strongly typed ID forms into one internal operation.
    let value = &action["decision"];
    let get = |key: &str| -> Result<&str> {
        value[key]
            .as_str()
            .ok_or_else(|| StoreError::Invalid(format!("decision lacks {key}")))
    };
    match get("action")? {
        "merge" | "automatic_merge" => {
            if get("action")? == "automatic_merge" {
                let score = value["score"]
                    .as_f64()
                    .ok_or_else(|| StoreError::Invalid("automatic merge score missing".into()))?;
                if !(0.0..=1.0).contains(&score) {
                    return Err(StoreError::Invalid(
                        "automatic merge score out of bounds".into(),
                    ));
                }
                journal::validate_token(get("matcher_version")?, "matcher version", 256)?;
            }
            merge(connection, kind, get("surviving")?, get("absorbed")?)?;
        }
        "distinct" => {
            let left = resolve(connection, get("left")?)?;
            let right = resolve(connection, get("right")?)?;
            if left == right {
                return Err(StoreError::Invalid(
                    "cannot mark an entity distinct from itself".into(),
                ));
            }
            let (left, right) = if left < right {
                (left, right)
            } else {
                (right, left)
            };
            connection.execute(
                "INSERT OR IGNORE INTO distinct_entities VALUES(?1,?2)",
                params![left, right],
            )?;
        }
        "split" => split(
            connection,
            directory,
            kind,
            get("original")?,
            get("created")?,
            value["observation_ids"]
                .as_array()
                .ok_or_else(|| StoreError::Invalid("split observations missing".into()))?,
        )?,
        _ => return Err(StoreError::Invalid("unknown decision action".into())),
    }
    connection.execute(
        "INSERT INTO decisions VALUES(?1,?2)",
        params![decision.id, serde_json::to_string(decision)?],
    )?;
    Ok(())
}

fn merge(connection: &Connection, kind: &str, surviving: &str, absorbed: &str) -> Result<()> {
    let surviving = resolve(connection, surviving)?;
    let absorbed = resolve(connection, absorbed)?;
    if surviving == absorbed {
        return Err(StoreError::Invalid("merge resolves to one entity".into()));
    }
    let distinct: bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM distinct_entities WHERE (left_id=?1 AND right_id=?2) OR (left_id=?2 AND right_id=?1))",params![surviving,absorbed],|row|row.get(0))?;
    if distinct {
        return Err(StoreError::Invalid(
            "merge conflicts with a distinct decision".into(),
        ));
    }
    if kind == "work" {
        let mut keep: Work = read_entity(connection, "works", &surviving)?
            .ok_or_else(|| StoreError::Invalid("surviving Work missing".into()))?;
        let remove: Work = read_entity(connection, "works", &absorbed)?
            .ok_or_else(|| StoreError::Invalid("absorbed Work missing".into()))?;
        keep.identifiers.extend(remove.identifiers);
        keep.title = keep.title.or(remove.title);
        keep.title_key = keep.title_key.or(remove.title_key);
        keep.year = keep.year.or(remove.year);
        keep.venue = keep.venue.or(remove.venue);
        for version in remove.versions {
            if let Some(existing) = keep.versions.iter().find(|item| item.id == version.id) {
                if serde_json::to_value(existing)? != serde_json::to_value(&version)? {
                    return Err(StoreError::Invalid(
                        "merge has conflicting version IDs".into(),
                    ));
                }
            } else {
                keep.versions.push(version);
            }
        }
        for copy in remove.local_copies {
            if let Some(existing) = keep
                .local_copies
                .iter()
                .find(|item| item.paper_id == copy.paper_id)
            {
                if existing != &copy {
                    return Err(StoreError::Invalid(
                        "merge has conflicting local copies".into(),
                    ));
                }
            } else {
                keep.local_copies.push(copy);
            }
        }
        if keep.kind == WorkType::Unknown {
            keep.kind = remove.kind;
        }
        if matches!(keep.acquisition, AcquisitionState::Unresolved) {
            keep.acquisition = remove.acquisition;
        }
        upsert_work(connection, &keep)?;
        connection.execute("INSERT OR IGNORE INTO authorships SELECT ?1,person_id,position,raw_name,observation_id FROM authorships WHERE work_id=?2",params![surviving,absorbed])?;
        connection.execute("DELETE FROM authorships WHERE work_id=?1", [&absorbed])?;
        remap_edges(connection, &absorbed, &surviving)?;
        connection.execute("DELETE FROM local_copies WHERE work_id=?1", [&absorbed])?;
        connection.execute("DELETE FROM work_versions WHERE work_id=?1", [&absorbed])?;
        connection.execute("DELETE FROM works WHERE id=?1", [&absorbed])?;
    } else {
        let mut keep: Person = read_entity(connection, "persons", &surviving)?
            .ok_or_else(|| StoreError::Invalid("surviving Person missing".into()))?;
        let remove: Person = read_entity(connection, "persons", &absorbed)?
            .ok_or_else(|| StoreError::Invalid("absorbed Person missing".into()))?;
        keep.identifiers.extend(remove.identifiers);
        for variant in remove.name_variants {
            if !keep.name_variants.contains(&variant) {
                keep.name_variants.push(variant);
            }
        }
        if keep.display_name.is_empty() {
            keep.display_name = remove.display_name;
        }
        upsert_person(connection, &keep)?;
        connection.execute("INSERT OR IGNORE INTO authorships SELECT work_id,?1,position,raw_name,observation_id FROM authorships WHERE person_id=?2",params![surviving,absorbed])?;
        connection.execute("DELETE FROM authorships WHERE person_id=?1", [&absorbed])?;
        connection.execute("DELETE FROM persons WHERE id=?1", [&absorbed])?;
    }
    connection.execute(
        "UPDATE observations SET entity_id=?1 WHERE entity_id=?2",
        params![surviving, absorbed],
    )?;
    let rows = connection
        .prepare("SELECT left_id,right_id FROM distinct_entities WHERE left_id=?1 OR right_id=?1")?
        .query_map([&absorbed], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    connection.execute(
        "DELETE FROM distinct_entities WHERE left_id=?1 OR right_id=?1",
        [&absorbed],
    )?;
    for (left, right) in rows {
        let other = if left == absorbed { right } else { left };
        let (left, right) = if other < surviving {
            (&other, &surviving)
        } else {
            (&surviving, &other)
        };
        connection.execute(
            "INSERT OR IGNORE INTO distinct_entities VALUES(?1,?2)",
            params![left, right],
        )?;
    }
    connection.execute(
        "INSERT INTO aliases VALUES(?1,?2)",
        params![absorbed, surviving],
    )?;
    Ok(())
}

fn remap_edges(connection: &Connection, old: &str, new: &str) -> Result<()> {
    connection.execute("INSERT OR IGNORE INTO citations SELECT CASE WHEN citing=?1 THEN ?2 ELSE citing END,CASE WHEN cited=?1 THEN ?2 ELSE cited END FROM citations WHERE citing=?1 OR cited=?1",params![old,new])?;
    connection.execute("INSERT OR IGNORE INTO citation_evidence SELECT CASE WHEN citing=?1 THEN ?2 ELSE citing END,CASE WHEN cited=?1 THEN ?2 ELSE cited END,id,observation_id,body FROM citation_evidence WHERE citing=?1 OR cited=?1",params![old,new])?;
    connection.execute(
        "DELETE FROM citation_evidence WHERE citing=?1 OR cited=?1",
        [old],
    )?;
    connection.execute("DELETE FROM citations WHERE citing=?1 OR cited=?1", [old])?;
    Ok(())
}

fn split(
    connection: &Connection,
    directory: &Path,
    kind: &str,
    original: &str,
    created: &str,
    observations: &[Value],
) -> Result<()> {
    let original = resolve(connection, original)?;
    if observations.is_empty() {
        return Err(StoreError::Invalid("split needs observations".into()));
    }
    let origin = format!("split:{created}");
    allocate(connection, created, kind, &origin)?;
    let mut seen = BTreeSet::new();
    for observation in observations {
        let observation = observation
            .as_str()
            .ok_or_else(|| StoreError::Invalid("invalid split observation".into()))?;
        if !seen.insert(observation) {
            return Err(StoreError::Invalid("duplicate split observation".into()));
        }
        let bound: String = connection.query_row(
            "SELECT entity_id FROM observations WHERE id=?1",
            [observation],
            |row| row.get(0),
        )?;
        if bound != original {
            return Err(StoreError::Invalid(
                "split observation belongs to another entity".into(),
            ));
        }
        connection.execute(
            "UPDATE observations SET entity_id=?1 WHERE id=?2",
            params![created, observation],
        )?;
    }
    // Replay only each current observation revision with its new binding. This
    // moves source-owned authorships/citation evidence without copying unrelated
    // observations or inventing new evidence for the split entity.
    for observation in observations {
        let observation = observation.as_str().expect("validated above");
        let (revision, sequence): (String, u64) = connection.query_row(
            "SELECT revision,admitted_sequence FROM observations WHERE id=?1",
            [observation],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        admit(connection, directory, &revision, sequence)?;
    }
    refresh_entity(connection, directory, &original, kind)?;
    refresh_entity(connection, directory, created, kind)?;
    Ok(())
}

pub(super) fn checkpoint(
    connection: &Connection,
    record: &Record,
    bytes: &[u8],
    hash: &str,
    length: u64,
) -> Result<()> {
    connection.execute(
        "INSERT INTO canonical_records VALUES(?1,?2,?3)",
        params![
            record.sequence,
            hash,
            std::str::from_utf8(bytes)
                .map_err(|_| StoreError::Invalid("non-UTF8 canonical JSON".into()))?
        ],
    )?;
    connection.execute(
        "UPDATE projection_state SET sequence=?1,journal_bytes=?2,sha256=?3 WHERE singleton=1",
        params![record.sequence, length, hash],
    )?;
    Ok(())
}

pub(super) fn clear(connection: &Connection) -> Result<()> {
    connection.execute_batch("DELETE FROM citation_evidence; DELETE FROM citations; DELETE FROM authorships; DELETE FROM observations; DELETE FROM local_copies; DELETE FROM work_versions; DELETE FROM aliases; DELETE FROM distinct_entities; DELETE FROM decisions; DELETE FROM works; DELETE FROM persons; DELETE FROM allocations; DELETE FROM canonical_records; DELETE FROM reading_lists; UPDATE projection_state SET sequence=0,journal_bytes=0,sha256='' WHERE singleton=1;")?;
    Ok(())
}

pub(super) fn snapshot(connection: &Connection) -> Result<BTreeMap<String, Vec<Value>>> {
    let tables = [
        ("allocations", "id,kind,origin"),
        ("works", "id,body"),
        ("persons", "id,body"),
        (
            "observations",
            "id,entity_id,origin,revision,body,admitted_sequence",
        ),
        ("work_versions", "work_id,id,body"),
        (
            "local_copies",
            "work_id,paper_id,content_hash,version_id,body",
        ),
        (
            "authorships",
            "work_id,person_id,position,raw_name,observation_id",
        ),
        ("citations", "citing,cited"),
        ("citation_evidence", "citing,cited,id,observation_id,body"),
        ("aliases", "absorbed,surviving"),
        ("distinct_entities", "left_id,right_id"),
        ("decisions", "id,body"),
        ("canonical_records", "sequence,sha256,body"),
        ("reading_lists", "id,body"),
    ];
    let mut result = BTreeMap::new();
    for (table, columns) in tables {
        let mut statement =
            connection.prepare(&format!("SELECT {columns} FROM {table} ORDER BY {columns}"))?;
        let count = statement.column_count();
        let rows = statement
            .query_map([], |row| {
                let mut values = Vec::new();
                for index in 0..count {
                    values.push(match row.get_ref(index)? {
                        rusqlite::types::ValueRef::Null => Value::Null,
                        rusqlite::types::ValueRef::Integer(value) => json!(value),
                        rusqlite::types::ValueRef::Real(value) => json!(value),
                        rusqlite::types::ValueRef::Text(value) => {
                            json!(String::from_utf8_lossy(value))
                        }
                        rusqlite::types::ValueRef::Blob(value) => json!(super::hex(value)),
                    });
                }
                Ok(Value::Array(values))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        result.insert(table.into(), rows);
    }
    Ok(result)
}

fn bound_edge_id(connection: &Connection, id: &str, source: &str, bound: &str) -> Result<String> {
    if id == source {
        Ok(bound.into())
    } else {
        resolve(connection, id)
    }
}

fn refresh_entity(connection: &Connection, directory: &Path, id: &str, kind: &str) -> Result<()> {
    let revision:Option<String>=connection.query_row(
        "SELECT revision FROM observations WHERE entity_id=?1 ORDER BY admitted_sequence DESC,id DESC LIMIT 1", [id], |row|row.get(0)).optional()?;
    match revision {
        Some(revision) => match journal::admission(directory, &revision)?.entity {
            EntityProjection::Work(mut work) => {
                work.id = id.parse().map_err(StoreError::Invalid)?;
                upsert_work(connection, &work)?;
            }
            EntityProjection::Person(mut person) => {
                person.id = id.parse().map_err(StoreError::Invalid)?;
                upsert_person(connection, &person)?;
            }
        },
        None => {
            if kind == "work" {
                upsert_work(connection, &empty_work(id)?)?;
            } else {
                upsert_person(connection, &empty_person(id)?)?;
            }
        }
    }
    Ok(())
}
