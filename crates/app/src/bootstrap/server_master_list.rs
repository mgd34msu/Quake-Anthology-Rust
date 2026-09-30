//! HTTP server master lists.
//!
//! Donor: `src/app/bootstrap/server-master-list.ts`
//! (`fetchServerMasterList`, q2 `ParseMasterPlain`). Async `fetch` +
//! `AbortSignal` become a sync caller-supplied byte supplier; the
//! supplier owns timeouts and cancellation.

use thiserror::Error;

use crate::bootstrap::server_browser_addresses::direct_server_text;

/// Master list HTTP response (status plus bounded body bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MasterHttpResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response body bytes.
    pub body: Vec<u8>,
}

/// Master list failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MasterListError {
    /// URL is malformed.
    #[error("Invalid master list URL")]
    BadUrl,
    /// URL is not HTTP or HTTPS.
    #[error("Master lists require HTTP or HTTPS")]
    BadScheme,
    /// HTTP status is not OK.
    #[error("Master list returned HTTP {0}")]
    BadStatus(u16),
    /// Body exceeds 1 MiB.
    #[error("Master list exceeds 1 MiB")]
    TooLarge,
    /// Body is not valid UTF-8.
    #[error("Master list is not valid UTF-8")]
    BadUtf8,
    /// List exceeds 4096 servers.
    #[error("Master list exceeds 4096 servers")]
    TooMany,
    /// Supplier fetch failure.
    #[error("Master list fetch failed: {0}")]
    Fetch(String),
    /// Invalid direct server line (sibling message preserved).
    #[error("{0}")]
    BadServer(String),
}

/// Fetch and parse a plain-text master list.
pub fn fetch_server_master_list(
    url: &str,
    fetch: &mut dyn FnMut(&str) -> Result<MasterHttpResponse, MasterListError>,
) -> Result<Vec<String>, MasterListError> {
    let Some(scheme_end) = url.find("://") else {
        return Err(MasterListError::BadUrl);
    };
    if !["http", "https"].contains(&url[..scheme_end].to_ascii_lowercase().as_str()) {
        return Err(MasterListError::BadScheme);
    }
    let after_scheme = &url[scheme_end + 3..];
    let host = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() {
        return Err(MasterListError::BadUrl);
    }
    let response = fetch(url)?;
    if !(200..=299).contains(&response.status) {
        return Err(MasterListError::BadStatus(response.status));
    }
    if response.body.len() > 1024 * 1024 {
        return Err(MasterListError::TooLarge);
    }
    let text = String::from_utf8(response.body).map_err(|_| MasterListError::BadUtf8)?;
    let mut seen = std::collections::HashSet::new();
    let mut servers = Vec::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line).trim();
        if line.is_empty() || line.starts_with('#') || !seen.insert(line.to_owned()) {
            continue;
        }
        servers.push(line.to_owned());
    }
    if servers.len() > 4096 {
        return Err(MasterListError::TooMany);
    }
    servers
        .iter()
        .map(|line| direct_server_text(line).map_err(|error| MasterListError::BadServer(error.to_string())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn supplier(status: u16, body: &[u8]) -> impl FnMut(&str) -> Result<MasterHttpResponse, MasterListError> {
        let body = body.to_vec();
        move |_| Ok(MasterHttpResponse { status, body: body.clone() })
    }

    #[test]
    fn parses_plain_list() {
        let mut fetch = supplier(200, b"# comment\r\nexample.com:27960\n\n  example.com:27960  \n# again\n10.0.0.1\n");
        let servers = fetch_server_master_list("https://master.example/list", &mut fetch).unwrap();
        assert_eq!(servers, vec!["example.com:27960".to_owned(), "10.0.0.1".to_owned()]);
    }

    #[test]
    fn rejects_bad_urls_and_status() {
        let mut fetch = supplier(200, b"a");
        assert_eq!(
            fetch_server_master_list("ftp://x/list", &mut fetch),
            Err(MasterListError::BadScheme)
        );
        assert_eq!(fetch_server_master_list("notaurl", &mut fetch), Err(MasterListError::BadUrl));
        assert_eq!(fetch_server_master_list("http://", &mut fetch), Err(MasterListError::BadUrl));
        let mut fetch = supplier(404, b"nope");
        assert_eq!(
            fetch_server_master_list("http://x/list", &mut fetch),
            Err(MasterListError::BadStatus(404))
        );
    }

    #[test]
    fn enforces_bounds() {
        let mut fetch = supplier(200, &vec![b'a'; 1024 * 1024 + 1]);
        assert_eq!(
            fetch_server_master_list("http://x/list", &mut fetch),
            Err(MasterListError::TooLarge)
        );
        let mut fetch = supplier(200, &[0xff, 0xfe]);
        assert_eq!(
            fetch_server_master_list("http://x/list", &mut fetch),
            Err(MasterListError::BadUtf8)
        );
        let big = (0..4097).map(|i| format!("s{i}")).collect::<Vec<_>>().join("\n");
        let mut fetch = supplier(200, big.as_bytes());
        assert_eq!(
            fetch_server_master_list("http://x/list", &mut fetch),
            Err(MasterListError::TooMany)
        );
        let mut fetch = supplier(200, b"has space");
        assert!(fetch_server_master_list("http://x/list", &mut fetch).is_err());
    }
}
