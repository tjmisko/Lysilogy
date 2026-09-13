//! Offline bridge: browser fixtures exercise the same object builder as the API.
use std::io::{self, Read};

use lysilogy::{
    domain::PaperId,
    objects::ObjectsArtifact,
    source_index::{IndexDocument, ReadingIndex},
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    index: ReadingIndex,
    generation: String,
    paper_id: PaperId,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = String::new();
    io::stdin()
        .take(8 * 1024 * 1024 + 1)
        .read_to_string(&mut input)?;
    if input.len() > 8 * 1024 * 1024 {
        return Err("fixture exceeds 8 MiB".into());
    }
    let fixture: Fixture = serde_json::from_str(&input)?;
    let document = IndexDocument {
        index: fixture.index,
        etag: fixture.generation,
    };
    println!(
        "{}",
        serde_json::to_string(&ObjectsArtifact::from_reading_index(
            &fixture.paper_id,
            &document
        ))?
    );
    Ok(())
}
