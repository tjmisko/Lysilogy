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

#[derive(Deserialize)]
#[serde(untagged)]
enum Input {
    Objects(Box<Fixture>),
    Fields { parse_entries: Vec<String> },
    Titles { normalize_titles: Vec<String> },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = String::new();
    io::stdin()
        .take(8 * 1024 * 1024 + 1)
        .read_to_string(&mut input)?;
    if input.len() > 8 * 1024 * 1024 {
        return Err("fixture exceeds 8 MiB".into());
    }
    let output = match serde_json::from_str::<Input>(&input)? {
        Input::Objects(fixture) => serde_json::to_value(ObjectsArtifact::from_reading_index(
            &fixture.paper_id,
            &IndexDocument {
                index: fixture.index,
                etag: fixture.generation,
            },
        ))?,
        Input::Fields { parse_entries } => serde_json::to_value(
            parse_entries
                .iter()
                .map(|raw| lysilogy::objects::bibliography::parse_fields(raw, None))
                .collect::<Vec<_>>(),
        )?,
        Input::Titles { normalize_titles } => serde_json::to_value(
            normalize_titles
                .iter()
                .map(|title| lysilogy::kb::titles::title_key(title))
                .collect::<Vec<_>>(),
        )?,
    };
    println!("{output}");
    Ok(())
}
