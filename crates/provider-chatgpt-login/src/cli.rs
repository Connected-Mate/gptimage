//! `hoi4-imagegen login|status|logout` (also `hoi4-imagegen chatgpt-login <action>`): the only code that starts a
//! sign-in.
//!
//! Signing in is a deliberate act by the account holder. The risk note is shown once and must be accepted, the
//! sign-in page opens in the user's browser (the address is also printed), the code arrives on 127.0.0.1, and the
//! tokens go to the operating system's credential store. Nothing here runs unless the user types the command.

use std::io::{BufRead, IsTerminal};
use std::time::Duration;

use imagegen_core::config::EnvSource;
use imagegen_core::hashing::Clock;
use imagegen_providers::transport::{send, HostPolicy, HttpTransport, UreqTransport};

use crate::consent::{self, RISK_NOTE};
use crate::oauth::{account_id, authorize_url, bind_listener, exchange_request, expiry_ms, make_pkce, make_state, parse_tokens, wait_for_callback};
use crate::provider::describe;
use crate::store::{CredentialStore, Credentials};
use crate::{evaluate_gates, PluginMode, CALLBACK_PORT, HOSTS};

/// Run a sign-in action. `accept_risk` is the `--accept-risk` flag; without it an interactive terminal is asked.
/// Returns the text to show the user.
pub fn run(action: &str, env: &dyn EnvSource, clock: &dyn Clock, accept_risk: bool) -> Result<String, String> {
    run_with(action, env, clock, accept_risk, &mut ask_yes)
}

/// [`run`] with the terminal question injected (tests).
pub fn run_with(action: &str, env: &dyn EnvSource, clock: &dyn Clock, accept_risk: bool, ask: &mut dyn FnMut() -> bool) -> Result<String, String> {
    evaluate_gates(PluginMode::Cli)?;
    let store = CredentialStore::for_environment(env)?;
    match action {
        "risk-note" => Ok(RISK_NOTE.to_string()),
        "status" => Ok(match store.load()? {
            Some(c) => format!("{}\nstored in: {}", describe(&c), store.location()),
            None => format!("not signed in (run `hoi4-imagegen login`; tokens would go to the {})", store.location()),
        }),
        "logout" => Ok(if store.delete()? {
            format!("signed out; removed the tokens from the {}", store.location())
        } else {
            "nothing to remove".to_string()
        }),
        "login" => {
            if !consent::accepted(env) {
                eprintln!("{RISK_NOTE}");
                let ok = accept_risk || ask();
                if !ok {
                    return Err("sign-in cancelled: the risk note was not accepted (type yes, or pass --accept-risk)".into());
                }
                consent::record(env, &clock.now_rfc3339())?;
            }
            login(&store, &UreqTransport::new(), clock, CALLBACK_PORT, Duration::from_secs(300), &mut |url| {
                eprintln!("Sign in with your own ChatGPT account in the browser. If it does not open, visit:\n\n{url}\n");
                open_browser(url);
            })
        }
        other => Err(format!("unknown action '{other}': use login, status or logout")),
    }
}

fn ask_yes() -> bool {
    if !std::io::stdin().is_terminal() {
        return false;
    }
    eprint!("Type yes to accept and continue: ");
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).is_ok() && line.trim().eq_ignore_ascii_case("yes")
}

/// Best effort: open the address in the default browser. The address is always printed as well.
fn open_browser(url: &str) {
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
    let pkce = make_pkce()?;
    let state = make_state()?;
    let listener = bind_listener(port)?;
    let actual_port = listener.local_addr().map_err(|e| e.to_string())?.port();
    show_url(&authorize_url(&pkce.challenge, &state, actual_port));
    let code = wait_for_callback(&listener, &state, timeout)?;
    let resp = send(transport, &HostPolicy::api(&HOSTS), &exchange_request(&code, &pkce.verifier, actual_port)).map_err(|e| e.message)?;
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
    Ok(format!("{}\nstored in: {}", describe(&creds), store.location()))
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

    #[test]
    fn the_whole_sign_in_flow_works_against_a_local_callback_and_a_fixture_token_endpoint() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join(".work");
        std::fs::create_dir_all(&base).unwrap();
        let dir = tempfile::Builder::new().prefix("l").tempdir_in(base).unwrap();
        let store = CredentialStore::at(dir.path().join("auth.json"));
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
        let text = login(&store, &fixture, &FixedClock(1_793_404_800), 0, Duration::from_secs(10), &mut |u| {
            shown.push(u.to_string());
            tx.send(u.to_string()).unwrap();
        })
        .unwrap();
        browser.join().unwrap();
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
    fn login_without_accepting_the_note_is_cancelled_and_status_logout_work() {
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
        assert!(
            run_with("login", &env, &clock, false, &mut || false).unwrap_err().contains("not accepted"),
            "a test process has no terminal, so the note cannot be accepted silently"
        );
        assert!(!crate::consent::accepted(&env));
        assert!(run_with("status", &env, &clock, false, &mut || false).unwrap().contains("not signed in"));
        CredentialStore::at(&file)
            .save(&Credentials { access: "a".into(), refresh: None, account_id: Some("acct".into()), expires_ms: None, last_refresh: "t".into() })
            .unwrap();
        assert!(run_with("status", &env, &clock, false, &mut || false).unwrap().contains("acct"));
        assert!(run_with("logout", &env, &clock, false, &mut || false).unwrap().contains("signed out"));
        assert!(run_with("logout", &env, &clock, false, &mut || false).unwrap().contains("nothing"));
        assert!(run_with("bogus", &env, &clock, false, &mut || false).is_err());
    }
}
