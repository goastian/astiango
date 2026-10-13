use std::{io::Read, net::IpAddr, sync::Arc};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use url::Url;

pub const VERSION: u32 = 1;
pub const IDENTITY_VERSION: u32 = 1;
pub const DEFAULT_AGENT: &str = "AstianGOBot";

pub fn url_key(url: &Url) -> Result<String> {
    anyhow::ensure!(
        matches!(url.scheme(), "http" | "https") && url.host().is_some(),
        "unsupported URL identity"
    );
    anyhow::ensure!(
        url.username().is_empty() && url.password().is_none(),
        "URL credentials are not supported"
    );
    let mut url = url.clone();
    url.set_fragment(None);
    Ok(url.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Header {
    pub name: String,
    pub value_hex: String,
}

impl Header {
    pub fn bytes(&self) -> Result<Vec<u8>> {
        anyhow::ensure!(self.value_hex.len() % 2 == 0, "invalid header encoding");
        (0..self.value_hex.len())
            .step_by(2)
            .map(|i| Ok(u8::from_str_radix(&self.value_hex[i..i + 2], 16)?))
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HttpResponse {
    pub url: String,
    pub status: u16,
    pub version: String,
    pub headers: Vec<Header>,
    pub received_at: chrono::DateTime<chrono::Utc>,
    pub peer_ip: Option<IpAddr>,
    pub address_validated: bool,
    pub network_policy: String,
}

impl HttpResponse {
    pub fn received(
        response: &reqwest::Response,
        url: &Url,
        date: chrono::DateTime<chrono::Utc>,
    ) -> Self {
        let peer = response.extensions().get::<VerifiedPeer>();
        Self {
            url: url.to_string(),
            status: response.status().as_u16(),
            version: format!("{:?}", response.version()),
            headers: response
                .headers()
                .iter()
                .map(|(name, value)| Header {
                    name: name.to_string(),
                    value_hex: hex(value.as_bytes()),
                })
                .collect(),
            received_at: date,
            peer_ip: peer.map(|p| p.ip),
            address_validated: peer.is_some_and(|p| p.validated),
            network_policy: peer.map_or("unreported".into(), |p| p.policy.clone()),
        }
    }

    pub fn values<'a>(&'a self, name: &'a str) -> impl Iterator<Item = String> + 'a {
        self.headers
            .iter()
            .filter(move |h| h.name.eq_ignore_ascii_case(name))
            .filter_map(|h| h.bytes().ok())
            .map(|v| String::from_utf8_lossy(&v).into_owned())
    }

    pub fn transformations(&self) -> Vec<String> {
        let mut result = vec![
            "http-header-name-case-and-order-not-preserved".into(),
            "http-reason-phrase-not-exposed".into(),
            "content-length-reconstructed-from-captured-payload".into(),
            "transfer-encoding-and-trailer-headers-removed".into(),
        ];
        if self
            .headers
            .iter()
            .any(|h| h.name.eq_ignore_ascii_case("transfer-encoding"))
        {
            result.push("transfer-framing-removed".into());
        }
        if !matches!(self.version.as_str(), "HTTP/1.0" | "HTTP/1.1") {
            result.push("http-envelope-serialized-as-http1".into());
        }
        result
    }
}

#[derive(Debug, Clone)]
pub struct VerifiedPeer {
    pub ip: IpAddr,
    pub validated: bool,
    pub policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Directive {
    pub agent: Option<String>,
    pub source: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Robots {
    #[serde(default)]
    pub directives_seen: u64,
    #[serde(default)]
    pub directives_truncated: bool,
    pub no_index: bool,
    pub no_follow: bool,
    pub directives: Vec<Directive>,
}

impl Robots {
    pub fn merge(&mut self, other: Self) {
        self.no_index |= other.no_index;
        self.no_follow |= other.no_follow;
        for directive in other.directives {
            self.add(directive, "");
        }
    }

    pub fn from_headers(response: &HttpResponse, agent: &str) -> Self {
        let mut result = Self::default();
        for value in response.values("x-robots-tag") {
            let mut scope = None;
            for part in value.split(',') {
                let mut directive = part.trim();
                if let Some((prefix, suffix)) = directive.split_once(':') {
                    if !matches!(
                        prefix.trim().to_ascii_lowercase().as_str(),
                        "unavailable_after"
                            | "max-snippet"
                            | "max-image-preview"
                            | "max-video-preview"
                    ) {
                        scope = Some(prefix.trim().to_owned());
                        directive = suffix.trim();
                    }
                }
                result.add(
                    Directive {
                        agent: scope.clone(),
                        source: "http".into(),
                        value: directive.into(),
                    },
                    agent,
                );
            }
        }
        result
    }

    pub fn add(&mut self, directive: Directive, agent: &str) {
        if directive.agent.as_ref().is_none_or(|a| {
            a.eq_ignore_ascii_case(agent) || a.eq_ignore_ascii_case("robots") || a == "*"
        }) {
            for token in directive
                .value
                .split(|c: char| c == ',' || c.is_ascii_whitespace())
            {
                match token.to_ascii_lowercase().as_str() {
                    "noindex" => self.no_index = true,
                    "nofollow" => self.no_follow = true,
                    "none" => {
                        self.no_index = true;
                        self.no_follow = true;
                    }
                    _ => {}
                }
            }
        }
        self.directives_seen += 1;
        if self.directives.len() < 64 {
            let mut directive = directive;
            if directive.value.len() > 4096 {
                let mut end = 4096;
                while !directive.value.is_char_boundary(end) {
                    end -= 1;
                }
                directive.value.truncate(end);
                self.directives_truncated = true;
            }
            self.directives.push(directive);
        } else {
            self.directives_truncated = true;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Metadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revalidation: Option<crate::crawler::recrawl::PayloadReference>,
    pub schema_version: u32,
    pub identity_version: u32,
    pub fetch_id: String,
    pub requested_url: String,
    pub response: HttpResponse,
    pub redirects: Vec<HttpResponse>,
    pub payload_sha256: String,
    pub payload_bytes: usize,
    pub decoded_bytes: usize,
    pub charset: String,
    pub decoded_limit_bytes: usize,
    pub user_agent: String,
    pub robots: Robots,
    pub transformations: Vec<String>,
    pub fetch_time_ms: u64,
    #[serde(default)]
    pub discovery_urls: Vec<Url>,
    #[serde(default)]
    pub internal_urls: Vec<Url>,
}

impl Metadata {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.schema_version == VERSION,
            "unsupported capture schema version {}; supported: {}",
            self.schema_version,
            VERSION
        );
        anyhow::ensure!(
            self.identity_version == IDENTITY_VERSION,
            "unsupported URL identity version {}",
            self.identity_version
        );
        if let Some(reference) = &self.revalidation {
            anyhow::ensure!(
                reference.version == 1
                    && self.response.status == 304
                    && self.payload_bytes == 0
                    && self.decoded_bytes == 0
                    && reference.record_end > reference.record_offset
                    && reference.payload_sha256.len() == 64
                    && reference.fetch_id != self.fetch_id,
                "invalid 304 payload reference"
            );
            uuid::Uuid::parse_str(&reference.fetch_id).context("invalid prior fetch_id")?;
        }
        uuid::Uuid::parse_str(&self.fetch_id).context("invalid capture fetch_id")?;
        anyhow::ensure!(
            self.decoded_limit_bytes > 0 && self.decoded_limit_bytes <= 128 * 1024 * 1024,
            "invalid capture decoding limit"
        );
        anyhow::ensure!(
            self.discovery_urls.len() <= 1000 && self.internal_urls.len() <= 1000,
            "capture discovery limit exceeded"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capture {
    pub metadata: Metadata,
    pub payload: Arc<Vec<u8>>,
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn sha256(bytes: &[u8]) -> String {
    hex(ring::digest::digest(&ring::digest::SHA256, bytes).as_ref())
}

#[cfg(any(test, feature = "crawler-bench"))]
pub fn fixture(url: &Url, fetch_id: String, payload: Vec<u8>, headers: Vec<Header>) -> Capture {
    let response = HttpResponse {
        url: url.to_string(),
        status: 200,
        version: "HTTP/1.1".into(),
        headers,
        received_at: chrono::Utc::now(),
        peer_ip: Some("127.0.0.1".parse().unwrap()),
        address_validated: true,
        network_policy: "fixture".into(),
    };
    let encoding = response
        .values("content-encoding")
        .collect::<Vec<_>>()
        .join(",");
    let body = decode(&payload, &encoding, "UTF-8", 32 * 1024 * 1024).unwrap();
    let robots = Robots::from_headers(&response, DEFAULT_AGENT);
    Capture {
        metadata: Metadata {
            revalidation: None,
            schema_version: VERSION,
            identity_version: IDENTITY_VERSION,
            fetch_id,
            requested_url: url_key(url).unwrap(),
            response,
            redirects: Vec::new(),
            payload_sha256: sha256(&payload),
            payload_bytes: payload.len(),
            decoded_bytes: body.len(),
            charset: "UTF-8".into(),
            decoded_limit_bytes: 32 * 1024 * 1024,
            user_agent: DEFAULT_AGENT.into(),
            robots,
            transformations: vec!["test-fixture".into()],
            fetch_time_ms: 1,
            discovery_urls: Vec::new(),
            internal_urls: Vec::new(),
        },
        payload: Arc::new(payload),
    }
}

#[cfg(test)]
#[path = "capture_tests.rs"]
mod tests;

pub fn content_encoding(value: &str) -> Result<&'static str> {
    let codecs: Vec<_> = value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("identity"))
        .collect();
    match codecs.as_slice() {
        [] => Ok("identity"),
        [codec] if codec.eq_ignore_ascii_case("gzip") || codec.eq_ignore_ascii_case("x-gzip") => {
            Ok("gzip")
        }
        _ => anyhow::bail!("unsupported Content-Encoding: {value}"),
    }
}

pub fn decode(
    payload: &[u8],
    content_encoding: &str,
    charset: &str,
    limit: usize,
) -> Result<String> {
    let reader: Box<dyn Read + '_> = match self::content_encoding(content_encoding)? {
        "" | "identity" => Box::new(payload),
        "gzip" | "x-gzip" => Box::new(flate2::read::MultiGzDecoder::new(payload)),
        other => anyhow::bail!("unsupported Content-Encoding: {other}"),
    };
    let mut reader = reader;
    let encoding =
        encoding_rs::Encoding::for_label(charset.as_bytes()).unwrap_or(encoding_rs::UTF_8);
    let mut decoder = encoding.new_decoder();
    let mut text = String::new();
    let mut input = [0u8; 8192];
    let mut output = [0u8; 8192];
    let mut received = 0usize;
    loop {
        let size = reader.read(&mut input)?;
        received = received
            .checked_add(size)
            .context("decoded length overflow")?;
        anyhow::ensure!(received <= limit, "decoded body exceeds limit {limit}");
        let mut bytes = &input[..size];
        loop {
            let (result, read, written, _) = decoder.decode_to_utf8(bytes, &mut output, size == 0);
            anyhow::ensure!(
                written <= limit - text.len(),
                "decoded text exceeds limit {limit}"
            );
            text.push_str(std::str::from_utf8(&output[..written])?);
            bytes = &bytes[read..];
            if result == encoding_rs::CoderResult::InputEmpty {
                break;
            }
        }
        if size == 0 {
            break;
        }
    }
    Ok(text)
}
