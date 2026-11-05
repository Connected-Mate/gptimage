//! `hoi4-imagegen login|status|logout` (also `hoi4-imagegen chatgpt-login <action>`) and the MCP tool
//! `hoi4.image_login`: the only code that starts a sign-in.
//!
//! Signing in is a deliberate act by the account holder in their own browser. The risk note is shown as information
//! (in the terminal, and its main points on the page the browser lands on), and signing in is the acceptance: there
//! is no question to answer (maintainer request of 2026-11-05, replacing the typed `yes` of D-12a). The code arrives
//! on 127.0.0.1 and the tokens go to the operating system's credential store.

use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

use imagegen_core::config::{EnvSource, ProcessEnv};
use imagegen_core::hashing::{Clock, SystemClock};
use imagegen_providers::transport::{send, HostPolicy, HttpTransport, UreqTransport};
use imagegen_providers::types::{CallContext, CancelToken, RealSleeper};

use crate::consent::{self, RISK_NOTE};
use crate::oauth::{
    account_id, authorize_url, bind_listener, exchange_request, expiry_ms, make_pkce, make_state, parse_tokens, wait_for_callback, Pkce,
};
use crate::provider::{describe, ChatGptLoginProvider, CheckOutcome};
use crate::route::{self, Route};
use crate::store::{CredentialStore, Credentials};
use crate::{evaluate_gates, PluginMode, CALLBACK_PORT, HOSTS};

/// How long a sign-in waits for the browser.
pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);

/// Run a sign-in action and return the text to show the user. `_accept_risk` is the old `--accept-risk` flag, kept
/// so scripts that pass it still work: there is no longer a question for it to skip.
pub fn run(action: &str, env: &dyn EnvSource, clock: &dyn Clock, _accept_risk: bool) -> Result<String, String> {
    run_with(action, env, clock)
}

/// [`run`] without the old flag.
pub fn run_with(action: &str, env: &dyn EnvSource, clock: &dyn Clock) -> Result<String, String> {
    evaluate_gates(PluginMode::Cli)?;
    let store = CredentialStore::for_environment(env)?;
    match action {
        "risk-note" => Ok(RISK_NOTE.to_string()),
        "status" => status_text(&store),
        "status-check" => {
            let provider = ChatGptLoginProvider::from_environment(env, Arc::new(UreqTransport::new()))?;
            status_check(&store, &provider, &CallContext::new())
        }
        "logout" => {
            let existed = store.delete()?;
            // signing out also forgets the server-revoked record: there is nothing left for it to describe
            Route::new(store.state_dir()).clear()?;
            Ok(if existed { format!("signed out; removed the tokens from the {}", store.location()) } else { "nothing to remove".to_string() })
        }
        "login" => {
            if !consent::accepted(env) {
                eprintln!("{RISK_NOTE}\nSigning in accepts this note. There is nothing to type: the browser opens now.\n");
            }
            let text = login(&store, &UreqTransport::new(), clock, CALLBACK_PORT, SIGN_IN_TIMEOUT, &mut |url| {
                eprintln!("Sign in with your own ChatGPT account in the browser. If it does not open, visit:\n\n{url}\n");
                open_browser(url);
            })?;
            consent::record(env, &clock.now_rfc3339())?;
            Ok(text)
        }
        other => Err(format!("unknown action '{other}': use login, status, status-check or logout")),
    }
}

/// Start a sign-in from a long-running local host (the MCP server, which HOI4 Mod Setup or an agent can ask): open
/// the browser and return at once with the address, while a background thread waits for the callback, stores the
/// tokens and records the note as accepted. A second start while one is waiting fails, because the callback port is
/// taken. `open` opens the address (the default browser in production).
pub fn start_login(env: &dyn EnvSource, mode: PluginMode, open: &mut dyn FnMut(&str)) -> Result<StartedLogin, String> {
    evaluate_gates(mode)?;
    let store = CredentialStore::for_environment(env)?;
    let pending = begin(CALLBACK_PORT)?;
    let url = pending.url.clone();
    let consent_file = consent::record_file(env);
    open(&url);
    std::thread::spawn(move || {
        let clock = SystemClock;
        match finish(pending, &store, &UreqTransport::new(), &clock, SIGN_IN_TIMEOUT) {
            Ok(_) => {
                if let Some(file) = consent_file {
                    if let Err(e) = consent::record_to(&file, &clock.now_rfc3339()) {
                        eprintln!("hoi4-imagegen: signed in, but the risk-note record could not be written: {e}");
                    }
                }
                eprintln!("hoi4-imagegen: signed in to ChatGPT");
            }
            Err(e) => eprintln!("hoi4-imagegen: the browser sign-in did not finish: {e}"),
        }
    });
    Ok(StartedLogin { url, timeout_secs: SIGN_IN_TIMEOUT.as_secs() })
}

/// What [`start_login`] returns.
#[derive(Debug, Clone)]
pub struct StartedLogin {
    /// The sign-in address the browser was sent to.
    pub url: String,
    /// How long the background sign-in waits for the browser.
    pub timeout_secs: u64,
}

/// A sign-in waiting for the browser: the listener is bound and the address is ready.
pub struct PendingLogin {
    listener: TcpListener,
    pkce: Pkce,
    state: String,
    port: u16,
    /// The address to open.
    pub url: String,
}

/// Bind the callback listener and build the sign-in address.
pub fn begin(port: u16) -> Result<PendingLogin, String> {
    let pkce = make_pkce()?;
    let state = make_state()?;
    let listener = bind_listener(port).map_err(|e| {
        if port == CALLBACK_PORT {
            format!("{e} (another sign-in may already be waiting for the browser: finish or close it first)")
        } else {
            e
        }
    })?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let url = authorize_url(&pkce.challenge, &state, port);
    Ok(PendingLogin { listener, pkce, state, port, url })
}

/// Wait for the browser, exchange the code and store the tokens.
pub fn finish(
    p: PendingLogin,
    store: &CredentialStore,
    transport: &dyn HttpTransport,
    clock: &dyn Clock,
    timeout: Duration,
) -> Result<String, String> {
    let code = wait_for_callback(&p.listener, &p.state, timeout)?;
    // The code exchange is a request to OpenAI like any other: it takes one of the route's slots. A successful
    // sign-in also clears a recorded server revocation, so the route works again.
    let route = Route::new(store.state_dir()).with_concurrency(route::configured_concurrency(&ProcessEnv, None));
    let _guard = route.acquire(&CancelToken::new(), &RealSleeper).map_err(|e| e.message)?;
    let resp = send(transport, &HostPolicy::api(&HOSTS), &exchange_request(&code, &p.pkce.verifier, p.port)).map_err(|e| e.message)?;
    if resp.status != 200 {
        return Err(format!("the code exchange failed with HTTP {}", resp.status));
    }
    let tokens = parse_tokens(&resp.body)?;
    let now_ms = (clock.now_secs().max(0) as u64) * 1000;
    let creds = Credentials {
        account_id: account_id(&tokens.access),
        expires_ms: tokens.expires_in.map(|s| now_ms + s * 1000).or_else(|| expiry_ms(&tokens.access)),
        access: tokens.access,
        refresh: tokens.refresh,
        last_refresh: clock.now_rfc3339(),
    };
    store.save(&creds)?;
    route.clear()?;
    Ok(format!("{}\nstored in: {}", describe(&creds), store.location()))
}

/// `login-status`: what is stored, and whether the server is known to have revoked it. Makes no network call.
pub fn status_text(store: &CredentialStore) -> Result<String, String> {
    status_text_at(store, SystemClock.now_secs())
}

/// [`status_text`] at a given time, which decides whether a fall back to one request at a time still holds.
pub fn status_text_at(store: &CredentialStore, now: i64) -> Result<String, String> {
    let route = Route::new(store.state_dir()).state();
    let stored = store.load()?;
    let warning = route.active_fallback(now).map(|f| format!("\nwarning: {}", f.message())).unwrap_or_default();
    if let Some(msg) = route.revoked_message() {
        return Ok(format!("{msg}\nno request is made until you sign in again{warning}\nstored in: {}", store.location()));
    }
    let text = match stored {
        Some(c) => {
            format!("{}\n(local view only: this does not ask the server; `login-status --check` can)\nstored in: {}", describe(&c), store.location())
        }
        None => format!("not signed in (run `hoi4-imagegen login`; tokens would go to the {})", store.location()),
    };
    Ok(format!("{text}{warning}"))
}

/// `login-status --check`: ask the server once, when a call that is cheap and sends no image exists.
pub fn status_check(store: &CredentialStore, provider: &ChatGptLoginProvider, ctx: &CallContext) -> Result<String, String> {
    let local = status_text(store)?;
    match provider.check(ctx) {
        Ok(CheckOutcome::Accepted) => Ok(format!("{local}\nserver check: the token endpoint accepted the refresh, the sign-in is alive")),
        Ok(CheckOutcome::NotChecked(why)) => Ok(format!("{local}\nserver check: not made ({why})")),
        Err(e) => Err(e.message),
    }
}

/// Best effort: open the address in the default browser. The address is always printed as well.
pub fn open_browser(url: &str) {
    let result = if cfg!(windows) {
        std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    if result.is_err() {
        eprintln!("(could not open a browser automatically)");
    }
}

/// The sign-in flow with every external part injected, so it can be tested without a browser or a network.
pub fn login(
    store: &CredentialStore,
    transport: &dyn HttpTransport,
    clock: &dyn Clock,
    port: u16,
    timeout: Duration,
    show_url: &mut dyn FnMut(&str),
) -> Result<String, String> {
    let pending = begin(port)?;
    show_url(&pending.url);
    finish(pending, store, transport, clock, timeout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use imagegen_core::config::MapEnv;
    use imagegen_core::hashing::FixedClock;
    use imagegen_providers::transport::{FixtureTransport, Method};
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::path::Path;

    // SYNTHETIC: the browser is a local socket and the token endpoint is a fixture. No network is used.

    fn work_dir(prefix: &str) -> tempfile::TempDir {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join(".work");
        std::fs::create_dir_all(&base).unwrap();
        tempfile::Builder::new().prefix(prefix).tempdir_in(base).unwrap()
    }

    /// Run the whole sign-in flow against a local socket "browser" and a fixture token endpoint.
    fn fixture_login(store: &CredentialStore) -> (String, Vec<String>, FixtureTransport) {
        let fixture = FixtureTransport::new();
        fixture.respond(
            Method::Post,
            "/oauth/token",
            200,
            &[],
            br#"{"access_token":"access-synthetic","refresh_token":"refresh-synthetic","expires_in":3600}"#.to_vec(),
        );

        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let browser = std::thread::spawn(move || {
            let url = rx.recv().unwrap();
            let get = |name: &str| url.split(&['?', '&'][..]).find_map(|kv| kv.strip_prefix(&format!("{name}="))).map(str::to_string).unwrap();
            let state = get("state");
            let port: u16 = {
                let r = get("redirect_uri");
                r.rsplit("%3A").next().unwrap().split('%').next().unwrap().parse().unwrap()
            };
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(s, "GET /auth/callback?code=synthetic-code&state={state} HTTP/1.1\r\n\r\n").unwrap();
            let mut r = String::new();
            s.read_to_string(&mut r).ok();
        });
        let mut shown = Vec::new();
        let text = login(store, &fixture, &FixedClock(1_793_404_800), 0, Duration::from_secs(10), &mut |u| {
            shown.push(u.to_string());
            tx.send(u.to_string()).unwrap();
        })
        .unwrap();
        browser.join().unwrap();
        (text, shown, fixture)
    }

    #[test]
    fn the_whole_sign_in_flow_works_against_a_local_callback_and_a_fixture_token_endpoint() {
        let dir = work_dir("l");
        let store = CredentialStore::at(dir.path().join("auth.json"));
        let (text, shown, fixture) = fixture_login(&store);
        assert!(shown[0].starts_with("https://auth.openai.com/oauth/authorize?"));
        assert!(text.contains("signed in"));
        let saved = store.load().unwrap().unwrap();
        assert_eq!((saved.access.as_str(), saved.refresh.as_deref()), ("access-synthetic", Some("refresh-synthetic")));
        assert_eq!(saved.expires_ms, Some(1_793_404_800_000 + 3_600_000));
        let sent = &fixture.requests()[0];
        let body = String::from_utf8_lossy(&sent.body);
        assert!(body.contains("code=synthetic-code") && body.contains("code_verifier="));
    }

    #[test]
    fn a_new_login_clears_a_recorded_revocation_and_status_says_signed_out_until_then() {
        let dir = work_dir("v");
        let store = CredentialStore::at(dir.path().join("auth.json"));
        store.save(&Credentials { access: "old".into(), refresh: None, account_id: None, expires_ms: None, last_refresh: "t".into() }).unwrap();
        let route = Route::new(store.state_dir());
        route.acquire(&CancelToken::new(), &RealSleeper).unwrap().record_revoked(
            1_793_000_000,
            401,
            b"{\"error\":{\"code\":\"token_revoked\"}}",
            "generation",
        );
        let before = status_text(&store).unwrap();
        assert!(before.starts_with("signed out by the server (token revoked at 2026-"), "{before}");
        assert!(before.contains("hoi4-imagegen login"));

        fixture_login(&store);
        assert_eq!(route.state().revoked, None, "login cleared the record");
        let after = status_text(&store).unwrap();
        assert!(after.starts_with("signed in;"), "{after}");
    }

    #[test]
    fn logout_forgets_a_recorded_revocation_too() {
        let dir = work_dir("o");
        let file = dir.path().join("auth.json").display().to_string();
        let home = dir.path().display().to_string();
        let env = MapEnv::from_pairs(&[(crate::store::PATH_ENV, file.as_str()), ("APPDATA", home.as_str()), ("HOME", home.as_str())]);
        let store = CredentialStore::at(&file);
        Route::new(store.state_dir()).acquire(&CancelToken::new(), &RealSleeper).unwrap().record_revoked(1_793_000_000, 401, b"", "generation");
        assert!(run_with("status", &env, &FixedClock(0)).unwrap().contains("signed out by the server"));
        run_with("logout", &env, &FixedClock(0)).unwrap();
        assert!(run_with("status", &env, &FixedClock(0)).unwrap().contains("not signed in"));
    }

    #[test]
    fn the_risk_note_warns_about_concurrent_use_and_says_what_the_tool_now_does() {
        assert!(RISK_NOTE.contains("revoke") && RISK_NOTE.contains("every Codex sign-in") && RISK_NOTE.contains("signs you out of Codex"));
        assert!(RISK_NOTE.contains("up to three requests at a time") && RISK_NOTE.contains("HOI4_IMAGEGEN_CHATGPT_CONCURRENCY"));
        assert!(RISK_NOTE.contains("falls back to one request at a time"));
        assert!(RISK_NOTE.contains(&format!("{} seconds", crate::route::MIN_GAP_SECS)));
        let text = run_with("risk-note", &MapEnv::from_pairs(&[(crate::store::PATH_ENV, "x.json")]), &FixedClock(0)).unwrap();
        assert_eq!(text, RISK_NOTE, "`login-risk-note` prints the same note `login` shows");
    }

    #[test]
    fn status_and_logout_work_without_a_sign_in() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join(".work");
        std::fs::create_dir_all(&base).unwrap();
        let dir = tempfile::Builder::new().prefix("s").tempdir_in(base).unwrap();
        let file = dir.path().join("auth.json").display().to_string();
        let home = dir.path().display().to_string();
        let env = MapEnv::from_pairs(&[
            (crate::store::PATH_ENV, file.as_str()),
            ("APPDATA", home.as_str()),
            ("HOME", home.as_str()),
            ("XDG_DATA_HOME", home.as_str()),
        ]);
        let clock = FixedClock(0);
        // `login` itself opens a real browser, so the flow is tested through `login` with a fixture endpoint below
        assert!(!crate::consent::accepted(&env));
        assert!(run_with("status", &env, &clock).unwrap().contains("not signed in"));
        CredentialStore::at(&file)
            .save(&Credentials { access: "a".into(), refresh: None, account_id: Some("acct".into()), expires_ms: None, last_refresh: "t".into() })
            .unwrap();
        assert!(run_with("status", &env, &clock).unwrap().contains("acct"));
        assert!(run_with("logout", &env, &clock).unwrap().contains("signed out"));
        assert!(run_with("logout", &env, &clock).unwrap().contains("nothing"));
        assert!(run_with("bogus", &env, &clock).is_err());
    }
}
