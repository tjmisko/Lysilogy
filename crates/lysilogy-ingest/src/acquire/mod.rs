//! Open-access acquisition: find where a Work's full text is legally available.
//!
//! This unit holds the deterministic locator (identifiers, provider candidates)
//! and landing-page parsing. Fetching and storage live in later units.

pub mod fetch;
pub mod landing;
pub mod locate;
pub mod picker;
pub mod store;
pub mod verify;

pub use fetch::{
    ContentClass, FetchError, FetchFuture, FetchRequest, FetchedResource, HostThrottle,
    MAX_HTML_BYTES, MAX_PDF_BYTES, PdfFetcher, ScriptedFetcher, ScriptedResponse,
    classify_content_type, classify_status, has_pdf_magic, throttle_delay,
};
pub use landing::{PageLink, page_links, pdf_link_from_meta};
pub use locate::{
    AcquisitionIdentifiers, LinkCandidate, OPENALEX_OA_KIND, UNPAYWALL_KIND, arxiv_pdf_url,
    fetch_openalex_oa_cached, fetch_unpaywall_cached, identifiers_from, normalize_candidate_url,
    openalex_oa_url, parse_openalex_oa, parse_unpaywall, rank_candidates, unpaywall_url,
};
pub use picker::{
    LINK_PICK_PROMPT_VERSION, LINK_PICK_RECORD_KIND, LINK_PICK_SCHEMA, LandingLinkPicker, LinkPick,
    LinkPickRequest, MAX_PICK_LINKS, PickFuture, PickerError, ScriptedLinkPicker, ground_pick,
    link_pick_cache_key, link_pick_prompt, pick_cached,
};
pub use store::{
    FETCHED_DIR, blob_path, blob_storage_key, fetched_filename, place_library_copy, store_blob,
};
pub use verify::title_appears_in;
