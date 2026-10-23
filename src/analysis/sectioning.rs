//! Shared reading-unit policy and deterministic feedback for model sectioning.
//! Size is a diagnostic, not permission to merge semantically unrelated text.

pub(super) const POLICY: &str = include_str!("../../prompts/sectioning.md");

pub(super) fn guidance(page_count: usize) -> String {
    let pages = page_count.max(1);
    let lower = pages.div_ceil(2);
    let upper = pages.saturating_mul(2).div_ceil(3).max(lower);
    format!(
        "{POLICY}\nThis PDF contains {pages} pages. Start by considering roughly {lower}–{upper} substantive reading units, then adjust for actual topic boundaries and subtract references, appendices, or adjacent articles from the main-text budget. This is a planning guide, not a quota or a reason to drop content. A short paper may need only one unit."
    )
}
