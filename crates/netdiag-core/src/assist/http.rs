//! JSON POST over TLS, for talking to a model provider.
//!
//! The crate deliberately carries no HTTP client dependency — the engine builds
//! with no system libraries, which is what lets it be tested on three platforms
//! without a toolchain. That rule stands here, so this is the same hand-rolled
//! shape as [`crate::scan::http`], extended with the two things a model API
//! needs and a scan never did: a request body, and chunked response decoding.
//!
//! Certificates are verified. The scanner's permissive verifier exists to
//! inspect whatever a LAN device presents; this path carries an API key, so it
//! uses [`crate::scan::http::verified_tls_config`] and nothing else.

use std::collections::BTreeMap;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use rustls::pki_types::ServerName;

/// A parsed response. `status` is checked by callers before trusting `body`:
/// provider errors arrive as a 4xx with a JSON body explaining why.
pub struct JsonResponse {
    pub status: u16,
    pub body: String,
}

/// POSTs a JSON body and returns the response.
///
/// `headers` carries the provider's auth and version headers. It is never
/// logged — an API key lives in there.
pub async fn post_json(
    url: &str,
    body: &str,
    headers: &[(&str, &str)],
    timeout: Duration,
    limit: usize,
) -> Result<JsonResponse, String> {
    let target = Target::parse(url)?;
    let request = target.build_request(body, headers);

    let work = async move {
        let stream = TcpStream::connect((target.host.as_str(), target.port))
            .await
            .map_err(|e| format!("could not reach {}: {e}", target.host))?;

        let raw = if target.secure {
            use tokio_rustls::TlsConnector;
            let connector = TlsConnector::from(crate::scan::http::verified_tls_config());
            let name = ServerName::try_from(target.host.clone())
                .map_err(|_| "invalid host".to_string())?;
            let mut tls = connector
                .connect(name, stream)
                .await
                .map_err(|e| format!("TLS verification failed: {e}"))?;
            tls.write_all(request.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
            read_to_end(&mut tls, limit).await
        } else {
            // Plain HTTP is reached only for a loopback provider such as
            // Ollama; a remote provider would be carrying an API key.
            let mut stream = stream;
            stream
                .write_all(request.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
            read_to_end(&mut stream, limit).await
        };

        parse(&raw).ok_or_else(|| "malformed HTTP response".to_string())
    };

    tokio::time::timeout(timeout, work)
        .await
        .map_err(|_| format!("no response within {}s", timeout.as_secs()))?
}

struct Target {
    secure: bool,
    host: String,
    port: u16,
    path: String,
}

impl Target {
    fn parse(url: &str) -> Result<Self, String> {
        let (secure, rest) = if let Some(rest) = url.strip_prefix("https://") {
            (true, rest)
        } else if let Some(rest) = url.strip_prefix("http://") {
            (false, rest)
        } else {
            return Err(format!("unsupported URL scheme: {url}"));
        };

        let (authority, path) = match rest.find('/') {
            Some(index) => (&rest[..index], &rest[index..]),
            None => (rest, "/"),
        };

        let default_port = if secure { 443 } else { 80 };
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (host.to_string(), port.parse().unwrap_or(default_port)),
            None => (authority.to_string(), default_port),
        };

        if host.is_empty() {
            return Err(format!("no host in URL: {url}"));
        }

        Ok(Self {
            secure,
            host,
            port,
            path: path.to_string(),
        })
    }

    fn build_request(&self, body: &str, headers: &[(&str, &str)]) -> String {
        let mut request = format!(
            "POST {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: local-network-diag\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
            self.path,
            self.host,
            body.len()
        );
        for (key, value) in headers {
            request.push_str(&format!("{key}: {value}\r\n"));
        }
        request.push_str("\r\n");
        request.push_str(body);
        request
    }
}

async fn read_to_end<S>(stream: &mut S, limit: usize) -> String
where
    S: AsyncReadExt + Unpin,
{
    let mut buf = Vec::with_capacity(16 * 1024);
    let mut chunk = [0u8; 16 * 1024];

    while buf.len() < limit {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }

    String::from_utf8_lossy(&buf).into_owned()
}

fn parse(raw: &str) -> Option<JsonResponse> {
    let (head, body) = raw.split_once("\r\n\r\n")?;
    let mut lines = head.lines();
    let status: u16 = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;

    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((key, value)) = line.split_once(':') {
            headers.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }

    let chunked = headers
        .get("transfer-encoding")
        .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"));

    Some(JsonResponse {
        status,
        body: if chunked {
            dechunk(body)
        } else {
            body.to_string()
        },
    })
}

/// Reassembles a `Transfer-Encoding: chunked` body.
///
/// The scanner's parser never needed this: it reads banners and small pages
/// where the body is whatever arrived. A JSON API answer must be exact, and a
/// chunked body left raw is a parse error with hex length markers embedded in
/// it — which reads as "the model returned nonsense" rather than "we did not
/// decode the transport".
fn dechunk(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;

    while let Some((header, tail)) = rest.split_once("\r\n") {
        // A chunk header may carry extensions after a semicolon.
        let size_text = header.split(';').next().unwrap_or("").trim();
        let Ok(size) = usize::from_str_radix(size_text, 16) else {
            break;
        };
        if size == 0 {
            break;
        }
        if tail.len() < size {
            // Truncated — return what is decodable rather than nothing, so a
            // size-limited read still surfaces a useful error to the caller.
            out.push_str(tail);
            break;
        }
        out.push_str(&tail[..size]);
        rest = tail[size..].strip_prefix("\r\n").unwrap_or(&tail[size..]);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chunked_body_is_reassembled() {
        // 5 bytes then 3: `{"a":` + `1}\n`.
        let raw = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
                   5\r\n{\"a\":\r\n3\r\n1}\n\r\n0\r\n\r\n";
        let response = parse(raw).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, "{\"a\":1}\n");
    }

    #[test]
    fn an_identity_body_passes_through_untouched() {
        let raw = "HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\n{\"a\":1}";
        assert_eq!(parse(raw).unwrap().body, "{\"a\":1}");
    }

    #[test]
    fn chunk_extensions_do_not_break_the_length() {
        let raw = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
                   3;name=value\r\nabc\r\n0\r\n\r\n";
        assert_eq!(parse(raw).unwrap().body, "abc");
    }

    #[test]
    fn an_error_status_is_reported_with_its_body() {
        // Providers explain refusals and bad requests in the body; discarding
        // it would leave the user with a bare number.
        let raw = "HTTP/1.1 401 Unauthorized\r\nContent-Length: 2\r\n\r\n{}";
        let response = parse(raw).unwrap();
        assert_eq!(response.status, 401);
        assert_eq!(response.body, "{}");
    }

    #[test]
    fn urls_split_into_host_port_and_path() {
        let target = Target::parse("https://api.anthropic.com/v1/messages").unwrap();
        assert!(target.secure);
        assert_eq!(target.host, "api.anthropic.com");
        assert_eq!(target.port, 443);
        assert_eq!(target.path, "/v1/messages");

        let local = Target::parse("http://127.0.0.1:11434/api/chat").unwrap();
        assert!(!local.secure);
        assert_eq!(local.port, 11434);
        assert_eq!(local.path, "/api/chat");
    }

    #[test]
    fn a_request_carries_the_body_length_and_extra_headers() {
        let target = Target::parse("https://example.com/v1/messages").unwrap();
        let request = target.build_request("{\"a\":1}", &[("x-api-key", "secret")]);

        assert!(request.starts_with("POST /v1/messages HTTP/1.1\r\n"));
        assert!(request.contains("Content-Length: 7\r\n"));
        assert!(request.contains("x-api-key: secret\r\n"));
        assert!(request.ends_with("\r\n\r\n{\"a\":1}"));
    }

    #[test]
    fn an_unsupported_scheme_is_refused() {
        assert!(Target::parse("ftp://example.com").is_err());
        assert!(Target::parse("api.anthropic.com/v1").is_err());
    }
}
