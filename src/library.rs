use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use walkdir::{DirEntry, WalkDir};

use crate::{
    Result,
    domain::{PaperId, PaperMetadata, PaperOverview, ProcessingStage, ProcessingStatus},
    error::Error,
    store::ArtifactStore,
};

#[derive(Clone, Debug)]
pub struct CatalogEntry {
    pub overview: PaperOverview,
    pub source_path: PathBuf,
}

#[derive(Debug)]
pub struct LibraryCatalog {
    root: PathBuf,
    entries: BTreeMap<PaperId, CatalogEntry>,
}

impl LibraryCatalog {
    pub async fn scan(root: impl Into<PathBuf>, store: &ArtifactStore) -> Result<Self> {
        let root = root.into();
        if !root.is_dir() {
            return Err(Error::InvalidRequest(format!(
                "library path is not a directory: {}",
                root.display()
            )));
        }

        let mut paths = WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_entry(visible_entry)
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_type().is_file() && is_pdf(entry.path()))
            .map(DirEntry::into_path)
            .collect::<Vec<_>>();
        paths.sort();

        let mut entries = BTreeMap::new();
        for path in paths {
            let relative = path.strip_prefix(&root).map_err(|error| {
                Error::InvalidRequest(format!("could not relativize {}: {error}", path.display()))
            })?;
            let id = PaperId::from_relative_path(relative);
            let analysis = store.load_analysis(&id).await?;
            let extraction = store.load_extraction(&id).await?;
            let mut metadata = metadata_from_filename(&path);
            if let Some(extracted) = &extraction {
                merge_metadata(&mut metadata, &extracted.metadata);
            }
            let status = if analysis.is_some() {
                ProcessingStatus::Ready
            } else if extraction.is_some() {
                ProcessingStatus::Extracted
            } else {
                ProcessingStatus::Discovered
            };
            let overview = PaperOverview {
                id: id.clone(),
                metadata,
                relative_path: relative.to_string_lossy().replace('\\', "/"),
                status,
                analyzed_at: analysis.as_ref().map(|value| value.generated_at),
                one_line_summary: analysis.as_ref().map(|value| value.thesis.clone()),
            };
            entries.insert(
                id,
                CatalogEntry {
                    overview,
                    source_path: path,
                },
            );
        }
        Ok(Self { root, entries })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn overviews(&self) -> Vec<PaperOverview> {
        let mut papers = self
            .entries
            .values()
            .map(|entry| entry.overview.clone())
            .collect::<Vec<_>>();
        papers.sort_by(|left, right| {
            left.metadata
                .title
                .to_lowercase()
                .cmp(&right.metadata.title.to_lowercase())
        });
        papers
    }

    #[must_use]
    pub fn get(&self, id: &PaperId) -> Option<&CatalogEntry> {
        self.entries.get(id)
    }

    pub fn get_mut(&mut self, id: &PaperId) -> Option<&mut CatalogEntry> {
        self.entries.get_mut(id)
    }

    pub fn replace_with(&mut self, mut replacement: Self) {
        for (id, entry) in &mut replacement.entries {
            if let Some(existing) = self.entries.get(id)
                && matches!(
                    existing.overview.status,
                    ProcessingStatus::Queued { .. }
                        | ProcessingStatus::Extracting
                        | ProcessingStatus::Analyzing { .. }
                )
            {
                entry.overview.status = existing.overview.status.clone();
            }
        }
        *self = replacement;
    }

    pub fn mark_failure(&mut self, id: &PaperId, stage: ProcessingStage, error: &Error) {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.overview.status = ProcessingStatus::Failed {
                stage,
                message: error.to_string(),
                retryable: !matches!(error, Error::EmptyExtraction(_)),
            };
        }
    }
}

fn visible_entry(entry: &DirEntry) -> bool {
    entry.depth() == 0
        || entry
            .file_name()
            .to_str()
            .is_none_or(|name| !name.starts_with('.'))
}

fn is_pdf(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
}

fn metadata_from_filename(path: &Path) -> PaperMetadata {
    let mut stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("Untitled paper")
        .trim()
        .to_owned();
    while stem.to_ascii_lowercase().ends_with(".pdf") {
        stem.truncate(stem.len().saturating_sub(4));
    }
    let pieces = stem
        .split(" - ")
        .map(str::trim)
        .filter(|piece| !piece.is_empty())
        .collect::<Vec<_>>();

    let (authors, year, title) = match pieces.as_slice() {
        [author, year, title @ ..] if parse_year(year).is_some() => (
            vec![(*author).to_owned()],
            parse_year(year),
            title.join(" — "),
        ),
        [date, title @ ..] if year_from_date(date).is_some() => {
            (Vec::new(), year_from_date(date), title.join(" — "))
        }
        [author, title @ ..] if !title.is_empty() => {
            (vec![(*author).to_owned()], None, title.join(" — "))
        }
        _ => (Vec::new(), None, stem),
    };

    PaperMetadata {
        title,
        authors,
        year,
        page_count: None,
        subject: None,
    }
}

fn parse_year(value: &str) -> Option<u16> {
    (value.len() == 4)
        .then(|| value.parse::<u16>().ok())
        .flatten()
        .filter(|year| (1400..=2200).contains(year))
}

fn year_from_date(value: &str) -> Option<u16> {
    value.get(..4).and_then(parse_year)
}

fn merge_metadata(target: &mut PaperMetadata, extracted: &PaperMetadata) {
    if !extracted.title.trim().is_empty() {
        target.title.clone_from(&extracted.title);
    }
    if !extracted.authors.is_empty() {
        target.authors.clone_from(&extracted.authors);
    }
    target.year = extracted.year.or(target.year);
    target.page_count = extracted.page_count.or(target.page_count);
    target.subject.clone_from(&extracted.subject);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_conventional_vault_filename() {
        let metadata = metadata_from_filename(Path::new(
            "Autor, Dorn, and Hanson - 2013 - The China Syndrome.pdf",
        ));
        assert_eq!(metadata.year, Some(2013));
        assert_eq!(metadata.authors, ["Autor, Dorn, and Hanson"]);
        assert_eq!(metadata.title, "The China Syndrome");
    }

    #[test]
    fn parses_date_first_filename() {
        let metadata =
            metadata_from_filename(Path::new("2026-01-14 - Anthropic Economic Index.pdf"));
        assert_eq!(metadata.year, Some(2026));
        assert!(metadata.authors.is_empty());
        assert_eq!(metadata.title, "Anthropic Economic Index");
    }
}
