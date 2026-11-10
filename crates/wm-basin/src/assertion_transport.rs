//! Assertion Provider transport over plain HTTP (WM-0703;
//! docs/spec/assertion-provider.md §3.1, §3.2, §3.3).
//!
//! Process boundary frozen here: `Execute(name, params, window_context)`
//! is a POST with a JSON body `{"name", "params", "window_context"}`;
//! the response body IS the §3.2 AssertionResult JSON. TLS is out of
//! scope the same way as the metric provider (plain http only).
//!
//! Inconclusive handling is the §3.2 rule made structural: a provider
//! that is unreachable, times out, returns a non-200, returns a body
//! that is not a valid AssertionResult, or returns a record that fails
//! its own integrity checks (wrong name, empty checked_at, no evidence)
//! is rendered through [`inconclusive_no_show`] — there is NO path that
//! lets "did not run" become "pass". A record the provider did complete
//! is taken verbatim: pass/fail/inconclusive are the provider's
//! judgment, and Wavemaker does not reinterpret business semantics.
//! `checked_at` on synthesized records comes from the injected clock —
//! the transport never invents timestamps.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::json;

use crate::assertions::{
    inconclusive_no_show, AssertionProviderContract, AssertionRecord, WindowContext,
};
use crate::provider::{decode_http_response, parse_url, HttpResponse, ProviderError};

/// HTTP seam for the assertion boundary (unit-testable without sockets).
pub trait AssertionHttpTransport {
    fn post(&self, url: &str, body: &str) -> Result<HttpResponse, ProviderError>;
}

/// Plain HTTP/1.1 POST transport over std::net — the metric provider's
/// transport discipline (Connection: close, optional timeout on connect,
/// read, and write) applied to the assertion boundary.
#[derive(Debug, Clone)]
pub struct TcpPostTransport {
    pub timeout: Option<Duration>,
}

impl AssertionHttpTransport for TcpPostTransport {
    fn post(&self, url: &str, body: &str) -> Result<HttpResponse, ProviderError> {
        let parsed = parse_url(url)?;
        let addr = format!("{}:{}", parsed.host, parsed.port);
        let stream = match self.timeout {
            Some(timeout) => {
                let resolved = std::net::ToSocketAddrs::to_socket_addrs(&addr)
                    .map_err(|err| ProviderError::Unreachable(format!("{addr}: {err}")))?
                    .next()
                    .ok_or_else(|| {
                        ProviderError::Unreachable(format!("{addr}: no address resolved"))
                    })?;
                TcpStream::connect_timeout(&resolved, timeout)
            }
            None => TcpStream::connect(&addr),
        }
        .map_err(|err| ProviderError::Unreachable(format!("{addr}: {err}")))?;
        stream
            .set_read_timeout(self.timeout)
            .map_err(|err| ProviderError::Unreachable(format!("{addr}: {err}")))?;
        stream
            .set_write_timeout(self.timeout)
            .map_err(|err| ProviderError::Unreachable(format!("{addr}: {err}")))?;

        let request = format!(
            "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            parsed.path,
            parsed.host,
            body.len(),
            body,
        );
        let mut stream = stream;
        stream
            .write_all(request.as_bytes())
            .map_err(|err| ProviderError::Unreachable(format!("write: {err}")))?;

        // Connection: close makes EOF the end of the exchange.
        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .map_err(|err| ProviderError::Unreachable(format!("read: {err}")))?;
        decode_http_response(&raw)
    }
}

/// What the transport tells execute() about a failed exchange — the
/// reason text goes verbatim into the inconclusive record's message.
fn failure_reason(error: &ProviderError) -> String {
    match error {
        ProviderError::BadStatus(status) => {
            format!("provider returned http status {status}")
        }
        other => format!("provider exchange failed: {other}"),
    }
}

/// A body excerpt for error messages — long bodies are truncated so a
/// huge HTML error page cannot flood the report.
fn body_excerpt(body: &[u8]) -> String {
    const MAX: usize = 512;
    let text = String::from_utf8_lossy(body);
    if text.len() <= MAX {
        text.into_owned()
    } else {
        format!("{}…(truncated)", &text[..MAX])
    }
}

/// The HTTP Assertion Provider: resolves `Execute` against `endpoint`.
pub struct HttpAssertionProvider {
    endpoint: String,
    transport: Box<dyn AssertionHttpTransport>,
    /// RFC3339 clock for records the transport itself synthesizes
    /// (unreachable, timeout, malformed). Provider-completed records
    /// carry the provider's own `checked_at`.
    clock: Box<dyn Fn() -> String + Send + Sync>,
}

impl std::fmt::Debug for HttpAssertionProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpAssertionProvider")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl HttpAssertionProvider {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            transport: Box::new(TcpPostTransport { timeout: None }),
            clock: Box::new(String::new),
        }
    }

    pub fn with_transport(
        endpoint: impl Into<String>,
        transport: Box<dyn AssertionHttpTransport>,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            transport,
            clock: Box::new(String::new),
        }
    }

    pub fn with_clock(mut self, clock: Box<dyn Fn() -> String + Send + Sync>) -> Self {
        self.clock = clock;
        self
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The no-show record this transport renders when the exchange
    /// cannot produce a trustworthy result.
    fn no_show(&self, name: &str, reason: &str) -> AssertionRecord {
        inconclusive_no_show("http", name, &(self.clock)(), reason)
    }
}

impl AssertionProviderContract for HttpAssertionProvider {
    fn execute(
        &self,
        name: &str,
        params: &serde_json::Map<String, serde_json::Value>,
        context: &WindowContext,
    ) -> AssertionRecord {
        let body = json!({
            "name": name,
            "params": params,
            "window_context": context,
        })
        .to_string();
        let response = match self.transport.post(&self.endpoint, &body) {
            Ok(response) => response,
            Err(error) => {
                return self.no_show(name, &failure_reason(&error));
            }
        };
        if response.status != 200 {
            return self.no_show(
                name,
                &format!(
                    "{}: {}",
                    failure_reason(&ProviderError::BadStatus(response.status)),
                    body_excerpt(&response.body),
                ),
            );
        }
        let record: AssertionRecord = match serde_json::from_slice(&response.body) {
            Ok(record) => record,
            Err(error) => {
                return self.no_show(
                    name,
                    &format!(
                        "provider response is not a valid assertion result ({error}): {}",
                        body_excerpt(&response.body),
                    ),
                );
            }
        };
        // Integrity checks on what came back: a record is only accepted
        // if it is the answer to THIS execution and it satisfies the
        // schema's own constraints.
        if record.name != name {
            return self.no_show(
                name,
                &format!(
                    "provider responded about {:?} instead of {:?}",
                    record.name, name,
                ),
            );
        }
        if record.validate().is_err() {
            return self.no_show(
                name,
                &format!(
                    "provider response fails its own schema checks: {:?}",
                    record.validate().err(),
                ),
            );
        }
        record
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assertions::{AssertionStatus, EvidenceKind, EvidenceRef, RecordWindow, Trigger};
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    fn context() -> WindowContext {
        WindowContext {
            swell_id: "sw-1".into(),
            backend_id: "kb-conf".into(),
            window_start_unix_ms: 1_000,
            window_end_unix_ms: 9_000,
            trigger: Trigger::AfterScenario,
            evidence_dir: None,
        }
    }

    fn params() -> serde_json::Map<String, serde_json::Value> {
        let mut map = serde_json::Map::new();
        map.insert(
            "endpoint".into(),
            serde_json::Value::String("http://gm.internal/audit".into()),
        );
        map
    }

    fn pass_json(name: &str) -> String {
        let record = AssertionRecord {
            provider: "game-admin-api".into(),
            name: name.into(),
            status: AssertionStatus::Pass,
            checked_at: "2026-11-10T00:00:00Z".into(),
            window: Some(RecordWindow {
                start: "2026-11-10T00:00:00Z".into(),
                end: "2026-11-10T00:05:00Z".into(),
            }),
            trigger: Some(Trigger::AfterScenario),
            message: None,
            evidence: vec![EvidenceRef {
                kind: EvidenceKind::Query,
                reference: "SELECT 1".into(),
            }],
        };
        serde_json::to_string(&record).expect("json")
    }

    fn static_transport(status: u16, body: &str) -> Box<dyn AssertionHttpTransport> {
        struct Static {
            status: u16,
            body: String,
        }
        impl AssertionHttpTransport for Static {
            fn post(&self, _url: &str, _body: &str) -> Result<HttpResponse, ProviderError> {
                Ok(HttpResponse {
                    status: self.status,
                    body: self.body.clone().into_bytes(),
                })
            }
        }
        Box::new(Static {
            status,
            body: body.to_string(),
        })
    }

    fn unreachable_transport() -> Box<dyn AssertionHttpTransport> {
        struct Down;
        impl AssertionHttpTransport for Down {
            fn post(&self, _url: &str, _body: &str) -> Result<HttpResponse, ProviderError> {
                Err(ProviderError::Unreachable(
                    "127.0.0.1:1: connection refused".into(),
                ))
            }
        }
        Box::new(Down)
    }

    fn provider_with(
        endpoint: &str,
        transport: Box<dyn AssertionHttpTransport>,
    ) -> HttpAssertionProvider {
        HttpAssertionProvider::with_transport(endpoint, transport)
            .with_clock(Box::new(|| "2026-11-10T00:09:59Z".into()))
    }

    fn serve_one(response_status: String, response_body: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(stream);
            let mut request_line = String::new();
            reader.read_line(&mut request_line).expect("request line");
            let mut content_length = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).expect("header");
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some(value) = line.strip_prefix("Content-Length:") {
                    content_length = value.trim().parse().expect("length");
                }
            }
            let mut body = vec![0u8; content_length];
            reader.read_exact(&mut body).expect("body");
            let http = format!(
                "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_status,
                response_body.len(),
                response_body,
            );
            let mut stream = reader.into_inner();
            stream.write_all(http.as_bytes()).expect("respond");
        });
        format!("http://{}", addr)
    }

    /// Captures the request body so the wire shape is asserted
    /// single-threaded.
    struct RecordingTransport {
        seen: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
        status: u16,
        body: String,
    }

    impl AssertionHttpTransport for RecordingTransport {
        fn post(&self, _url: &str, body: &str) -> Result<HttpResponse, ProviderError> {
            self.seen.borrow_mut().push(body.to_string());
            Ok(HttpResponse {
                status: self.status,
                body: self.body.clone().into_bytes(),
            })
        }
    }

    #[test]
    fn completed_results_are_taken_verbatim() {
        let url = serve_one("200 OK".to_string(), pass_json("dupes"));
        let provider = provider_with(&url, Box::new(TcpPostTransport { timeout: None }));
        let record = provider.execute("dupes", &params(), &context());
        assert_eq!(record.status, AssertionStatus::Pass, "record: {record:?}");
        assert_eq!(record.provider, "game-admin-api");
        assert_eq!(record.checked_at, "2026-11-10T00:00:00Z");
        assert_eq!(provider.endpoint(), url);
    }

    #[test]
    fn request_wire_shape_carries_name_params_and_context() {
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let transport = RecordingTransport {
            seen: std::rc::Rc::clone(&seen),
            status: 200,
            body: pass_json("dupes"),
        };
        let provider = HttpAssertionProvider::with_transport(
            "http://gm.internal/audit/duplicate-roles",
            Box::new(transport),
        );
        let _ = provider.execute("dupes", &params(), &context());
        drop(provider);
        let seen = seen.borrow();
        assert_eq!(seen.len(), 1);
        let body: serde_json::Value = serde_json::from_str(&seen[0]).expect("body is json");
        assert_eq!(body["name"], "dupes");
        assert_eq!(body["params"]["endpoint"], "http://gm.internal/audit");
        assert_eq!(body["window_context"]["swell_id"], "sw-1");
        assert_eq!(body["window_context"]["trigger"], "after_scenario");
        assert_eq!(body["window_context"]["window_start_unix_ms"], 1_000);
    }

    #[test]
    fn completed_fail_results_pass_through_untouched() {
        let mut record: AssertionRecord = serde_json::from_str(&pass_json("dupes")).expect("json");
        record.status = AssertionStatus::Fail;
        record.message = Some("3 duplicate role pairs found for accounts 1042,1087,2210".into());
        let url = serve_one(
            "200 OK".to_string(),
            serde_json::to_string(&record).expect("json"),
        );
        let provider = provider_with(&url, Box::new(TcpPostTransport { timeout: None }));
        let returned = provider.execute("dupes", &params(), &context());
        assert_eq!(
            returned.status,
            AssertionStatus::Fail,
            "record: {returned:?}"
        );
        assert_eq!(
            returned.message.as_deref(),
            Some("3 duplicate role pairs found for accounts 1042,1087,2210")
        );
        assert!(url.starts_with("http://127.0.0.1"));
    }

    #[test]
    fn non_200_is_inconclusive_with_verbatim_status() {
        let provider = provider_with(
            "http://gm.internal/audit/duplicate-roles",
            static_transport(503, "upstream down"),
        );
        let record = provider.execute("dupes", &params(), &context());
        assert_eq!(record.status, AssertionStatus::Inconclusive);
        assert!(record
            .message
            .as_ref()
            .expect("message")
            .contains("http status 503"));
        assert!(record
            .message
            .as_ref()
            .expect("message")
            .contains("upstream down"));
        assert_eq!(record.checked_at, "2026-11-10T00:09:59Z");
    }

    #[test]
    fn malformed_body_is_inconclusive_never_pass() {
        let provider = provider_with(
            "http://gm.internal/audit/duplicate-roles",
            static_transport(200, "<html>oops</html>"),
        );
        let record = provider.execute("dupes", &params(), &context());
        assert_eq!(record.status, AssertionStatus::Inconclusive);
        assert!(record
            .message
            .as_ref()
            .expect("message")
            .contains("not a valid assertion result"));

        // A fourth status ("skipped") is rejected by the closed enum and
        // lands here too.
        let fourth = pass_json("dupes").replace("\"pass\"", "\"skipped\"");
        let provider = provider_with(
            "http://gm.internal/audit/duplicate-roles",
            static_transport(200, &fourth),
        );
        assert_eq!(
            provider.execute("dupes", &params(), &context()).status,
            AssertionStatus::Inconclusive
        );
    }

    #[test]
    fn records_failing_their_own_schema_are_refused() {
        // No evidence (schema minItems 1).
        let mut no_evidence: AssertionRecord =
            serde_json::from_str(&pass_json("dupes")).expect("json");
        no_evidence.evidence = vec![];
        let provider = provider_with(
            "http://gm.internal/audit/duplicate-roles",
            static_transport(200, &serde_json::to_string(&no_evidence).expect("json")),
        );
        assert_eq!(
            provider.execute("dupes", &params(), &context()).status,
            AssertionStatus::Inconclusive
        );

        // Empty checked_at.
        let mut no_stamp: AssertionRecord =
            serde_json::from_str(&pass_json("dupes")).expect("json");
        no_stamp.checked_at = String::new();
        let provider = provider_with(
            "http://gm.internal/audit/duplicate-roles",
            static_transport(200, &serde_json::to_string(&no_stamp).expect("json")),
        );
        assert_eq!(
            provider.execute("dupes", &params(), &context()).status,
            AssertionStatus::Inconclusive
        );

        // A record about a different assertion.
        let provider = provider_with(
            "http://gm.internal/audit/duplicate-roles",
            static_transport(200, &pass_json("someone-else")),
        );
        let record = provider.execute("dupes", &params(), &context());
        assert_eq!(record.status, AssertionStatus::Inconclusive);
        assert!(record
            .message
            .as_ref()
            .expect("message")
            .contains("someone-else"));
    }

    #[test]
    fn unreachable_provider_is_inconclusive() {
        let provider = provider_with(
            "http://gm.internal/audit/duplicate-roles",
            unreachable_transport(),
        );
        let record = provider.execute("dupes", &params(), &context());
        assert_eq!(record.status, AssertionStatus::Inconclusive);
        assert!(record
            .message
            .as_ref()
            .expect("message")
            .contains("connection refused"));
    }

    #[test]
    fn silent_server_hits_the_read_timeout_as_inconclusive() {
        // Accept, never respond: the read timeout is the enforcement
        // path a hung provider hits.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        std::thread::spawn(move || {
            let _held = listener.accept().expect("accept");
            std::thread::sleep(Duration::from_millis(1_000));
        });
        let provider = HttpAssertionProvider::with_transport(
            format!("http://{addr}"),
            Box::new(TcpPostTransport {
                timeout: Some(Duration::from_millis(200)),
            }),
        )
        .with_clock(Box::new(|| "2026-11-10T00:09:59Z".into()));
        let record = provider.execute("dupes", &params(), &context());
        assert_eq!(record.status, AssertionStatus::Inconclusive);
        assert!(record.message.as_ref().expect("message").contains("read:"));
    }

    #[test]
    fn long_error_bodies_are_truncated_not_flooded() {
        let flood = "x".repeat(10_000);
        let provider = provider_with(
            "http://gm.internal/audit/duplicate-roles",
            static_transport(500, &flood),
        );
        let record = provider.execute("dupes", &params(), &context());
        let message = record.message.as_ref().expect("message");
        assert!(message.contains("truncated"));
        assert!(message.len() < 1_000);
    }
}
