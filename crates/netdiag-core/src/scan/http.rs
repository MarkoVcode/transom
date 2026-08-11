//! A deliberately small HTTP/1.1 client.
//!
//! Banner grabbing needs status line, a few headers and a bounded slice of the
//! body — nothing that justifies pulling a full HTTP stack and its TLS
//! verification machinery into the binary. Certificates here are *inspected*,
//! never trusted, so verification is intentionally disabled: self-signed certs
//! are the norm on LAN devices and are exactly what we want to report on.

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: String,
}

/// Accepts any certificate. This client only ever *inspects* certificates for
/// reporting; it never transmits credentials, so trust decisions are irrelevant
/// and rejecting self-signed LAN certs would defeat the purpose.
#[derive(Debug)]
struct InspectOnlyVerifier;

impl ServerCertVerifier for InspectOnlyVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ED25519,
        ]
    }
}

pub fn tls_config() -> Arc<ClientConfig> {
    let mut config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(InspectOnlyVerifier))
        .with_no_client_auth();
    config.enable_sni = false;
    Arc::new(config)
}

fn build_request(host: &str, path: &str) -> String {
    format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: transom/1.0\r\nAccept: text/html,*/*\r\nConnection: close\r\n\r\n"
    )
}

pub(crate) fn parse_response(raw: &str) -> Option<HttpResponse> {
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw, ""));
    let mut lines = head.lines();
    let status_line = lines.next()?;

    // "HTTP/1.1 301 Moved Permanently"
    let status: u16 = status_line.split_whitespace().nth(1)?.parse().ok()?;

    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((key, value)) = line.split_once(':') {
            headers.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }

    // A chunked body is not the body: it is the body wrapped in framing. Left
    // undecoded, the first thing any parser sees is a hex chunk length, which is
    // why this surfaced as "could not reach GitHub" rather than as a parse error
    // — the API switches to chunked once a response grows past a few tens of KB,
    // so it stayed invisible while the release payload was small.
    let chunked = headers
        .get("transfer-encoding")
        .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"));

    Some(HttpResponse {
        status,
        headers,
        body: if chunked {
            decode_chunked(body)
        } else {
            body.to_string()
        },
    })
}

/// Reassembles a `Transfer-Encoding: chunked` body.
///
/// Returns whatever it could decode rather than failing. Every caller here reads
/// a bounded prefix of the response, so a final chunk cut short by that limit is
/// the expected case, not a malformed server.
///
/// Indexing is done on bytes because chunk lengths count bytes: a chunk boundary
/// can fall inside a multi-byte character, and slicing the `&str` there would
/// panic.
fn decode_chunked(body: &str) -> String {
    let bytes = body.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut pos = 0usize;

    while pos < bytes.len() {
        let Some(offset) = bytes[pos..].windows(2).position(|w| w == b"\r\n") else {
            break;
        };
        let line = &bytes[pos..pos + offset];

        // A chunk length may carry extensions: "1a;name=value".
        let token = line.split(|b| *b == b';').next().unwrap_or_default();
        let Ok(token) = std::str::from_utf8(token) else {
            break;
        };
        let Ok(size) = usize::from_str_radix(token.trim(), 16) else {
            break;
        };
        if size == 0 {
            break; // terminating chunk
        }

        let start = pos + offset + 2;
        let end = start.saturating_add(size).min(bytes.len());
        out.extend_from_slice(&bytes[start..end]);

        if end == bytes.len() {
            break; // truncated by the read limit
        }
        pos = end + 2; // step over the chunk's trailing CRLF
    }

    String::from_utf8_lossy(&out).into_owned()
}

async fn read_bounded<S>(stream: &mut S, limit: usize) -> String
where
    S: AsyncReadExt + Unpin,
{
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];

    while buf.len() < limit {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }

    String::from_utf8_lossy(&buf).into_owned()
}

pub async fn fetch(
    host: &str,
    port: u16,
    secure: bool,
    timeout: Duration,
    limit: usize,
) -> Result<HttpResponse, String> {
    let work = async {
        let stream = TcpStream::connect((host, port))
            .await
            .map_err(|e| e.to_string())?;
        let request = build_request(&format!("{host}:{port}"), "/");

        let raw = if secure {
            use tokio_rustls::TlsConnector;
            let connector = TlsConnector::from(tls_config());
            // An IP is not a valid SNI name; SNI is disabled in the config and a
            // placeholder name satisfies the API without being sent.
            let server_name = ServerName::try_from("scan.invalid").map_err(|e| e.to_string())?;
            let mut tls = connector
                .connect(server_name, stream)
                .await
                .map_err(|e| e.to_string())?;
            tls.write_all(request.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
            read_bounded(&mut tls, limit).await
        } else {
            let mut stream = stream;
            stream
                .write_all(request.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
            read_bounded(&mut stream, limit).await
        };

        parse_response(&raw).ok_or_else(|| "malformed HTTP response".to_string())
    };

    tokio::time::timeout(timeout, work)
        .await
        .map_err(|_| "timed out".to_string())?
}

/// Convenience wrapper for fetching a URL, used for SSDP description XML.
pub async fn get_text(url: &str, timeout: Duration, limit: usize) -> Result<String, String> {
    let (secure, rest) = if let Some(rest) = url.strip_prefix("http://") {
        (false, rest)
    } else if let Some(rest) = url.strip_prefix("https://") {
        (true, rest)
    } else {
        return Err("unsupported scheme".into());
    };

    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, "/"),
    };

    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (
            h.to_string(),
            p.parse().unwrap_or(if secure { 443 } else { 80 }),
        ),
        None => (authority.to_string(), if secure { 443 } else { 80 }),
    };

    let work = async {
        let stream = TcpStream::connect((host.as_str(), port))
            .await
            .map_err(|e| e.to_string())?;
        let request = build_request(authority, path);

        let raw = if secure {
            use tokio_rustls::TlsConnector;
            let connector = TlsConnector::from(tls_config());
            let server_name = ServerName::try_from("scan.invalid").map_err(|e| e.to_string())?;
            let mut tls = connector
                .connect(server_name, stream)
                .await
                .map_err(|e| e.to_string())?;
            tls.write_all(request.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
            read_bounded(&mut tls, limit).await
        } else {
            let mut stream = stream;
            stream
                .write_all(request.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
            read_bounded(&mut stream, limit).await
        };

        Ok::<String, String>(parse_response(&raw).map(|r| r.body).unwrap_or(raw))
    };

    tokio::time::timeout(timeout, work)
        .await
        .map_err(|_| "timed out".to_string())?
}

/// Extracts `<title>` from an HTML body, collapsing whitespace.
pub fn extract_title(body: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    let start_tag = lower.find("<title")?;
    let close = lower[start_tag..].find('>')? + start_tag + 1;
    let end = lower[close..].find("</title>")? + close;

    let title = body[close..end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let trimmed = title.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.chars().take(200).collect())
    }
}

/* ------------------------------------------------------- verified public HTTPS */

/// TLS config that performs **real** certificate validation against the bundled
/// CA roots.
///
/// This is the opposite of [`tls_config`] and the distinction is deliberate.
/// [`tls_config`] accepts any certificate because it inspects LAN devices that
/// legitimately present self-signed ones, and it never sends them anything. This
/// one is for talking to a public service on the internet, where accepting any
/// certificate would let anyone on the path impersonate it.
/// Real certificate verification, for talking to the public internet.
///
/// Distinct from [`tls_config`], which accepts anything: that one exists to
/// *inspect* LAN certificates and must not be used where credentials are sent.
pub fn verified_tls_config() -> Arc<ClientConfig> {
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    Arc::new(
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

/// GETs a public HTTPS URL with full certificate verification.
///
/// Used for the update check. Follows one redirect, because GitHub's API
/// redirects `/releases/latest` to the concrete release.
pub async fn get_text_public(
    url: &str,
    timeout: Duration,
    limit: usize,
    extra_headers: &[(&str, &str)],
) -> Result<String, String> {
    let mut current = url.to_string();

    for _ in 0..3 {
        let rest = current
            .strip_prefix("https://")
            .ok_or_else(|| "only https is supported".to_string())?;

        let (authority, path) = match rest.find('/') {
            Some(index) => (&rest[..index], &rest[index..]),
            None => (rest, "/"),
        };
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse().unwrap_or(443)),
            None => (authority.to_string(), 443u16),
        };

        let host_for_sni = host.clone();
        let request_path = path.to_string();
        let headers = extra_headers.to_vec();

        let work = async move {
            let stream = TcpStream::connect((host_for_sni.as_str(), port))
                .await
                .map_err(|e| e.to_string())?;

            use tokio_rustls::TlsConnector;
            let connector = TlsConnector::from(verified_tls_config());
            let server_name =
                ServerName::try_from(host_for_sni.clone()).map_err(|e| e.to_string())?;
            let mut tls = connector
                .connect(server_name, stream)
                .await
                .map_err(|e| format!("TLS verification failed: {e}"))?;

            // GitHub rejects requests without a User-Agent.
            let mut request = format!(
                "GET {request_path} HTTP/1.1\r\nHost: {host_for_sni}\r\n\
                 User-Agent: transom\r\nConnection: close\r\n"
            );
            for (key, value) in headers {
                request.push_str(&format!("{key}: {value}\r\n"));
            }
            request.push_str("\r\n");

            tls.write_all(request.as_bytes())
                .await
                .map_err(|e| e.to_string())?;

            Ok::<String, String>(read_bounded_generic(&mut tls, limit).await)
        };

        let raw = tokio::time::timeout(timeout, work)
            .await
            .map_err(|_| "timed out".to_string())??;

        let response = parse_response(&raw).ok_or_else(|| "malformed response".to_string())?;

        match response.status {
            200 => return Ok(response.body),
            301 | 302 | 307 | 308 => {
                let Some(location) = response.headers.get("location") else {
                    return Err("redirect without a location".into());
                };
                current = location.clone();
            }
            other => return Err(format!("HTTP {other}")),
        }
    }

    Err("too many redirects".into())
}

async fn read_bounded_generic<S>(stream: &mut S, limit: usize) -> String
where
    S: AsyncReadExt + Unpin,
{
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 8192];

    while buf.len() < limit {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }

    String::from_utf8_lossy(&buf).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status_line_and_headers() {
        let raw = "HTTP/1.1 301 Moved Permanently\r\nLocation: https://10.0.3.1/\r\nServer: Server\r\n\r\n<html></html>";
        let response = parse_response(raw).unwrap();
        assert_eq!(response.status, 301);
        assert_eq!(
            response.headers.get("location").unwrap(),
            "https://10.0.3.1/"
        );
        assert_eq!(response.headers.get("server").unwrap(), "Server");
    }

    #[test]
    fn extracts_title_with_attributes_and_whitespace() {
        assert_eq!(
            extract_title("<html><title>UniFi</title>"),
            Some("UniFi".into())
        );
        assert_eq!(
            extract_title("<TITLE lang=\"en\">  Router\n  Login  </TITLE>"),
            Some("Router Login".into())
        );
        assert_eq!(extract_title("<html>no title</html>"), None);
        assert_eq!(extract_title("<title>   </title>"), None);
    }

    #[test]
    fn malformed_responses_are_rejected_not_panicked() {
        assert!(parse_response("").is_none());
        assert!(parse_response("garbage").is_none());
        assert!(parse_response("HTTP/1.1\r\n\r\n").is_none());
    }

    #[test]
    fn a_chunked_body_is_reassembled() {
        // The shape GitHub's API returns over HTTP/1.1 once a response grows
        // past a few tens of KB. Undecoded, the body starts with "1a" and no
        // JSON parser gets past the first two bytes — which is how a working
        // request came to be reported as "could not reach GitHub".
        let raw = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
                   15\r\n{\"tag_name\":\"v1.8.0\",\r\n\
                   e\r\n\"draft\":false}\r\n\
                   0\r\n\r\n";

        let response = parse_response(raw).unwrap();
        assert_eq!(response.body, "{\"tag_name\":\"v1.8.0\",\"draft\":false}");
        assert!(
            serde_json::from_str::<serde_json::Value>(&response.body).is_ok(),
            "the decoded body must be parseable JSON"
        );
    }

    #[test]
    fn chunk_extensions_and_uppercase_hex_are_understood() {
        let raw = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
                   A;name=value\r\n0123456789\r\n\
                   0\r\n\r\n";
        assert_eq!(parse_response(raw).unwrap().body, "0123456789");
    }

    #[test]
    fn a_body_cut_short_by_the_read_limit_keeps_what_arrived() {
        // read_bounded stops at its limit, so the last chunk is routinely
        // truncated. That must yield a short body, not an empty one.
        let raw = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
                   ff\r\ntruncated here";
        assert_eq!(parse_response(raw).unwrap().body, "truncated here");
    }

    #[test]
    fn a_chunk_boundary_inside_a_multibyte_character_does_not_panic() {
        // The em dash is three bytes; splitting it across chunks is legal and
        // must not panic the way slicing a &str on that index would.
        let raw = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
                   2\r\n\u{2014}\r\n\
                   0\r\n\r\n";
        let _ = parse_response(raw).unwrap().body;
    }

    #[test]
    fn an_unchunked_body_is_left_alone() {
        let raw = "HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nbody";
        assert_eq!(parse_response(raw).unwrap().body, "body");
    }

    #[tokio::test]
    async fn fetches_from_a_local_server() {
        use tokio::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut discard = [0u8; 1024];
                let _ = socket.read(&mut discard).await;
                let _ = socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nServer: test-server\r\nContent-Type: text/html\r\n\r\n<title>Test Device</title>",
                    )
                    .await;
            }
        });

        let response = fetch("127.0.0.1", port, false, Duration::from_secs(3), 65536)
            .await
            .unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.headers.get("server").unwrap(), "test-server");
        assert_eq!(
            extract_title(&response.body).as_deref(),
            Some("Test Device")
        );
    }
}
