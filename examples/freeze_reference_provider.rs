//! Explicit truth-build transport. The Python builder freezes this public JSON externally.
use clap::Parser;
use lysilogy::citation_graph::{GraphHttp, Identifier, Provider, Request};
use serde_json::json;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, clap::ValueEnum)]
enum TruthProvider {
    Crossref,
    Openalex,
}

#[derive(Parser)]
struct Arguments {
    #[arg(long, value_enum)]
    provider: TruthProvider,
    #[arg(long, required = true, num_args = 1..)]
    doi: Vec<String>,
}

fn request(provider: TruthProvider, identifiers: &[String]) -> Result<Request, &'static str> {
    if identifiers.is_empty() || identifiers.len() > 100 {
        return Err("Truth lookup requires between one and 100 DOIs");
    }
    let ids = identifiers
        .iter()
        .map(|text| {
            Identifier::parse(text)
                .ok()
                .filter(|id| id.scheme == "doi")
                .map(|id| id.value)
                .ok_or("Truth lookup requires valid DOIs")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (provider, base) = match provider {
        TruthProvider::Crossref => (Provider::Crossref, "https://api.crossref.org/"),
        TruthProvider::Openalex => (Provider::Openalex, "https://api.openalex.org/"),
    };
    if provider == Provider::Crossref && ids.len() != 1 {
        return Err("Crossref deposited-reference lookup requires one citing DOI");
    }
    let mut url = reqwest::Url::parse(base).expect("static HTTPS provider URL");
    url.path_segments_mut()
        .expect("hierarchical URL")
        .pop_if_empty()
        .push("works");
    if ids.len() == 1 {
        let identifier = if provider == Provider::Crossref {
            ids[0].clone()
        } else {
            format!("https://doi.org/{}", ids[0])
        };
        url.path_segments_mut()
            .expect("hierarchical URL")
            .push(&identifier);
    } else {
        if ids.iter().any(|id| id.contains(['|', ','])) {
            return Err("DOIs containing OpenAlex filter syntax require singleton lookups");
        }
        url.query_pairs_mut()
            .append_pair("filter", &format!("doi:{}", ids.join("|")))
            .append_pair("per-page", "100");
    }
    Ok(Request { provider, url })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Arguments::parse();
    let request = request(args.provider, &args.doi).map_err(std::io::Error::other)?;
    let provider = request.provider;
    let url = request.url.to_string();
    let client = GraphHttp::from_environment()?;
    let started = std::time::Instant::now();
    let response = client
        .get_with_provenance(request)
        .await
        .map_err(|failure| std::io::Error::other(failure.message))?;
    let body = serde_json::to_string(&response.value)?;
    let fetched_at =
        chrono::DateTime::from_timestamp_millis(i64::try_from(response.fetched_at_ms)?)
            .ok_or_else(|| std::io::Error::other("Invalid provider fetch timestamp"))?;
    serde_json::to_writer(
        std::io::stdout().lock(),
        &json!({
            "schema_version": 1, "provider": provider, "url": url,
            "retrieved_at": fetched_at.to_rfc3339(), "frozen_at": chrono::Utc::now().to_rfc3339(),
            "cache_hit": response.cache_hit, "body_sha256": format!("{:x}", Sha256::digest(body.as_bytes())),
            "body": body, "wall_seconds": started.elapsed().as_secs_f64(),
            "model_calls": 0, "model_cost_usd": 0, "provider_cost_usd": null
        }),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_encode_identifier_as_path_data_when_doi_contains_url_delimiters() {
        let request = request(TruthProvider::Crossref, &["10.1234/a?b#c".to_owned()]).unwrap();
        assert!(request.url.query().is_none());
        assert!(request.url.fragment().is_none());
        assert!(request.url.path().ends_with("10.1234%2Fa%3Fb%23c"));
    }

    #[test]
    fn should_bound_and_disambiguate_requests_when_openalex_lookups_are_batched() {
        assert!(request(TruthProvider::Openalex, &[]).is_err());
        assert!(request(TruthProvider::Openalex, &vec!["10.1234/x".to_owned(); 101]).is_err());
        assert!(
            request(
                TruthProvider::Crossref,
                &["10.1234/x".to_owned(), "10.1234/y".to_owned()]
            )
            .is_err()
        );
        assert!(
            request(
                TruthProvider::Openalex,
                &["10.1234/x|y".to_owned(), "10.1234/z".to_owned()]
            )
            .is_err()
        );
        let request = request(
            TruthProvider::Openalex,
            &["10.1234/x".to_owned(), "10.1234/y".to_owned()],
        )
        .unwrap();
        assert_eq!(request.provider, Provider::Openalex);
        assert!(
            request
                .url
                .query_pairs()
                .any(|(key, value)| key == "filter" && value == "doi:10.1234/x|10.1234/y")
        );
    }
}
