//! Landing-page parsing: find a PDF link in publisher or repository HTML.
//!
//! Deliberately regex-based and forgiving; the caller caps the input size.

use std::sync::LazyLock;

use regex::Regex;
use reqwest::Url;
use serde::{Deserialize, Serialize};

/// The longest anchor text kept for a [`PageLink`], in characters.
const MAX_LINK_TEXT_CHARS: usize = 160;

static META_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<meta\b[^>]*>").expect("meta regex"));
static LINK_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<link\b[^>]*>").expect("link regex"));
static ANCHOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<a\b([^>]*)>(.*?)</a\s*>").expect("anchor regex"));
static ATTRIBUTE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)([a-z_:][a-z0-9_:.-]*)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+))"#)
        .expect("attribute regex")
});
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<[^>]*>").expect("tag regex"));

/// One absolute http(s) link on a page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageLink {
    pub href: String,
    pub text: String,
}

/// The attributes of one tag, lowercased by name, in source order.
fn attributes(tag: &str) -> Vec<(String, String)> {
    ATTRIBUTE
        .captures_iter(tag)
        .map(|captures| {
            let value = captures
                .get(2)
                .or_else(|| captures.get(3))
                .or_else(|| captures.get(4))
                .map_or("", |found| found.as_str());
            (captures[1].to_ascii_lowercase(), value.to_owned())
        })
        .collect()
}

fn attribute<'a>(attributes: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

fn decode_entities(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

/// Resolve a possibly relative `href` to an absolute http(s) URL.
fn resolve_http(base_url: &str, href: &str) -> Option<String> {
    let href = decode_entities(href.trim());
    if href.is_empty() {
        return None;
    }
    let resolved = match Url::parse(base_url) {
        Ok(base) => base.join(&href).ok()?,
        Err(_) => Url::parse(&href).ok()?,
    };
    matches!(resolved.scheme(), "http" | "https").then(|| resolved.to_string())
}

fn anchor_text(inner_html: &str) -> String {
    let stripped = TAG.replace_all(inner_html, " ");
    decode_entities(&stripped)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn clip_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

fn meta_pdf_url(html: &str, base_url: &str) -> Option<String> {
    META_TAG.find_iter(html).find_map(|tag| {
        let attributes = attributes(tag.as_str());
        let name = attribute(&attributes, "name")?;
        if !name.trim().eq_ignore_ascii_case("citation_pdf_url") {
            return None;
        }
        resolve_http(base_url, attribute(&attributes, "content")?)
    })
}

fn alternate_pdf_url(html: &str, base_url: &str) -> Option<String> {
    LINK_TAG.find_iter(html).find_map(|tag| {
        let attributes = attributes(tag.as_str());
        let is_alternate = attribute(&attributes, "rel")?
            .split_whitespace()
            .any(|token| token.eq_ignore_ascii_case("alternate"));
        let is_pdf = attribute(&attributes, "type")?
            .trim()
            .eq_ignore_ascii_case("application/pdf");
        if !is_alternate || !is_pdf {
            return None;
        }
        resolve_http(base_url, attribute(&attributes, "href")?)
    })
}

fn unique_pdf_anchor(html: &str, base_url: &str) -> Option<String> {
    let mut found: Vec<String> = Vec::new();
    for captures in ANCHOR.captures_iter(html) {
        let attributes = attributes(&captures[1]);
        let Some(href) = attribute(&attributes, "href") else {
            continue;
        };
        let Some(resolved) = resolve_http(base_url, href) else {
            continue;
        };
        let Ok(parsed) = Url::parse(&resolved) else {
            continue;
        };
        if !parsed.path().to_ascii_lowercase().ends_with(".pdf") {
            continue;
        }
        let text = anchor_text(&captures[2]).to_ascii_lowercase();
        let signal = |haystack: &str| haystack.contains("pdf") || haystack.contains("download");
        if !signal(&text) && !signal(&resolved.to_ascii_lowercase()) {
            continue;
        }
        if !found.contains(&resolved) {
            found.push(resolved);
        }
    }
    if found.len() == 1 { found.pop() } else { None }
}

/// The PDF link a landing page advertises, resolved against `base_url`.
///
/// Order: `citation_pdf_url` meta, then `<link rel=alternate type=application/pdf>`,
/// then the only anchor that points at a `.pdf` and mentions pdf or download.
#[must_use]
pub fn pdf_link_from_meta(html: &str, base_url: &str) -> Option<String> {
    meta_pdf_url(html, base_url)
        .or_else(|| alternate_pdf_url(html, base_url))
        .or_else(|| unique_pdf_anchor(html, base_url))
}

/// Up to `limit` distinct absolute http(s) anchors, in page order.
#[must_use]
pub fn page_links(html: &str, base_url: &str, limit: usize) -> Vec<PageLink> {
    let mut links: Vec<PageLink> = Vec::new();
    for captures in ANCHOR.captures_iter(html) {
        if links.len() >= limit {
            break;
        }
        let attributes = attributes(&captures[1]);
        let Some(href) = attribute(&attributes, "href") else {
            continue;
        };
        if href.trim().starts_with('#') {
            continue;
        }
        let Some(mut resolved) = resolve_http(base_url, href) else {
            continue;
        };
        if let Some(position) = resolved.find('#') {
            resolved.truncate(position);
        }
        if links.iter().any(|link| link.href == resolved) {
            continue;
        }
        links.push(PageLink {
            href: resolved,
            text: clip_chars(&anchor_text(&captures[2]), MAX_LINK_TEXT_CHARS),
        });
    }
    links
}

#[cfg(test)]
mod tests {
    use super::*;

    const HIGHWIRE: &str = include_str!("../../tests/fixtures/acquire/landing_highwire.html");
    const ANCHOR_ONLY: &str = include_str!("../../tests/fixtures/acquire/landing_anchor_only.html");
    const NONE: &str = include_str!("../../tests/fixtures/acquire/landing_none.html");
    const BASE: &str = "https://journal.example.org/articles/1/";

    #[test]
    fn should_find_citation_pdf_url_when_meta_attributes_are_reordered() {
        assert_eq!(
            pdf_link_from_meta(HIGHWIRE, BASE).as_deref(),
            Some("https://journal.example.org/content/article-1.full.pdf?download=1&v=2"),
            "meta wins over link and anchor, with reordered attrs, odd case and &amp;"
        );
        let double = r#"<meta content="https://a.example.org/x.pdf" name="citation_pdf_url" />"#;
        assert_eq!(
            pdf_link_from_meta(double, BASE).as_deref(),
            Some("https://a.example.org/x.pdf")
        );
        let unquoted = "<meta name=citation_pdf_url content=https://a.example.org/y.pdf>";
        assert_eq!(
            pdf_link_from_meta(unquoted, BASE).as_deref(),
            Some("https://a.example.org/y.pdf")
        );
    }

    #[test]
    fn should_use_alternate_link_when_meta_is_missing() {
        let html = r#"<link type="application/pdf" rel="alternate" href="/a.pdf">"#;
        assert_eq!(
            pdf_link_from_meta(html, BASE).as_deref(),
            Some("https://journal.example.org/a.pdf")
        );
        let stylesheet = r#"<link rel="stylesheet" type="application/pdf" href="/a.pdf">"#;
        assert_eq!(pdf_link_from_meta(stylesheet, BASE), None);
    }

    #[test]
    fn should_resolve_relative_pdf_link_when_base_url_given() {
        assert_eq!(
            pdf_link_from_meta(ANCHOR_ONLY, "https://repo.example.org/record/42").as_deref(),
            Some("https://repo.example.org/files/record-42.pdf")
        );
        let relative = r#"<meta name="citation_pdf_url" content="paper.pdf">"#;
        assert_eq!(
            pdf_link_from_meta(relative, BASE).as_deref(),
            Some("https://journal.example.org/articles/1/paper.pdf")
        );
    }

    #[test]
    fn should_ignore_non_http_meta_content_when_scheme_is_unsafe() {
        let html = r#"<meta name="citation_pdf_url" content="javascript:alert(1)">"#;
        assert_eq!(pdf_link_from_meta(html, BASE), None);
        let ftp = r#"<meta name="citation_pdf_url" content="ftp://a.example.org/x.pdf">"#;
        assert_eq!(pdf_link_from_meta(ftp, BASE), None);
    }

    #[test]
    fn should_return_none_when_landing_page_has_no_pdf_signal() {
        assert_eq!(pdf_link_from_meta(NONE, BASE), None);
        assert_eq!(pdf_link_from_meta("", BASE), None);
        assert_eq!(pdf_link_from_meta("<meta <a <link", BASE), None);
    }

    #[test]
    fn should_return_none_when_several_pdf_anchors_are_ambiguous() {
        let html = r#"<a href="/a.pdf">PDF</a> <a href="/b.pdf">Download</a>"#;
        assert_eq!(pdf_link_from_meta(html, BASE), None);
        let repeated = r#"<a href="/a.pdf">PDF</a> <a href="/a.pdf">Download</a>"#;
        assert_eq!(
            pdf_link_from_meta(repeated, BASE).as_deref(),
            Some("https://journal.example.org/a.pdf"),
            "the same target twice is still unique"
        );
    }

    #[test]
    fn should_drop_javascript_and_mailto_links_when_listing_page_links() {
        let links = page_links(ANCHOR_ONLY, "https://repo.example.org/record/42", 50);
        let hrefs: Vec<&str> = links.iter().map(|link| link.href.as_str()).collect();
        assert_eq!(
            hrefs,
            [
                "https://repo.example.org/about",
                "https://repo.example.org/files/record-42.pdf"
            ]
        );
        assert!(
            page_links(NONE, BASE, 50)
                .iter()
                .all(|link| link.href != BASE)
        );
    }

    #[test]
    fn should_dedupe_strip_tags_and_collapse_whitespace_when_listing_page_links() {
        let html = "<a href='/x'>  Hello\n <b>big</b>\t world </a><a href=\"/x#frag\">again</a>\
                    <a href=\"https://o.example.org/y?a=1&amp;b=2\">Other</a>";
        let links = page_links(html, BASE, 50);
        assert_eq!(
            links,
            [
                PageLink {
                    href: "https://journal.example.org/x".to_owned(),
                    text: "Hello big world".to_owned()
                },
                PageLink {
                    href: "https://o.example.org/y?a=1&b=2".to_owned(),
                    text: "Other".to_owned()
                },
            ]
        );
        assert_eq!(page_links(html, BASE, 1).len(), 1);
        assert!(page_links(html, BASE, 0).is_empty());
    }

    #[test]
    fn should_clip_link_text_when_anchor_text_is_long() {
        let long = "é".repeat(500);
        let html = format!(r#"<a href="/long">{long}</a>"#);
        let links = page_links(&html, BASE, 5);
        assert_eq!(links[0].text.chars().count(), 160);
    }
}
