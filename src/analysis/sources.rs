use std::{
    collections::HashSet,
    future::Future,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};

use chrono::Utc;
use reqwest::{
    Client, Url,
    header::{ACCEPT, LOCATION},
    redirect::Policy,
};
use tokio::{net::lookup_host, time::timeout};

use crate::domain::{AnalysisProvider, PaperAnalysis};

const MAX_REDIRECTS: usize = 5;
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
    for redirect_count in 0..=MAX_REDIRECTS {
        let (host, address) = resolve_public_target(&current).await?;
        let mut builder = Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .connect_timeout(REQUEST_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .user_agent(concat!("lysilogos/", env!("CARGO_PKG_VERSION")));
        if host.parse::<IpAddr>().is_err() {
            builder = builder.resolve(&host, address);
        }
        let client = builder.build().ok()?;
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
        if !response.status().is_redirection() || redirect_count == MAX_REDIRECTS {
            return None;
        }
        let location = response.headers().get(LOCATION)?.to_str().ok()?;
        current = current.join(location).ok()?;
        current.set_fragment(None);
    }
    None
}

async fn resolve_public_target(url: &Url) -> Option<(String, SocketAddr)> {
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    let expected_port = match url.scheme() {
        "http" => 80,
        "https" => 443,
        _ => return None,
    };
    if url.port().is_some_and(|port| port != expected_port) {
        return None;
    }
    let raw_host = url.host_str()?;
    if raw_host.ends_with('.') {
        return None;
    }
    let host = raw_host.to_ascii_lowercase();
    if host.is_empty() || host == "localhost" || host.ends_with(".localhost") {
        return None;
    }

    if let Ok(address) = host.parse::<IpAddr>() {
        return is_public_address(address)
            .then_some((host, SocketAddr::new(address, expected_port)));
    }

    let addresses = lookup_host((host.as_str(), expected_port))
        .await
        .ok()?
        .collect::<Vec<_>>();
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| !is_public_address(address.ip()))
    {
        return None;
    }
    Some((host, *addresses.first()?))
}

fn is_public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_ipv4(address),
        IpAddr::V6(address) => is_public_ipv6(address),
    }
}

fn is_public_ipv4(address: Ipv4Addr) -> bool {
    let [first, second, third, _] = address.octets();
    !(first == 0
        || first == 10
        || first == 127
        || first >= 224
        || (first == 100 && (64..=127).contains(&second))
        || (first == 169 && second == 254)
        || (first == 172 && (16..=31).contains(&second))
        || (first == 192 && second == 0 && third == 0)
        || (first == 192 && second == 0 && third == 2)
        || (first == 192 && second == 88 && third == 99)
        || (first == 192 && second == 168)
        || (first == 198 && (second == 18 || second == 19))
        || (first == 198 && second == 51 && third == 100)
        || (first == 203 && second == 0 && third == 113))
}

fn is_public_ipv6(address: Ipv6Addr) -> bool {
    if let Some(embedded) = address.to_ipv4() {
        return is_public_ipv4(embedded);
    }
    let segments = address.segments();
    !(address.is_unspecified()
        || address.is_loopback()
        || address.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] & 0xffc0) == 0xfec0
        || (segments[0] == 0x0064 && segments[1] == 0xff9b)
        || (segments[0] == 0x2001 && segments[1] < 0x0200)
        || (segments[0] == 0x2001 && segments[1] == 0x0db8)
        || segments[0] == 0x2002)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::domain::{ContextNote, ContextSource, PaperAnalysis};

    #[test]
    fn rejects_private_reserved_and_metadata_networks() {
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "100.64.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "192.168.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
        ] {
            let parsed = address
                .parse::<IpAddr>()
                .expect("test address should parse");
            assert!(!is_public_address(parsed), "{address} must not be fetched");
        }
        assert!(is_public_address(IpAddr::V4(Ipv4Addr::new(
            93, 184, 216, 34
        ))));
        assert!(is_public_address(
            "2606:2800:220:1:248:1893:25c8:1946"
                .parse()
                .expect("test address should parse")
        ));
    }

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

    #[tokio::test]
    async fn rejects_unsafe_url_shapes_before_a_request() {
        for value in [
            "ftp://93.184.216.34/source",
            "https://reader@93.184.216.34/source",
            "https://93.184.216.34:8443/source",
            "https://127.0.0.1/source",
            "https://example.com./source",
        ] {
            let url = Url::parse(value).expect("test URL should parse");
            assert!(
                resolve_public_target(&url).await.is_none(),
                "{value} must be rejected"
            );
        }

        let public_literal =
            Url::parse("https://93.184.216.34/source").expect("public test URL should parse");
        assert!(resolve_public_target(&public_literal).await.is_some());
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
