//! HTTP transport with a destination policy.
//!
//! Every request passes [`HostPolicy::check`] before any bytes leave: https only, port 443, an allowlist of
//! API hosts, no credentials in the URL, no private, loopback or link-local addresses, and no redirects. The
//! one exception is an endpoint the maintainer configured themselves (a ComfyUI server on a local GPU or a
//! RunPod URL, decision D3), which may be any host and plain http.
//!
//! The real client is [`UreqTransport`]. Tests use [`FixtureTransport`], which never touches a network.

use std::collections::VecDeque;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::Duration;

use imagegen_core::error::RetryClass;

use crate::types::{ErrorKind, ProviderError};

/// HTTP method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// GET
    Get,
    /// POST
    Post,
    /// PUT
    Put,
}

/// A request.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    /// Method.
    pub method: Method,
    /// Absolute URL.
    pub url: String,
    /// Headers.
    pub headers: Vec<(String, String)>,
    /// Body.
    pub body: Vec<u8>,
}

impl HttpRequest {
    /// GET request.
    pub fn get(url: impl Into<String>) -> Self {
        HttpRequest { method: Method::Get, url: url.into(), headers: Vec::new(), body: Vec::new() }
    }

    /// POST with a JSON body.
    pub fn post_json(url: impl Into<String>, body: &serde_json::Value) -> Self {
        HttpRequest {
            method: Method::Post,
            url: url.into(),
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: serde_json::to_vec(body).unwrap_or_default(),
        }
    }

    /// Add a header.
    pub fn header(mut self, k: &str, v: &str) -> Self {
        self.headers.push((k.to_string(), v.to_string()));
        self
    }
}

/// A response.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// Status code.
    pub status: u16,
    /// Headers (names lowercase).
    pub headers: Vec<(String, String)>,
    /// Body.
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// Header value by case-insensitive name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// Transport-level failure.
#[derive(Debug, Clone)]
pub enum TransportError {
    /// Failed before the request was sent (DNS, connect).
    NotSent(String),
    /// Failed after the request may have been sent (timeout, reset).
    MaybeSent(String),
    /// The response exceeded the size cap.
    TooLarge(usize),
}

/// A client.
pub trait HttpTransport: Send + Sync {
    /// Perform one request. No redirects are followed.
    fn execute(&self, req: &HttpRequest, max_response_bytes: usize, timeout: Duration) -> Result<HttpResponse, TransportError>;
}

/// A URL split into the parts the policy needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedUrl {
    /// `http` or `https`.
    pub scheme: String,
    /// Lowercase host without brackets.
    pub host: String,
    /// Explicit port.
    pub port: Option<u16>,
    /// Path and query.
    pub path_and_query: String,
}

/// Split an absolute http(s) URL. Rejects credentials, whitespace and backslashes.
pub fn parse_url(url: &str) -> Result<ParsedUrl, String> {
    if url.chars().any(|c| c.is_whitespace() || c == '\\' || c.is_control()) {
        return Err("URL contains whitespace, backslashes or control characters".into());
    }
    let (scheme, rest) = url.split_once("://").ok_or("URL has no scheme")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(format!("scheme '{scheme}' is not allowed"));
    }
    let rest = rest.split('#').next().unwrap_or("");
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    if authority.contains('@') {
        return Err("credentials in the URL are not allowed".into());
    }
    let (host, port) = if let Some(stripped) = authority.strip_prefix('[') {
        let close = stripped.find(']').ok_or("unterminated IPv6 literal")?;
        let host = &stripped[..close];
        let after = &stripped[close + 1..];
        let port = match after.strip_prefix(':') {
            Some(p) => Some(p.parse::<u16>().map_err(|_| "bad port")?),
            None if after.is_empty() => None,
            None => return Err("junk after IPv6 literal".into()),
        };
        (host.to_string(), port)
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), Some(p.parse::<u16>().map_err(|_| "bad port")?)),
            None => (authority.to_string(), None),
        }
    };
    if host.is_empty() {
        return Err("URL has no host".into());
    }
    let path_and_query = if tail.is_empty() { "/".to_string() } else { tail.to_string() };
    Ok(ParsedUrl { scheme, host: host.to_ascii_lowercase(), port, path_and_query })
}

/// True for loopback, private, link-local, unique-local, CGNAT and unspecified addresses.
pub fn is_private_host(host: &str) -> bool {
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => {
            let o = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || (o[0] == 100 && (64..128).contains(&o[1]))
        }
        Ok(IpAddr::V6(v6)) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_private_host(&v4.to_string());
            }
            let seg = v6.segments();
            v6.is_loopback() || v6.is_unspecified() || (seg[0] & 0xfe00) == 0xfc00 || (seg[0] & 0xffc0) == 0xfe80
        }
        Err(_) => false,
    }
}

/// True for loopback only.
pub fn is_loopback_host(host: &str) -> bool {
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    host.parse::<IpAddr>().map(|ip| ip.is_loopback()).unwrap_or(false)
}

/// Where a provider may send requests.
#[derive(Debug, Clone)]
pub struct HostPolicy {
    /// Exact hosts, or suffixes written with a leading dot (`.fal.media`).
    pub allowed_hosts: Vec<String>,
    /// A maintainer-configured endpoint: any host, any port, http allowed (decision D3).
    pub user_endpoint: bool,
    /// Allow plain http to loopback hosts (local mock servers, local APIs).
    pub allow_loopback_http: bool,
    /// Largest response body.
    pub max_response_bytes: usize,
    /// Whole-request timeout.
    pub timeout: Duration,
}

impl HostPolicy {
    /// A policy for an API with fixed hosts.
    pub fn api(hosts: &[&str]) -> Self {
        HostPolicy {
            allowed_hosts: hosts.iter().map(|h| h.to_string()).collect(),
            user_endpoint: false,
            allow_loopback_http: false,
            max_response_bytes: 96 * 1024 * 1024,
            timeout: Duration::from_secs(300),
        }
    }

    /// A policy for an endpoint the maintainer configured.
    pub fn user_endpoint() -> Self {
        HostPolicy {
            allowed_hosts: Vec::new(),
            user_endpoint: true,
            allow_loopback_http: true,
            max_response_bytes: 96 * 1024 * 1024,
            timeout: Duration::from_secs(600),
        }
    }

    fn host_allowed(&self, host: &str) -> bool {
        self.allowed_hosts.iter().any(|a| {
            if let Some(suffix) = a.strip_prefix('.') {
                host.ends_with(&format!(".{suffix}")) || host == suffix
            } else {
                host == a
            }
        })
    }

    /// Check a URL before sending.
    pub fn check(&self, url: &str) -> Result<ParsedUrl, String> {
        let parsed = parse_url(url)?;
        if self.user_endpoint {
            return Ok(parsed);
        }
        let loopback = is_loopback_host(&parsed.host);
        if loopback {
            if !(self.allow_loopback_http && self.host_allowed(&parsed.host)) {
                return Err(format!("host '{}' is a loopback address and is not allowed for this provider", parsed.host));
            }
            return Ok(parsed);
        }
        if parsed.scheme != "https" {
            return Err("only https is allowed".into());
        }
        if parsed.port.map(|p| p != 443).unwrap_or(false) {
            return Err("only port 443 is allowed".into());
        }
        if is_private_host(&parsed.host) {
            return Err(format!("host '{}' is a private, link-local or reserved address", parsed.host));
        }
        if !self.host_allowed(&parsed.host) {
            return Err(format!(
                "host '{}' is not on this provider's allowlist. Hosts outside it are allowed only when named in HOI4_IMAGEGEN_ALLOWED_HOSTS",
                parsed.host
            ));
        }
        Ok(parsed)
    }
}

/// Check the policy, then send. Maps transport failures to provider errors with a retry class that is honest
/// about uncertain charges: a POST that may have reached a synchronous API must be reconciled before it is
/// repeated.
pub fn send(transport: &dyn HttpTransport, policy: &HostPolicy, req: &HttpRequest) -> Result<HttpResponse, ProviderError> {
    if let Err(why) = policy.check(&req.url) {
        return Err(ProviderError::new(ErrorKind::Blocked, RetryClass::Terminal, format!("destination blocked: {why}")));
    }
    transport.execute(req, policy.max_response_bytes, policy.timeout).map_err(|e| match e {
        TransportError::NotSent(m) => ProviderError::new(ErrorKind::Transport, RetryClass::SafeSameInputs, format!("could not reach the provider: {m}")),
        TransportError::MaybeSent(m) => {
            let retry = if req.method == Method::Get { RetryClass::SafeSameInputs } else { RetryClass::ReconcileFirst };
            ProviderError::new(ErrorKind::Transport, retry, format!("the connection failed after the request may have been sent: {m}"))
        }
        TransportError::TooLarge(n) => ProviderError::new(ErrorKind::BadImage, RetryClass::Terminal, format!("response exceeded the {n} byte limit")),
    })
}

/// The real client. No redirects, no credentials in URLs, timeouts, response size cap.
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    /// Build a client.
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .user_agent("hoi4-imagegen-mcp")
            .build();
        UreqTransport { agent: ureq::Agent::new_with_config(config) }
    }
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpTransport for UreqTransport {
    fn execute(&self, req: &HttpRequest, max_response_bytes: usize, timeout: Duration) -> Result<HttpResponse, TransportError> {
        let map_err = |e: ureq::Error| match e {
            ureq::Error::HostNotFound | ureq::Error::ConnectionFailed => TransportError::NotSent(e.to_string()),
            ureq::Error::Io(ref io) if io.kind() == std::io::ErrorKind::ConnectionRefused => TransportError::NotSent(e.to_string()),
            other => TransportError::MaybeSent(other.to_string()),
        };
        let result = match req.method {
            Method::Get => {
                let mut r = self.agent.get(&req.url).config().timeout_global(Some(timeout)).build();
                for (k, v) in &req.headers {
                    r = r.header(k, v);
                }
                r.call()
            }
            Method::Post | Method::Put => {
                let builder = if req.method == Method::Post { self.agent.post(&req.url) } else { self.agent.put(&req.url) };
                let mut r = builder.config().timeout_global(Some(timeout)).build();
                for (k, v) in &req.headers {
                    r = r.header(k, v);
                }
                r.send(&req.body[..])
            }
        };
        let mut resp = result.map_err(map_err)?;
        let status = resp.status().as_u16();
        let headers = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str().to_ascii_lowercase(), v.to_str().unwrap_or("").to_string()))
            .collect();
        let body = resp
            .body_mut()
            .with_config()
            .limit(max_response_bytes as u64)
            .read_to_vec()
            .map_err(|e| match e {
                ureq::Error::BodyExceedsLimit(_) => TransportError::TooLarge(max_response_bytes),
                other => TransportError::MaybeSent(other.to_string()),
            })?;
        Ok(HttpResponse { status, headers, body })
    }
}

/// Replays canned responses and records requests. **Synthetic**: nothing it returns came from a provider.
/// Contract tests use it to check wire format, parsing and error mapping; it is never provider evidence.
#[derive(Default)]
pub struct FixtureTransport {
    inner: Mutex<FixtureState>,
}

#[derive(Default)]
struct FixtureState {
    routes: Vec<Route>,
    requests: Vec<HttpRequest>,
}

struct Route {
    method: Method,
    contains: String,
    queue: VecDeque<Result<HttpResponse, TransportError>>,
}

impl FixtureTransport {
    /// An empty fixture.
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue a response for requests whose method matches and whose URL contains `contains` (a leading `$`
    /// means the URL must end with the rest instead). The last queued response for a route is repeated once the
    /// queue runs down to one.
    pub fn respond(&self, method: Method, contains: &str, status: u16, headers: &[(&str, &str)], body: impl Into<Vec<u8>>) -> &Self {
        let resp = HttpResponse {
            status,
            headers: headers.iter().map(|(k, v)| (k.to_ascii_lowercase(), v.to_string())).collect(),
            body: body.into(),
        };
        self.push(method, contains, Ok(resp));
        self
    }

    /// Forget everything queued for a route, so the next `respond` starts it afresh.
    pub fn clear_route(&self, method: Method, contains: &str) -> &Self {
        let mut st = self.inner.lock().expect("fixture lock");
        st.routes.retain(|r| !(r.method == method && r.contains == contains));
        self
    }

    /// Queue a transport failure.
    pub fn fail(&self, method: Method, contains: &str, err: TransportError) -> &Self {
        self.push(method, contains, Err(err));
        self
    }

    fn push(&self, method: Method, contains: &str, item: Result<HttpResponse, TransportError>) {
        let mut st = self.inner.lock().expect("fixture lock");
        if let Some(r) = st.routes.iter_mut().find(|r| r.method == method && r.contains == contains) {
            r.queue.push_back(item);
        } else {
            st.routes.push(Route { method, contains: contains.to_string(), queue: VecDeque::from([item]) });
        }
    }

    /// Requests seen so far.
    pub fn requests(&self) -> Vec<HttpRequest> {
        self.inner.lock().expect("fixture lock").requests.clone()
    }
}

impl HttpTransport for FixtureTransport {
    fn execute(&self, req: &HttpRequest, _max: usize, _timeout: Duration) -> Result<HttpResponse, TransportError> {
        let mut st = self.inner.lock().expect("fixture lock");
        st.requests.push(req.clone());
        // longest matching substring wins
        let idx = st
            .routes
            .iter()
            .enumerate()
            .filter(|(_, r)| r.method == req.method && route_matches(&r.contains, &req.url))
            .max_by_key(|(_, r)| r.contains.len())
            .map(|(i, _)| i);
        match idx {
            None => Err(TransportError::NotSent(format!("fixture has no route for {:?} {}", req.method, req.url))),
            Some(i) => {
                let route = &mut st.routes[i];
                if route.queue.len() > 1 {
                    route.queue.pop_front().expect("non-empty")
                } else {
                    route.queue.front().cloned().expect("non-empty")
                }
            }
        }
    }
}

fn route_matches(pattern: &str, url: &str) -> bool {
    match pattern.strip_prefix('$') {
        Some(suffix) => url.ends_with(suffix),
        None => url.contains(pattern),
    }
}

/// Remove secrets from text that may reach a log, a ledger or an error message.
pub fn redact(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_string();
    for s in secrets {
        if s.len() >= 6 {
            out = out.replace(s.as_str(), "[redacted]");
        }
    }
    out
}

/// Pull a human message out of a provider error body, truncated and redacted.
pub fn error_message(body: &[u8], secrets: &[String]) -> String {
    let text = String::from_utf8_lossy(body);
    let msg = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .or_else(|| v.pointer("/error"))
                .or_else(|| v.pointer("/detail"))
                .or_else(|| v.pointer("/message"))
                .and_then(|m| m.as_str().map(str::to_string).or_else(|| Some(m.to_string())))
        })
        .unwrap_or_else(|| text.chars().take(300).collect());
    let msg: String = msg.chars().take(500).collect();
    redact(&msg, secrets)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_parsing() {
        let u = parse_url("https://API.OpenAI.com/v1/images?x=1#frag").unwrap();
        assert_eq!((u.scheme.as_str(), u.host.as_str(), u.port), ("https", "api.openai.com", None));
        assert_eq!(u.path_and_query, "/v1/images?x=1");
        assert_eq!(parse_url("http://127.0.0.1:8188").unwrap().port, Some(8188));
        assert_eq!(parse_url("http://[::1]:8188/x").unwrap().host, "::1");
        for bad in ["ftp://x/y", "https://user:pw@host/", "https:///x", "https://a b/", "no-scheme", "https://host:99999/", "https://h\\x/"] {
            assert!(parse_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn private_address_detection() {
        for h in ["127.0.0.1", "10.1.2.3", "172.16.0.1", "192.168.1.1", "169.254.169.254", "100.64.0.1", "::1", "fd00::1", "fe80::1", "::ffff:10.0.0.1", "localhost", "a.localhost", "0.0.0.0"] {
            assert!(is_private_host(h), "{h}");
        }
        for h in ["8.8.8.8", "api.openai.com", "2606:4700::1111", "100.63.0.1"] {
            assert!(!is_private_host(h), "{h}");
        }
    }

    #[test]
    fn api_policy_enforces_https_port_and_allowlist() {
        let p = HostPolicy::api(&["api.openai.com", ".fal.media"]);
        assert!(p.check("https://api.openai.com/v1/images/generations").is_ok());
        assert!(p.check("https://v3.fal.media/files/x.png").is_ok());
        assert!(p.check("https://fal.media/x").is_ok());
        for bad in [
            "http://api.openai.com/v1",
            "https://api.openai.com:8443/v1",
            "https://evil.example/v1",
            "https://notfal.media/x",
            "https://127.0.0.1/x",
            "https://169.254.169.254/latest/meta-data",
            "https://api.openai.com@evil.example/",
        ] {
            assert!(p.check(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn loopback_needs_both_the_flag_and_an_explicit_host() {
        let mut p = HostPolicy::api(&["127.0.0.1"]);
        assert!(p.check("http://127.0.0.1:9999/x").is_err());
        p.allow_loopback_http = true;
        assert!(p.check("http://127.0.0.1:9999/x").is_ok());
        assert!(p.check("http://localhost:9999/x").is_err(), "localhost is not on the allowlist");
        let q = HostPolicy::api(&["api.openai.com"]);
        assert!(q.check("http://127.0.0.1:9999/x").is_err());
    }

    #[test]
    fn user_endpoints_may_be_anywhere_but_not_carry_credentials() {
        let p = HostPolicy::user_endpoint();
        for ok in ["http://127.0.0.1:8188/prompt", "http://192.168.1.20:8188/prompt", "https://abc-8188.proxy.runpod.net/prompt"] {
            assert!(p.check(ok).is_ok(), "{ok}");
        }
        assert!(p.check("http://user:pw@127.0.0.1:8188/").is_err());
        assert!(p.check("file:///etc/passwd").is_err());
    }

    #[test]
    fn send_blocks_before_the_transport_is_touched() {
        let fixture = FixtureTransport::new();
        let policy = HostPolicy::api(&["api.openai.com"]);
        let err = send(&fixture, &policy, &HttpRequest::get("https://evil.example/")).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Blocked);
        assert!(fixture.requests().is_empty(), "a blocked request must never reach the transport");
    }

    #[test]
    fn transport_failures_map_to_honest_retry_classes() {
        let fixture = FixtureTransport::new();
        fixture.fail(Method::Post, "api.openai.com", TransportError::MaybeSent("timed out".into()));
        fixture.fail(Method::Get, "api.openai.com", TransportError::MaybeSent("reset".into()));
        fixture.fail(Method::Put, "api.openai.com", TransportError::NotSent("dns".into()));
        let policy = HostPolicy::api(&["api.openai.com"]);
        let post = send(&fixture, &policy, &HttpRequest::post_json("https://api.openai.com/x", &serde_json::json!({}))).unwrap_err();
        assert_eq!(post.retry, RetryClass::ReconcileFirst, "an uncertain POST must not be blindly repeated");
        let get = send(&fixture, &policy, &HttpRequest::get("https://api.openai.com/x")).unwrap_err();
        assert_eq!(get.retry, RetryClass::SafeSameInputs);
        let mut put = HttpRequest::get("https://api.openai.com/x");
        put.method = Method::Put;
        let put = send(&fixture, &policy, &put).unwrap_err();
        assert_eq!(put.retry, RetryClass::SafeSameInputs);
    }

    #[test]
    fn fixture_routes_queue_and_stick_on_the_last_response() {
        let f = FixtureTransport::new();
        f.respond(Method::Get, "/status", 200, &[], "queued").respond(Method::Get, "/status", 200, &[], "done");
        f.respond(Method::Get, "/status/special", 200, &[], "special");
        let get = |u: &str| String::from_utf8(f.execute(&HttpRequest::get(u), 1000, Duration::from_secs(1)).unwrap().body).unwrap();
        assert_eq!(get("https://x/status"), "queued");
        assert_eq!(get("https://x/status"), "done");
        assert_eq!(get("https://x/status"), "done");
        assert_eq!(get("https://x/status/special"), "special");
        assert!(f.execute(&HttpRequest::get("https://x/other"), 1000, Duration::from_secs(1)).is_err());
        assert_eq!(f.requests().len(), 5);
    }

    #[test]
    fn redaction_and_error_messages() {
        let secrets = vec!["sk-live-123456".to_string()];
        assert_eq!(redact("bad key sk-live-123456 here", &secrets), "bad key [redacted] here");
        assert_eq!(redact("short ab", &["ab".to_string()]), "short ab", "very short secrets are ignored to avoid mangling text");
        let body = br#"{"error":{"message":"Invalid key sk-live-123456","type":"auth"}}"#;
        assert_eq!(error_message(body, &secrets), "Invalid key [redacted]");
        assert_eq!(error_message(b"plain text failure", &secrets), "plain text failure");
        let long = vec![b'x'; 2000];
        assert!(error_message(&long, &secrets).len() <= 300);
    }
}
