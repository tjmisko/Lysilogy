//! Model fallback for landing pages: pick the article PDF from a page's links.
//!
//! The model is off by default and only sees links the page already offered; its
//! answer is grounded to that list, so it can never invent a URL.

use std::fmt::Write as _;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};

use lysilogy_core::Coverage;
use lysilogy_db::source_record::SourceRecordRepository;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::landing::PageLink;
use crate::Result;

/// Bump when the prompt or request shape changes, so old cache entries are not reused.
pub const LINK_PICK_PROMPT_VERSION: u32 = 1;

/// Source record kind for cached picks.
pub const LINK_PICK_RECORD_KIND: &str = "acquisition_link_pick";

/// Most links shown to the model.
pub const MAX_PICK_LINKS: usize = 150;

/// Strict output schema: both keys required, nothing else allowed.
pub const LINK_PICK_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "required": ["pdf_link_index", "reason"],
  "properties": {
    "pdf_link_index": { "type": ["integer", "null"], "minimum": 0 },
    "reason": { "type": "string" }
  }
}"#;

/// What the model is shown.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct LinkPickRequest {
    pub work_title: Option<String>,
    pub work_year: Option<i32>,
    pub page_url: String,
    /// At most [`MAX_PICK_LINKS`] absolute links, in page order.
    pub links: Vec<PageLink>,
}

/// The model's answer, before grounding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkPick {
    pub pdf_link_index: Option<usize>,
    pub reason: String,
}

/// Why a picker produced no pick.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PickerError {
    /// The model program is not installed or not reachable.
    #[error("link picker unavailable: {0}")]
    Unavailable(String),
    /// The model program ran and failed.
    #[error("link picker failed: {0}")]
    Failed(String),
    /// The model answered with something that is not a pick.
    #[error("link picker returned a malformed answer: {0}")]
    Malformed(String),
}

/// The future returned by a picker.
pub type PickFuture<'a> =
    Pin<Box<dyn Future<Output = std::result::Result<LinkPick, PickerError>> + Send + 'a>>;

/// A model-backed chooser of the article PDF link on a landing page.
pub trait LandingLinkPicker: Send + Sync {
    /// Identifies the model in cache keys.
    fn model_key(&self) -> &str;

    /// Choose a link from `request.links`, or answer `null`.
    fn pick<'a>(&'a self, request: &'a LinkPickRequest) -> PickFuture<'a>;
}

/// The fixed-format prompt for one landing page.
#[must_use]
pub fn link_pick_prompt(request: &LinkPickRequest) -> String {
    let mut prompt = String::from(
        "You are helping find the full-text PDF of a scholarly article on its landing page. \
         Below are the links found on the page, numbered. Choose the one link that downloads \
         the PDF of exactly this article.\n\n",
    );
    let _ = writeln!(
        prompt,
        "Article title: {}",
        request.work_title.as_deref().unwrap_or("(unknown title)")
    );
    if let Some(year) = request.work_year {
        let _ = writeln!(prompt, "Article year: {year}");
    }
    let _ = writeln!(prompt, "Page URL: {}\n\nLinks:", request.page_url);
    for (index, link) in request.links.iter().enumerate() {
        let _ = writeln!(prompt, "[{index}] {} \u{2014} {}", link.text, link.href);
    }
    prompt.push_str(
        "\nAnswer with JSON: pdf_link_index (the number of the link that is the article's PDF, \
         or null if none of the links is the article PDF; do not guess, and do not choose \
         supplementary material, other articles, or the landing page itself) and reason (one \
         short sentence).\n",
    );
    prompt
}

/// SHA-256 (hex) of the prompt version and prompt, which keys the pick cache.
#[must_use]
pub fn link_pick_cache_key(request: &LinkPickRequest) -> String {
    let material = format!("{LINK_PICK_PROMPT_VERSION}\n{}", link_pick_prompt(request));
    format!("{:x}", Sha256::digest(material.as_bytes()))
}

/// Check a pick against the links it was chosen from.
///
/// # Errors
/// `index_out_of_range` when the index is not one of the offered links.
pub fn ground_pick(
    request: &LinkPickRequest,
    pick: &LinkPick,
) -> std::result::Result<Option<PageLink>, &'static str> {
    let Some(index) = pick.pdf_link_index else {
        return Ok(None);
    };
    request
        .links
        .get(index)
        .cloned()
        .map(Some)
        .ok_or("index_out_of_range")
}

/// Ask the picker, reusing a cached pick for the same prompt and model. Picker
/// errors come back as `Ok(Err(..))` and are never cached. `refresh` skips the
/// cache read.
pub async fn pick_cached(
    records: &SourceRecordRepository,
    picker: &dyn LandingLinkPicker,
    request: &LinkPickRequest,
    refresh: bool,
) -> Result<std::result::Result<LinkPick, PickerError>> {
    let key = format!("{}|{}", picker.model_key(), link_pick_cache_key(request));
    if !refresh
        && let Some(cached) = records.get(LINK_PICK_RECORD_KIND, &key).await?
        && let Ok(pick) = serde_json::from_value::<LinkPick>(cached)
    {
        return Ok(Ok(pick));
    }
    let pick = match picker.pick(request).await {
        Ok(pick) => pick,
        Err(error) => return Ok(Err(error)),
    };
    let payload = json!({ "pdf_link_index": pick.pdf_link_index, "reason": pick.reason });
    records
        .upsert(LINK_PICK_RECORD_KIND, &key, Coverage::Complete, &payload)
        .await?;
    Ok(Ok(pick))
}

type ScriptedFn =
    dyn Fn(&LinkPickRequest) -> std::result::Result<LinkPick, PickerError> + Send + Sync;

/// A deterministic picker for tests: answers from a closure and counts calls.
pub struct ScriptedLinkPicker {
    model_key: String,
    script: Box<ScriptedFn>,
    calls: AtomicUsize,
}

impl ScriptedLinkPicker {
    /// A picker that answers every request with `script`.
    #[must_use]
    pub fn new(
        model_key: &str,
        script: impl Fn(&LinkPickRequest) -> std::result::Result<LinkPick, PickerError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            model_key: model_key.to_owned(),
            script: Box::new(script),
            calls: AtomicUsize::new(0),
        }
    }

    /// How many times `pick` was called.
    #[must_use]
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl LandingLinkPicker for ScriptedLinkPicker {
    fn model_key(&self) -> &str {
        &self.model_key
    }

    fn pick<'a>(&'a self, request: &'a LinkPickRequest) -> PickFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let answer = (self.script)(request);
        Box::pin(async move { answer })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> LinkPickRequest {
        LinkPickRequest {
            work_title: Some("Cooperative Inverse Reinforcement Learning".to_owned()),
            work_year: Some(2016),
            page_url: "https://journal.example/article/1".to_owned(),
            links: vec![
                PageLink {
                    href: "https://journal.example/issue".to_owned(),
                    text: "Current issue".to_owned(),
                },
                PageLink {
                    href: "https://journal.example/dl/1".to_owned(),
                    text: "Full text".to_owned(),
                },
            ],
        }
    }

    #[test]
    fn should_number_links_when_building_pick_prompt() {
        let prompt = link_pick_prompt(&request());
        assert!(prompt.contains("[0] Current issue \u{2014} https://journal.example/issue"));
        assert!(prompt.contains("[1] Full text \u{2014} https://journal.example/dl/1"));
        assert!(prompt.contains("Article title: Cooperative Inverse Reinforcement Learning"));
        assert!(prompt.contains("Article year: 2016"));
        assert!(prompt.contains("Page URL: https://journal.example/article/1"));
        assert!(prompt.contains("null"), "tells the model it may decline");
    }

    #[test]
    fn should_parse_link_pick_schema_when_embedded() {
        let schema: serde_json::Value = serde_json::from_str(LINK_PICK_SCHEMA).expect("json");
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(
            schema["required"],
            json!(["pdf_link_index", "reason"]),
            "both keys required"
        );
        assert_eq!(
            schema["properties"]["pdf_link_index"],
            json!({"type": ["integer", "null"], "minimum": 0})
        );
    }

    #[test]
    fn should_key_cache_by_prompt_when_requests_differ() {
        let base = request();
        let mut other = request();
        other.links[1].href = "https://journal.example/dl/2".to_owned();
        assert_eq!(link_pick_cache_key(&base), link_pick_cache_key(&request()));
        assert_ne!(link_pick_cache_key(&base), link_pick_cache_key(&other));
        assert_eq!(link_pick_cache_key(&base).len(), 64);
    }

    #[test]
    fn should_ground_pick_to_offered_links_when_index_is_valid() {
        let request = request();
        let pick = |index| LinkPick {
            pdf_link_index: index,
            reason: String::new(),
        };
        assert_eq!(
            ground_pick(&request, &pick(Some(1)))
                .expect("ok")
                .expect("link")
                .href,
            "https://journal.example/dl/1"
        );
        assert_eq!(ground_pick(&request, &pick(None)), Ok(None));
        assert_eq!(
            ground_pick(&request, &pick(Some(2))),
            Err("index_out_of_range")
        );
        assert_eq!(
            ground_pick(&request, &pick(Some(usize::MAX))),
            Err("index_out_of_range")
        );
    }

    #[test]
    fn should_reject_negative_or_fractional_index_when_deserializing_pick() {
        assert!(serde_json::from_str::<LinkPick>(r#"{"pdf_link_index":-1,"reason":"x"}"#).is_err());
        assert!(
            serde_json::from_str::<LinkPick>(r#"{"pdf_link_index":1.5,"reason":"x"}"#).is_err()
        );
        assert!(
            serde_json::from_str::<LinkPick>(r#"{"reason":"x"}"#)
                .is_ok_and(|pick| pick.pdf_link_index.is_none())
        );
    }

    #[tokio::test]
    async fn should_count_calls_when_scripted_picker_is_asked() {
        let picker = ScriptedLinkPicker::new("fake:model", |_| {
            Ok(LinkPick {
                pdf_link_index: Some(1),
                reason: "full text".to_owned(),
            })
        });
        let request = request();
        let pick = picker.pick(&request).await.expect("pick");
        assert_eq!(pick.pdf_link_index, Some(1));
        assert_eq!(picker.calls(), 1);
        assert_eq!(picker.model_key(), "fake:model");
    }
}
