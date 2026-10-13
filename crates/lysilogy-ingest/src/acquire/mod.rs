//! Open-access acquisition: find where a Work's full text is legally available.
//!
//! This unit holds the deterministic locator (identifiers, provider candidates)
//! and landing-page parsing. Fetching and storage live in later units.

pub mod landing;
pub mod locate;

pub use landing::{PageLink, page_links, pdf_link_from_meta};
pub use locate::{
    AcquisitionIdentifiers, LinkCandidate, OPENALEX_OA_KIND, UNPAYWALL_KIND, arxiv_pdf_url,
    fetch_openalex_oa_cached, fetch_unpaywall_cached, identifiers_from, normalize_candidate_url,
    openalex_oa_url, parse_openalex_oa, parse_unpaywall, rank_candidates, unpaywall_url,
};
