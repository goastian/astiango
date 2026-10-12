use anyhow::Result;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::webpage::url_ext::UrlExt;

pub const URL_VERSION: u32 = 1;
pub const SITE_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UrlIdentity {
    pub version: u32,
    pub site_version: u32,
    pub url: String,
    pub origin: String,
    pub host: String,
    pub registrable_domain: Option<String>,
    pub tenant: String,
    pub private_suffix_tenant: bool,
}

impl UrlIdentity {
    pub fn from_url(url: &Url) -> Result<Self> {
        let key = crate::capture::url_key(url)?;
        let host = url.host_str().unwrap_or_default().to_owned();
        let domain_host = matches!(url.host(), Some(url::Host::Domain(_)));
        let registrable_domain = domain_host
            .then(|| url.icann_domain().map(str::to_owned))
            .flatten();
        let tenant = if domain_host {
            url.root_domain().unwrap_or(&host).to_owned()
        } else {
            host.clone()
        };
        let private_suffix_tenant = registrable_domain
            .as_ref()
            .is_some_and(|domain| domain != &tenant);
        Ok(Self {
            version: URL_VERSION,
            site_version: SITE_VERSION,
            url: key,
            origin: url.origin().ascii_serialization(),
            host,
            registrable_domain,
            tenant,
            private_suffix_tenant,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(value: &str) -> UrlIdentity {
        UrlIdentity::from_url(&Url::parse(value).unwrap()).unwrap()
    }

    #[test]
    fn safe_equivalences_keep_the_existing_url_key() {
        for (a, b) in [
            ("HTTP://EXAMPLE.COM:80/a#x", "http://example.com/a"),
            ("https://BÜCHER.de:443/", "https://xn--bcher-kva.de/"),
            ("http://[2001:db8:0:0::1]:80/", "http://[2001:db8::1]/"),
        ] {
            assert_eq!(identity(a).url, identity(b).url);
        }
    }

    #[test]
    fn resource_distinctions_and_signatures_survive() {
        for (a, b) in [
            ("http://example.com/", "https://example.com/"),
            ("https://example.com/a", "https://example.com/a/"),
            ("https://example.com/a//b", "https://example.com/a/b"),
            ("https://example.com/a%2Fb", "https://example.com/a/b"),
            (
                "https://example.com/?a=1&b=2",
                "https://example.com/?b=2&a=1",
            ),
            (
                "https://example.com/?sig=Ab%2B&x=1&x=2",
                "https://example.com/?sig=Ab%2B&x=2&x=1",
            ),
            (
                "https://example.com/?utm_source=keep",
                "https://example.com/",
            ),
            ("https://example.com:444/", "https://example.com/"),
        ] {
            assert_ne!(identity(a).url, identity(b).url);
        }
        assert_eq!(
            identity("https://example.com/?sig=Ab%2B&x=1&x=2").url,
            "https://example.com/?sig=Ab%2B&x=1&x=2"
        );
    }

    #[test]
    fn private_suffixes_have_independent_tenants() {
        for suffix in ["blogspot.com", "github.io", "wordpress.com"] {
            let a = identity(&format!("https://alice.{suffix}/"));
            let b = identity(&format!("https://bob.{suffix}/"));
            assert_eq!(a.registrable_domain, b.registrable_domain);
            assert_ne!(a.tenant, b.tenant);
            assert!(a.private_suffix_tenant);
        }
        assert_eq!(
            identity("https://www.example.co.uk/").tenant,
            "example.co.uk"
        );
        assert_eq!(identity("http://[::1]:8080/").tenant, "[::1]");
        assert_ne!(
            identity("http://example.com/").origin,
            identity("https://example.com/").origin
        );
        assert_ne!(
            identity("http://example.com:8080/").origin,
            identity("http://example.com:8081/").origin
        );
    }
}
