//! Sign-in: OAuth 2.0 authorization code with PKCE, as the official Codex CLI does it.
//!
//! Rust port of Connected-Mate/gptimage `src/login.js` and the token parts of `src/auth.js` (MIT, commit
//! dfb4948). Request building and response parsing are pure functions; only [`wait_for_callback`] touches a
//! socket, and only on 127.0.0.1. Nothing here sends a request by itself.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use imagegen_providers::transport::{HttpRequest, Method};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{AUTHORIZE_URL, CALLBACK_PATH, CALLBACK_PORT, CLIENT_ID, ORIGINATOR, SCOPE, TOKEN_URL};

/// PKCE pair.
#[derive(Debug, Clone)]
pub struct Pkce {
    /// Secret kept locally until the code exchange.
    pub verifier: String,
    /// S256 challenge sent in the authorize URL.
    pub challenge: String,
}

/// Challenge for a verifier (S256, base64url without padding).
pub fn challenge_for(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// A fresh PKCE pair from the operating system's random source.
pub fn make_pkce() -> Result<Pkce, String> {
    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw).map_err(|e| format!("no secure random source: {e}"))?;
    let verifier = URL_SAFE_NO_PAD.encode(raw);
    Ok(Pkce { challenge: challenge_for(&verifier), verifier })
}

/// A random `state` value.
pub fn make_state() -> Result<String, String> {
    let mut raw = [0u8; 16];
    getrandom::fill(&mut raw).map_err(|e| format!("no secure random source: {e}"))?;
    Ok(raw.iter().map(|b| format!("{b:02x}")).collect())
}

/// The redirect URI for a loopback port.
pub fn redirect_uri(port: u16) -> String {
    format!("http://localhost:{port}{CALLBACK_PATH}")
}

fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn form(pairs: &[(&str, &str)]) -> Vec<u8> {
    pairs.iter().map(|(k, v)| format!("{}={}", enc(k), enc(v))).collect::<Vec<_>>().join("&").into_bytes()
}

/// The URL the user opens in a browser.
pub fn authorize_url(challenge: &str, state: &str, port: u16) -> String {
    let params = [
        ("response_type", "code"),
        ("client_id", CLIENT_ID),
        ("redirect_uri", &redirect_uri(port)),
        ("scope", SCOPE),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
        ("originator", ORIGINATOR),
    ];
    let query = params.iter().map(|(k, v)| format!("{}={}", enc(k), enc(v))).collect::<Vec<_>>().join("&");
    format!("{AUTHORIZE_URL}?{query}")
}

/// Request exchanging an authorization code for tokens.
pub fn exchange_request(code: &str, verifier: &str, port: u16) -> HttpRequest {
    HttpRequest {
        method: Method::Post,
        url: TOKEN_URL.to_string(),
        headers: vec![("Content-Type".into(), "application/x-www-form-urlencoded".into())],
        body: form(&[
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", &redirect_uri(port)),
        ]),
    }
}

/// Request refreshing an access token.
pub fn refresh_request(refresh_token: &str) -> HttpRequest {
    HttpRequest {
        method: Method::Post,
        url: TOKEN_URL.to_string(),
        headers: vec![("Content-Type".into(), "application/x-www-form-urlencoded".into())],
        body: form(&[("grant_type", "refresh_token"), ("refresh_token", refresh_token), ("client_id", CLIENT_ID)]),
    }
}

/// Tokens as returned by the token endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tokens {
    /// Access token.
    pub access: String,
    /// Refresh token, when issued.
    pub refresh: Option<String>,
    /// Lifetime in seconds, when reported.
    pub expires_in: Option<u64>,
}

/// Parse a token response body.
pub fn parse_tokens(body: &[u8]) -> Result<Tokens, String> {
    let v: Value = serde_json::from_slice(body).map_err(|e| format!("the token response is not JSON: {e}"))?;
    let access = v.get("access_token").and_then(Value::as_str).ok_or("the token response has no access_token")?.to_string();
    Ok(Tokens {
        access,
        refresh: v.get("refresh_token").and_then(Value::as_str).map(str::to_string),
        expires_in: v.get("expires_in").and_then(Value::as_u64),
    })
}

/// Claims of a JWT payload. The signature is not verified: the server does that.
pub fn jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

const CLAIM_PATH: &str = "https://api.openai.com/auth";

/// ChatGPT account id carried in the access token.
pub fn account_id(access: &str) -> Option<String> {
    jwt_claims(access)?.get(CLAIM_PATH)?.get("chatgpt_account_id")?.as_str().map(str::to_string)
}

/// Plan type carried in the access token.
pub fn plan_type(access: &str) -> Option<String> {
    jwt_claims(access)?.get(CLAIM_PATH)?.get("chatgpt_plan_type")?.as_str().map(str::to_string)
}

/// Expiry in epoch milliseconds from the `exp` claim.
pub fn expiry_ms(access: &str) -> Option<u64> {
    jwt_claims(access)?.get("exp")?.as_u64().map(|s| s * 1000)
}

/// Pull `code` out of a callback request line, checking `state`. Returns the code.
pub fn parse_callback(request_line: &str, expected_state: &str) -> Result<String, String> {
    let target = request_line.split_whitespace().nth(1).ok_or("empty request")?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != CALLBACK_PATH {
        return Err("not the callback path".into());
    }
    let get = |name: &str| -> Option<String> {
        query.split('&').find_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            (k == name).then(|| decode_component(v))
        })
    };
    if let Some(err) = get("error") {
        return Err(format!("sign-in was refused: {err}"));
    }
    if get("state").as_deref() != Some(expected_state) {
        return Err("the state value does not match: ignoring a callback this sign-in did not start".into());
    }
    get("code").filter(|c| !c.is_empty()).ok_or_else(|| "the callback has no code".to_string())
}

fn decode_component(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    out.push(v);
                    i += 3;
                    continue;
                }
                out.push(b'%');
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Shown in the browser after the sign-in. Signing in is the acceptance of the risk note (maintainer, 2026-11-05),
/// so the page repeats its main points as information.
const SUCCESS_PAGE: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>Signed in</title></head><body style=\"font-family:system-ui,sans-serif;max-width:40em;margin:15vh auto;padding:0 1em;line-height:1.5\"><h1>Signed in</h1><p>hoi4-imagegen can now make images on your ChatGPT plan. You can close this tab.</p><p style=\"color:#555\">Good to know: this route presents itself as the Codex CLI to a backend OpenAI does not document for other tools, so it can stop working without notice. Images count against your plan's limits, and heavy or parallel use can make OpenAI sign you out or restrict the account, so hoi4-imagegen sends one request at a time. Keep it to your own account on your own computer. <code>hoi4-imagegen login-risk-note</code> prints the full note.</p></body></html>";

/// Listen on 127.0.0.1 for the redirect and return the authorization code. Stops after `timeout`.
/// Callbacks with a wrong `state` are answered with 400 and ignored, so a stray request cannot end the sign-in.
pub fn wait_for_callback(listener: &TcpListener, expected_state: &str, timeout: Duration) -> Result<String, String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let started = Instant::now();
    loop {
        if started.elapsed() > timeout {
            return Err("timed out waiting for the browser sign-in".into());
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_nonblocking(false).ok();
                stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
                let line = read_request_line(&mut stream);
                match parse_callback(&line, expected_state) {
                    Ok(code) => {
                        respond(&mut stream, "200 OK", SUCCESS_PAGE);
                        return Ok(code);
                    }
                    Err(why) if why.starts_with("sign-in was refused") => {
                        respond(&mut stream, "400 Bad Request", "Sign-in was refused. You can close this tab.");
                        return Err(why);
                    }
                    Err(_) => respond(&mut stream, "400 Bad Request", "Not the sign-in callback."),
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn read_request_line(stream: &mut TcpStream) -> String {
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf).unwrap_or(0);
    String::from_utf8_lossy(&buf[..n]).lines().next().unwrap_or("").to_string()
}

fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let _ = stream.flush();
}

/// Bind the sign-in listener on the loopback interface.
pub fn bind_listener(port: u16) -> Result<TcpListener, String> {
    TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("cannot listen on 127.0.0.1:{port}: {e}"))
}

/// The default sign-in port.
pub const fn default_port() -> u16 {
    CALLBACK_PORT
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test vector from RFC 7636 appendix B.
    #[test]
    fn pkce_challenge_matches_the_rfc_vector() {
        // The verifier is the RFC's octets [116, 24, 223, ...] base64url-encoded; checked against Python's hashlib too.
        assert_eq!(challenge_for("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let p = make_pkce().unwrap();
        assert_eq!(p.verifier.len(), 43);
        assert_eq!(challenge_for(&p.verifier), p.challenge);
        assert_ne!(make_pkce().unwrap().verifier, p.verifier);
        assert_eq!(make_state().unwrap().len(), 32);
    }

    #[test]
    fn authorize_url_carries_every_upstream_parameter() {
        let url = authorize_url("CHAL", "STATE", 1455);
        assert!(url.starts_with("https://auth.openai.com/oauth/authorize?"));
        for part in [
            "response_type=code",
            "client_id=app_EMoamEEZ73f0CkXaXp7hrann",
            "redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback",
            "scope=openid%20profile%20email%20offline_access",
            "code_challenge=CHAL",
            "code_challenge_method=S256",
            "state=STATE",
            "id_token_add_organizations=true",
            "codex_cli_simplified_flow=true",
            "originator=codex_cli_rs",
        ] {
            assert!(url.contains(part), "missing {part} in {url}");
        }
    }

    #[test]
    fn token_requests_are_form_encoded() {
        let r = exchange_request("the code/1", "ver", 1455);
        assert_eq!(r.url, TOKEN_URL);
        let body = String::from_utf8(r.body).unwrap();
        assert!(body.contains("grant_type=authorization_code") && body.contains("code=the%20code%2F1") && body.contains("code_verifier=ver"));
        let r = refresh_request("rt-1");
        let body = String::from_utf8(r.body).unwrap();
        assert!(body.contains("grant_type=refresh_token") && body.contains("refresh_token=rt-1") && body.contains("client_id=app_"));
    }

    fn fake_jwt(claims: &Value) -> String {
        format!("{}.{}.sig", URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#), URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).unwrap()))
    }

    #[test]
    fn jwt_claims_are_decoded_without_verification() {
        // SYNTHETIC token: made up for the tests
        let t = fake_jwt(
            &serde_json::json!({"exp": 2_000_000_000u64, "https://api.openai.com/auth": {"chatgpt_account_id": "acct-synthetic", "chatgpt_plan_type": "plus"}}),
        );
        assert_eq!(account_id(&t).as_deref(), Some("acct-synthetic"));
        assert_eq!(plan_type(&t).as_deref(), Some("plus"));
        assert_eq!(expiry_ms(&t), Some(2_000_000_000_000));
        assert!(account_id("not-a-jwt").is_none() && expiry_ms("a.b.c").is_none());
    }

    #[test]
    fn token_responses_parse() {
        let t = parse_tokens(br#"{"access_token":"a","refresh_token":"r","expires_in":3600}"#).unwrap();
        assert_eq!((t.access.as_str(), t.refresh.as_deref(), t.expires_in), ("a", Some("r"), Some(3600)));
        assert!(parse_tokens(br#"{"refresh_token":"r"}"#).is_err());
        assert!(parse_tokens(b"nope").is_err());
    }

    #[test]
    fn callbacks_check_path_state_and_errors() {
        assert_eq!(parse_callback("GET /auth/callback?code=abc%2F1&state=S HTTP/1.1", "S").unwrap(), "abc/1");
        assert!(parse_callback("GET /auth/callback?code=abc&state=WRONG HTTP/1.1", "S").unwrap_err().contains("state"));
        assert!(parse_callback("GET /other?code=abc&state=S HTTP/1.1", "S").is_err());
        assert!(parse_callback("GET /auth/callback?error=access_denied&state=S HTTP/1.1", "S").unwrap_err().contains("refused"));
        assert!(parse_callback("GET /auth/callback?state=S HTTP/1.1", "S").is_err());
        assert!(parse_callback("", "S").is_err());
    }

    // The loopback listener is exercised with a local client only: no request leaves this machine.
    #[test]
    fn the_listener_ignores_strays_and_returns_the_code() {
        let listener = bind_listener(0).unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = std::thread::spawn(move || {
            let send = |path: &str| {
                let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
                write!(s, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
                let mut resp = String::new();
                s.read_to_string(&mut resp).ok();
                resp
            };
            let stray = send("/auth/callback?code=evil&state=nope");
            let good = send("/auth/callback?code=real-code&state=GOOD");
            (stray, good)
        });
        let code = wait_for_callback(&listener, "GOOD", Duration::from_secs(10)).unwrap();
        assert_eq!(code, "real-code");
        let (stray, good) = client.join().unwrap();
        assert!(stray.starts_with("HTTP/1.1 400") && good.starts_with("HTTP/1.1 200"));
    }

    #[test]
    fn the_listener_times_out_and_reports_a_refusal() {
        let listener = bind_listener(0).unwrap();
        assert!(wait_for_callback(&listener, "S", Duration::from_millis(100)).unwrap_err().contains("timed out"));
        let port = listener.local_addr().unwrap().port();
        let client = std::thread::spawn(move || {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(s, "GET /auth/callback?error=access_denied&state=S HTTP/1.1\r\n\r\n").unwrap();
            let mut r = String::new();
            s.read_to_string(&mut r).ok();
        });
        assert!(wait_for_callback(&listener, "S", Duration::from_secs(10)).unwrap_err().contains("refused"));
        client.join().unwrap();
    }
}
