//! Prometheus Provider (WM-0404; docs/basin.md "Providers": "Prometheus
//! Provider: implement first", "Custom Provider: keep a unified interface").
//!
//! Basin "is not a monitoring product and does not reimplement Prometheus":
//! the provider is a thin query client over Prometheus's HTTP API. It
//! serves RAW series — labels and values exactly as Prometheus reports
//! them — to the metric mapping (WM-0405), which is where names gain
//! catalog meaning. Transport is plain HTTP/1.1 over std::net (no TLS in
//! this pass; TLS deployments go through a custom transport or the custom
//! provider kind, and that boundary is stated rather than hidden).

use std::fmt;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::Labels;

/// Raw sample exactly as the source produced it (its timestamp semantics
/// — source_timestamp per WM-0407 — are carried, not reinterpreted).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub timestamp_unix_ms: i64,
    pub value: f64,
}

/// One raw series: the label map as Prometheus sent it (including
/// `__name__`) plus its samples in timestamp order. The provider does not
/// interpret labels — that is the mapping's job (WM-0405).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderSeries {
    #[serde(default)]
    pub metric: Labels,
    #[serde(default)]
    pub samples: Vec<Sample>,
}

/// The query window the scheduler will drive (WM-0406); recorded alongside
/// results because docs/basin.md's timestamp model requires "query time
/// window" to be reportable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryWindow {
    pub start_unix_ms: i64,
    pub end_unix_ms: i64,
    pub step_ms: i64,
}

impl QueryWindow {
    pub fn validate(&self) -> Result<(), ProviderError> {
        if self.start_unix_ms > self.end_unix_ms {
            return Err(ProviderError::InvalidWindow(format!(
                "start {} after end {}",
                self.start_unix_ms, self.end_unix_ms
            )));
        }
        if self.step_ms <= 0 {
            return Err(ProviderError::InvalidWindow(format!(
                "step must be positive, got {}",
                self.step_ms
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    #[error("metric provider endpoint is not reachable: {0}")]
    Unreachable(String),
    #[error("metric provider returned http status {0}")]
    BadStatus(u16),
    #[error("prometheus query failed: {0}")]
    QueryFailed(String),
    #[error("unexpected prometheus response: {0}")]
    BadResponse(String),
    #[error("invalid query window: {0}")]
    InvalidWindow(String),
    #[error("unsupported url scheme {0}: provider transport is plain http")]
    UnsupportedScheme(String),
}

/// The unified provider interface (docs/basin.md "Custom Provider: keep a
/// unified interface"). Prometheus implements it first; OTel and custom
/// providers plug into the same shape.
pub trait MetricProvider {
    /// Range query over the window; returns raw series (mapping WM-0405
    /// binds them to catalog entries).
    fn query_range(
        &self,
        query: &str,
        window: &QueryWindow,
    ) -> Result<Vec<ProviderSeries>, ProviderError>;

    /// Reachability probe for Preflight (docs/basin.md: "Providers are
    /// reachable").
    fn ready(&self) -> Result<(), ProviderError>;
}

/// Minimal HTTP GET transport; unit of the plain-HTTP boundary above.
pub trait HttpTransport {
    fn get(&self, url: &str) -> Result<HttpResponse, ProviderError>;
}

pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

// --- Prometheus HTTP API -------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedUrl {
    pub(crate) host: String,
    pub(crate) port: u16,
    /// Path incl. leading '/', without scheme/host; empty string = "/".
    pub(crate) path: String,
}

pub(crate) fn parse_url(url: &str) -> Result<ParsedUrl, ProviderError> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| ProviderError::BadResponse(format!("malformed url {url}")))?;
    if scheme != "http" {
        return Err(ProviderError::UnsupportedScheme(scheme.to_string()));
    }
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (
            h.to_string(),
            p.parse::<u16>()
                .map_err(|_| ProviderError::BadResponse(format!("malformed port in {url}")))?,
        ),
        None => (authority.to_string(), 80),
    };
    if host.is_empty() {
        return Err(ProviderError::BadResponse(format!(
            "malformed host in {url}"
        )));
    }
    Ok(ParsedUrl {
        host,
        port,
        path: path.to_string(),
    })
}

/// Percent-encode a query-string value (RFC 3986 unreserved set kept).
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Prometheus API wants seconds; millisecond precision survives as the
/// fractional part (1710000000123ms -> "1710000000.123").
fn unix_ms_to_seconds_string(unix_ms: i64) -> String {
    let seconds = unix_ms / 1000;
    let millis = unix_ms % 1000;
    if millis == 0 {
        format!("{seconds}")
    } else {
        format!("{}.{:03}", seconds, millis.abs())
    }
}

fn query_range_url(
    base_url: &str,
    query: &str,
    window: &QueryWindow,
) -> Result<String, ProviderError> {
    window.validate()?;
    Ok(format!(
        "{}{}?query={}&start={}&end={}&step={}",
        base_url.trim_end_matches('/'),
        "/api/v1/query_range",
        percent_encode(query),
        unix_ms_to_seconds_string(window.start_unix_ms),
        unix_ms_to_seconds_string(window.end_unix_ms),
        unix_ms_to_seconds_string(window.step_ms),
    ))
}

fn ready_url(base_url: &str) -> String {
    format!("{}/-/ready", base_url.trim_end_matches('/'))
}

// --- API response parsing ------------------------------------------------

#[derive(Deserialize)]
struct ApiEnvelope {
    status: String,
    #[serde(default)]
    error: String,
    #[serde(default)]
    data: Option<ApiData>,
}

#[derive(Deserialize)]
struct ApiData {
    #[serde(default, rename = "resultType")]
    result_type: String,
    #[serde(default)]
    result: serde_json::Value,
}

fn parse_query_response(body: &[u8]) -> Result<Vec<ProviderSeries>, ProviderError> {
    let envelope: ApiEnvelope = serde_json::from_slice(body)
        .map_err(|err| ProviderError::BadResponse(format!("body is not valid JSON: {err}")))?;
    if envelope.status != "success" {
        if envelope.error.is_empty() {
            return Err(ProviderError::BadResponse(format!(
                "status {:?} without an error message",
                envelope.status
            )));
        }
        return Err(ProviderError::QueryFailed(envelope.error));
    }
    let data = envelope
        .data
        .ok_or_else(|| ProviderError::BadResponse("success response without data".to_string()))?;
    if data.result_type != "matrix" {
        return Err(ProviderError::BadResponse(format!(
            "expected resultType matrix, got {:?}",
            data.result_type
        )));
    }
    let entries = data
        .result
        .as_array()
        .ok_or_else(|| ProviderError::BadResponse("matrix result is not an array".to_string()))?;
    let mut series = Vec::with_capacity(entries.len());
    for entry in entries {
        let labels_json = entry
            .get("metric")
            .ok_or_else(|| ProviderError::BadResponse("matrix entry without metric".to_string()))?;
        let metric: Labels = serde_json::from_value(labels_json.clone()).map_err(|err| {
            ProviderError::BadResponse(format!("metric labels are not a string map: {err}"))
        })?;
        let values = entry
            .get("values")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ProviderError::BadResponse("matrix entry without values".to_string()))?;
        let mut samples = Vec::with_capacity(values.len());
        for value in values {
            let pair = value.as_array().ok_or_else(|| {
                ProviderError::BadResponse("sample is not a [timestamp, value] pair".to_string())
            })?;
            if pair.len() != 2 {
                return Err(ProviderError::BadResponse(
                    "sample is not a [timestamp, value] pair".to_string(),
                ));
            }
            let ts_seconds = pair[0].as_f64().ok_or_else(|| {
                ProviderError::BadResponse("sample timestamp is not a number".to_string())
            })?;
            let text = pair[1].as_str().ok_or_else(|| {
                ProviderError::BadResponse("sample value is not a string".to_string())
            })?;
            let parsed: f64 = text.parse().map_err(|_| {
                ProviderError::BadResponse(format!("sample value {text:?} is not a number"))
            })?;
            samples.push(Sample {
                timestamp_unix_ms: (ts_seconds * 1000.0).round() as i64,
                value: parsed,
            });
        }
        series.push(ProviderSeries { metric, samples });
    }
    Ok(series)
}

// --- Prometheus provider --------------------------------------------------

pub struct PrometheusProvider {
    base_url: String,
    transport: Box<dyn HttpTransport>,
}

impl PrometheusProvider {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            transport: Box::new(TcpHttpTransport::default()),
        }
    }

    /// Inject a transport (tests; or TLS via an external impl).
    pub fn with_transport(base_url: impl Into<String>, transport: Box<dyn HttpTransport>) -> Self {
        Self {
            base_url: base_url.into(),
            transport,
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

impl MetricProvider for PrometheusProvider {
    fn query_range(
        &self,
        query: &str,
        window: &QueryWindow,
    ) -> Result<Vec<ProviderSeries>, ProviderError> {
        let url = query_range_url(&self.base_url, query, window)?;
        let response = self.transport.get(&url)?;
        if response.status != 200 {
            return Err(ProviderError::BadStatus(response.status));
        }
        parse_query_response(&response.body)
    }

    fn ready(&self) -> Result<(), ProviderError> {
        let response = self.transport.get(&ready_url(&self.base_url))?;
        if response.status == 200 {
            Ok(())
        } else {
            Err(ProviderError::BadStatus(response.status))
        }
    }
}

// --- Plain-HTTP transport over std::net -----------------------------------

/// HTTP/1.1 GET with `Connection: close` (the server closes; the body is
/// read per Content-Length, chunked, or to EOF). No TLS — see the module
/// comment for why that boundary is stated, not hidden.
#[derive(Default)]
pub struct TcpHttpTransport {
    pub timeout: Option<Duration>,
}

impl TcpHttpTransport {
    fn request_target(url: &str) -> Result<(ParsedUrl, String), ProviderError> {
        let parsed = parse_url(url)?;
        let target = if parsed.path.is_empty() {
            "/".to_string()
        } else {
            parsed.path.clone()
        };
        Ok((parsed, target))
    }
}

impl HttpTransport for TcpHttpTransport {
    fn get(&self, url: &str) -> Result<HttpResponse, ProviderError> {
        let (parsed, target) = Self::request_target(url)?;
        let connect = |host: &str, port: u16| -> Result<TcpStream, ProviderError> {
            let stream = match self.timeout {
                Some(timeout) => {
                    let addr = format!("{host}:{port}");
                    let stream = std::net::ToSocketAddrs::to_socket_addrs(&addr)
                        .map_err(|err| ProviderError::Unreachable(format!("{addr}: {err}")))?
                        .next()
                        .ok_or_else(|| {
                            ProviderError::Unreachable(format!("{addr}: no address resolved"))
                        })?;
                    TcpStream::connect_timeout(&stream, timeout)
                }
                None => TcpStream::connect((host, port)),
            };
            stream.map_err(|err| ProviderError::Unreachable(format!("{}:{}: {err}", host, port)))
        };
        let mut stream = connect(&parsed.host, parsed.port)?;

        let request = format!(
            "GET {target} HTTP/1.1\r\nHost: {}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
            parsed.host,
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|err| ProviderError::Unreachable(format!("write: {err}")))?;

        // Read the whole exchange (Connection: close makes EOF the end).
        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .map_err(|err| ProviderError::Unreachable(format!("read: {err}")))?;
        decode_http_response(&raw)
    }
}

impl fmt::Debug for TcpHttpTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TcpHttpTransport")
            .field("timeout", &self.timeout)
            .finish()
    }
}

pub(crate) fn decode_http_response(raw: &[u8]) -> Result<HttpResponse, ProviderError> {
    let header_end = find_header_end(raw)
        .ok_or_else(|| ProviderError::BadResponse("no http header terminator found".to_string()))?;
    let head = String::from_utf8_lossy(&raw[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let status = status_line
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| {
            ProviderError::BadResponse(format!("malformed http status line {status_line:?}"))
        })?;
    let mut content_length: Option<usize> = None;
    let mut chunked = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();
        if name == "content-length" {
            content_length = value.parse().ok();
        } else if name == "transfer-encoding" && value.to_ascii_lowercase().contains("chunked") {
            chunked = true;
        }
    }
    let body_bytes = &raw[header_end + 4..];
    let body = if chunked {
        decode_chunked(body_bytes)?
    } else if let Some(length) = content_length {
        if body_bytes.len() < length {
            return Err(ProviderError::BadResponse(format!(
                "body truncated: {} of {length} bytes",
                body_bytes.len()
            )));
        }
        body_bytes[..length].to_vec()
    } else {
        body_bytes.to_vec()
    };
    Ok(HttpResponse { status, body })
}

fn find_header_end(raw: &[u8]) -> Option<usize> {
    raw.windows(4).position(|window| window == b"\r\n\r\n")
}

fn decode_chunked(mut bytes: &[u8]) -> Result<Vec<u8>, ProviderError> {
    let mut body = Vec::new();
    loop {
        let Some(line_end) = bytes.windows(2).position(|window| window == b"\r\n") else {
            return Err(ProviderError::BadResponse(
                "chunked body: missing chunk size line".to_string(),
            ));
        };
        let size_text = String::from_utf8_lossy(&bytes[..line_end]);
        let size_text = size_text.split(';').next().unwrap_or_default().trim();
        let size = usize::from_str_radix(size_text, 16).map_err(|_| {
            ProviderError::BadResponse(format!("chunked body: bad chunk size {size_text:?}"))
        })?;
        bytes = &bytes[line_end + 2..];
        if size == 0 {
            return Ok(body);
        }
        if bytes.len() < size + 2 {
            return Err(ProviderError::BadResponse(
                "chunked body: chunk truncated".to_string(),
            ));
        }
        body.extend_from_slice(&bytes[..size]);
        bytes = &bytes[size + 2..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_window_rejects_inverted_and_zero_step() {
        let ok = QueryWindow {
            start_unix_ms: 100,
            end_unix_ms: 200,
            step_ms: 50,
        };
        ok.validate().expect("valid window");

        let inverted = QueryWindow {
            start_unix_ms: 200,
            end_unix_ms: 100,
            step_ms: 50,
        };
        assert!(matches!(
            inverted.validate(),
            Err(ProviderError::InvalidWindow(_))
        ));

        let zero_step = QueryWindow { step_ms: 0, ..ok };
        assert!(matches!(
            zero_step.validate(),
            Err(ProviderError::InvalidWindow(_))
        ));
    }

    #[test]
    fn promql_query_is_percent_encoded() {
        let window = QueryWindow {
            start_unix_ms: 1_710_000_000_000,
            end_unix_ms: 1_710_000_060_000,
            step_ms: 15_000,
        };
        let url = query_range_url("http://prometheus:9090/", r#"up{job="kbengine"}"#, &window)
            .expect("url");
        assert!(url.starts_with("http://prometheus:9090/api/v1/query_range?"));
        assert!(url.contains(r#"query=up%7Bjob%3D%22kbengine%22%7D"#));
        assert!(url.contains("start=1710000000"));
        assert!(url.contains("end=1710000060"));
        assert!(url.contains("step=15"));
    }

    #[test]
    fn millisecond_precision_survives_in_seconds_strings() {
        let window = QueryWindow {
            start_unix_ms: 1_710_000_000_123,
            end_unix_ms: 1_710_000_000_500,
            step_ms: 1500,
        };
        let url = query_range_url("http://p:9090", "up", &window).expect("url");
        assert!(url.contains("start=1710000000.123"));
        assert!(url.contains("end=1710000000.500"));
        assert!(url.contains("step=1.500"));
    }

    #[test]
    fn matrix_response_parses_into_raw_series() {
        let body = br#"{
            "status": "success",
            "data": {
                "resultType": "matrix",
                "result": [
                    {
                        "metric": {"__name__": "kbengine_online", "job": "kbengine"},
                        "values": [[1710000000.0, "12"], [1710000015.0, "NaN"], [1710000030.0, "+Inf"]]
                    },
                    {
                        "metric": {"__name__": "kbengine_tick_ms"},
                        "values": [[1710000000.25, "3.5"]]
                    }
                ]
            }
        }"#;
        let series = parse_query_response(body).expect("parse");
        assert_eq!(series.len(), 2);
        assert_eq!(series[0].metric["__name__"], "kbengine_online");
        assert_eq!(series[0].metric["job"], "kbengine");
        assert_eq!(series[0].samples.len(), 3);
        assert_eq!(series[0].samples[0].timestamp_unix_ms, 1_710_000_000_000);
        assert_eq!(series[0].samples[0].value, 12.0);
        assert!(series[0].samples[1].value.is_nan());
        assert!(series[0].samples[2].value.is_infinite());
        assert_eq!(series[1].samples[0].timestamp_unix_ms, 1_710_000_000_250);
        assert_eq!(series[1].samples[0].value, 3.5);
    }

    #[test]
    fn failed_query_status_surfaces_the_error_message() {
        let body = br#"{
            "status": "error",
            "errorType": "bad_data",
            "error": "parse error at char 3"
        }"#;
        assert_eq!(
            parse_query_response(body),
            Err(ProviderError::QueryFailed("parse error at char 3".into()))
        );
    }

    #[test]
    fn non_matrix_result_types_are_refused() {
        let body = br#"{
            "status": "success",
            "data": {"resultType": "vector", "result": []}
        }"#;
        assert!(matches!(
            parse_query_response(body),
            Err(ProviderError::BadResponse(_))
        ));
    }

    #[test]
    fn non_success_status_is_reported() {
        let provider = PrometheusProvider::with_transport(
            "http://prometheus:9090",
            Box::new(FakeTransport {
                status: 503,
                body: br#"{"status":"success","data":{"resultType":"matrix","result":[]}}"#
                    .to_vec(),
            }),
        );
        let window = QueryWindow {
            start_unix_ms: 0,
            end_unix_ms: 1,
            step_ms: 1,
        };
        assert_eq!(
            provider.query_range("up", &window),
            Err(ProviderError::BadStatus(503))
        );
        assert_eq!(provider.ready(), Err(ProviderError::BadStatus(503)));
    }

    #[test]
    fn provider_end_to_end_over_a_fake_transport() {
        let provider = PrometheusProvider::with_transport(
            "http://prometheus:9090",
            Box::new(FakeTransport {
                status: 200,
                body: br#"{"status":"success","data":{"resultType":"matrix","result":[{"metric":{"__name__":"up"},"values":[[1.0,"1"]]}]}}"#.to_vec(),
            }),
        );
        provider.ready().expect("reachable");
        let window = QueryWindow {
            start_unix_ms: 0,
            end_unix_ms: 1000,
            step_ms: 500,
        };
        let series = provider.query_range("up", &window).expect("query");
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].samples[0].value, 1.0);
    }

    #[test]
    fn tls_scheme_is_refused_explicitly() {
        assert!(matches!(
            parse_url("https://prometheus:9090/api/v1/query_range"),
            Err(ProviderError::UnsupportedScheme(_))
        ));
    }

    /// Full transport path against a real local socket: chunked and
    /// content-length responses both decode through the actual reader.
    #[test]
    fn tcp_transport_decodes_chunked_and_content_length_bodies() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let server = std::thread::spawn(move || {
            let payload = br#"{"ok":true}"#.to_vec();
            let serve = |out: &mut Vec<u8>| {
                let (mut sock, _) = listener.accept().expect("accept");
                let mut buf = [0u8; 2048];
                let _ = sock.read(&mut buf); // request bytes; we only respond
                sock.write_all(out).expect("write");
            };
            // chunked variant
            {
                let mut out = Vec::new();
                out.extend_from_slice(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n",
                );
                out.extend_from_slice(format!("{:x}\r\n", payload.len()).as_bytes());
                out.extend_from_slice(&payload);
                out.extend_from_slice(b"\r\n0\r\n\r\n");
                serve(&mut out);
            }
            // content-length variant
            {
                let mut out = Vec::new();
                out.extend_from_slice(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                        payload.len()
                    )
                    .as_bytes(),
                );
                out.extend_from_slice(&payload);
                serve(&mut out);
            }
        });

        let transport = TcpHttpTransport::default();
        let chunked = transport
            .get(&format!(
                "http://127.0.0.1:{port}/api/v1/query_range?query=up"
            ))
            .expect("chunked response");
        assert_eq!(chunked.status, 200);
        assert_eq!(chunked.body, br#"{"ok":true}"#.to_vec());

        let with_length = transport
            .get(&format!("http://127.0.0.1:{port}/-/ready"))
            .expect("content-length response");
        assert_eq!(with_length.status, 200);
        assert_eq!(with_length.body, br#"{"ok":true}"#.to_vec());
        server.join().expect("server thread");
    }

    #[test]
    fn unreachable_endpoint_reports_unreachable() {
        let transport = TcpHttpTransport::default();
        let result = transport.get("http://127.0.0.1:1/nothing");
        assert!(matches!(result, Err(ProviderError::Unreachable(_))));
    }

    struct FakeTransport {
        status: u16,
        body: Vec<u8>,
    }

    impl HttpTransport for FakeTransport {
        fn get(&self, _url: &str) -> Result<HttpResponse, ProviderError> {
            Ok(HttpResponse {
                status: self.status,
                body: self.body.clone(),
            })
        }
    }
}
