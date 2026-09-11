use std::{collections::HashSet, future::Future, time::Duration};

use chrono::Utc;
use reqwest::{
    Url,
    header::{ACCEPT, LOCATION},
};
use tokio::time::timeout;

use crate::{
    domain::{AnalysisProvider, PaperAnalysis},
    remote::{MAX_PUBLIC_REDIRECTS, public_http_client},
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const LINK_CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const NO_VERIFIED_CONTEXT: &str = "No independently sourced field-history, reception, or later-interpretation note passed link verification for this analysis.";

pub(super) async fn verify_context_sources(analysis: &mut PaperAnalysis) {
    if analysis.provider == AnalysisProvider::Heuristic {
        analysis.context_notes.clear();
        analysis.context_sources.clear();
        return;
    }
    verify_context_sources_with(
        analysis,
        |url| async move { verify_public_link(&url).await },
    )
    .await;
}

async fn verify_context_sources_with<F, Fut>(analysis: &mut PaperAnalysis, mut verify: F)
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Option<String>>,
{
    let cited_ids = analysis
        .context_notes
        .iter()
        .flat_map(|note| note.source_ids.iter().cloned())
        .collect::<HashSet<_>>();
    let candidates = std::mem::take(&mut analysis.context_sources)
        .into_iter()
        .filter(|source| cited_ids.contains(&source.id))
        .collect::<Vec<_>>();
    let mut verified = Vec::new();
    for mut source in candidates {
        if let Some(final_url) = verify(source.url.clone()).await {
            source.url = final_url;
            source.verified_at = Utc::now();
            verified.push(source);
        }
    }

    let verified_ids = verified
        .iter()
        .map(|source| source.id.as_str())
        .collect::<HashSet<_>>();
    analysis.context_notes.retain(|note| {
        !note.source_ids.is_empty()
            && note
                .source_ids
                .iter()
                .all(|source_id| verified_ids.contains(source_id.as_str()))
    });
    let referenced_ids = analysis
        .context_notes
        .iter()
        .flat_map(|note| note.source_ids.iter().map(String::as_str))
        .collect::<HashSet<_>>();
    verified.retain(|source| referenced_ids.contains(source.id.as_str()));
    analysis.context_sources = verified;

    analysis.outsider_brief = if analysis.context_notes.is_empty() {
        NO_VERIFIED_CONTEXT.to_owned()
    } else {
        analysis
            .context_notes
            .iter()
            .map(|note| note.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    };
}

async fn verify_public_link(value: &str) -> Option<String> {
    timeout(LINK_CHECK_TIMEOUT, verify_public_link_redirects(value))
        .await
        .ok()?
}

async fn verify_public_link_redirects(value: &str) -> Option<String> {
    let mut current = Url::parse(value).ok()?;
    current.set_fragment(None);
    for redirect_count in 0..=MAX_PUBLIC_REDIRECTS {
        let client = public_http_client(&current, REQUEST_TIMEOUT).await.ok()?;
        let response = client
            .get(current.clone())
            .header(
                ACCEPT,
                "text/html,application/xhtml+xml,application/pdf;q=0.9,*/*;q=0.5",
            )
            .send()
            .await
            .ok()?;
        if response.status().is_success() {
            return Some(current.to_string());
        }
        if !response.status().is_redirection() || redirect_count == MAX_PUBLIC_REDIRECTS {
            return None;
        }
        let location = response.headers().get(LOCATION)?.to_str().ok()?;
        current = current.join(location).ok()?;
        current.set_fragment(None);
    }
    None
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::domain::{ContextNote, ContextSource, PaperAnalysis};

    #[tokio::test]
    async fn drops_a_note_unless_every_cited_link_verifies() {
        let mut analysis = analysis_with_sources();
        verify_context_sources_with(&mut analysis, |url| async move {
            url.contains("working").then_some(url)
        })
        .await;

        assert!(analysis.context_notes.is_empty());
        assert!(analysis.context_sources.is_empty());
        assert_eq!(analysis.outsider_brief, NO_VERIFIED_CONTEXT);
    }

    #[tokio::test]
    async fn persists_only_sources_referenced_by_verified_notes() {
        let mut analysis = analysis_with_sources();
        analysis.context_notes[0].source_ids = vec!["working".to_owned()];
        verify_context_sources_with(&mut analysis, |url| async move { Some(url) }).await;

        assert_eq!(analysis.context_notes.len(), 1);
        assert_eq!(analysis.context_sources.len(), 1);
        assert_eq!(analysis.context_sources[0].id, "working");
        assert_eq!(analysis.outsider_brief, "A grounded reception claim.");
    }

    fn analysis_with_sources() -> PaperAnalysis {
        let source = |id: &str| ContextSource {
            id: id.to_owned(),
            title: format!("{id} source"),
            authors: vec!["Researcher".to_owned()],
            year: Some(2020),
            url: format!("https://example.com/{id}"),
            supports: "Supports the contextual claim.".to_owned(),
            verified_at: Utc::now(),
        };
        PaperAnalysis {
            schema_version: 4,
            provider: AnalysisProvider::Codex,
            generated_at: Utc::now(),
            thesis: "A thesis.".to_owned(),
            outsider_brief: "Temporary unverified context.".to_owned(),
            author_abstract: None,
            abstract_extraction: None,
            context_notes: vec![ContextNote {
                text: "A grounded reception claim.".to_owned(),
                source_ids: vec!["working".to_owned(), "broken".to_owned()],
            }],
            context_sources: vec![source("working"), source("broken"), source("unused")],
            prerequisites: Vec::new(),
            sections: Vec::new(),
            claims: Vec::new(),
            glossary: Vec::new(),
            caveats: Vec::new(),
            reading_path: Vec::new(),
        }
    }
}
