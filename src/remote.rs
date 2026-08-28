use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};

use reqwest::{Client, Url, redirect::Policy};
use tokio::net::lookup_host;

use crate::{Error, Result};

pub const MAX_PUBLIC_REDIRECTS: usize = 5;

pub async fn public_http_client(url: &Url, request_timeout: Duration) -> Result<Client> {
    let (host, address) = resolve_public_target(url).await.ok_or_else(|| {
        Error::InvalidRequest(
            "remote URLs must use standard HTTP(S) and resolve only to public addresses".to_owned(),
        )
    })?;
    let mut builder = Client::builder()
        .redirect(Policy::none())
        .no_proxy()
        .connect_timeout(request_timeout)
        .timeout(request_timeout)
        .user_agent(concat!("lysilogy/", env!("CARGO_PKG_VERSION")));
    if host.parse::<IpAddr>().is_err() {
        builder = builder.resolve(&host, address);
    }
    builder
        .build()
        .map_err(|error| Error::RemoteImport(format!("could not create HTTP client: {error}")))
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
    use super::*;

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
}
