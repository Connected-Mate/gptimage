//! `hoi4-imagegen chatgpt-login login|status|logout`: the only code that starts a sign-in.
//!
//! Signing in is a deliberate act by the account holder: the URL is printed for the user to open themselves,
//! the code is received on 127.0.0.1, and the tokens are stored in this plugin's own file. Nothing here runs
//! unless the user types the command, and `login` also needs the environment acknowledgement.

use std::time::Duration;

use imagegen_core::config::EnvSource;
use imagegen_core::hashing::Clock;
use imagegen_providers::transport::{send, HostPolicy, HttpTransport, UreqTransport};

use crate::oauth::{account_id, authorize_url, bind_listener, exchange_request, expiry_ms, make_pkce, make_state, parse_tokens, wait_for_callback};
use crate::provider::describe;
use crate::store::{default_path, CredentialStore, Credentials};
use crate::{evaluate_gates, PluginMode, ACK_ENV, CALLBACK_PORT, HOSTS};

/// Run a sign-in action. Returns the text to show the user.
pub fn run(action: &str, env: &dyn EnvSource, clock: &dyn Clock) -> Result<String, String> {
    let store = CredentialStore::at(default_path(env));
    match action {
        "status" => Ok(match store.load()? {
            Some(c) => format!("{}\ncredential file: {}", describe(&c), store.path().display()),
            None => format!("not signed in (credential file would be {})", store.path().display()),
        }),
        "logout" => Ok(if store.delete()? { format!("deleted {}", store.path().display()) } else { "nothing to delete".to_string() }),
        "login" => {
            evaluate_gates(true, env.get(ACK_ENV).as_deref(), PluginMode::Cli).map_err(|r| r.join("; "))?;
            login(&store, &UreqTransport::new(), clock, CALLBACK_PORT, Duration::from_secs(300), &mut |url| {
                eprintln!("Open this address in your browser and sign in with your own ChatGPT account:\n\n{url}\n");
            })
        }
        other => Err(format!("unknown action '{other}': use login, status or logout")),
    }
}

/// The sign-in flow with every external part injected, so it can be tested without a browser or a network.
pub fn login(store: &CredentialStore, transport: &dyn HttpTransport, clock: &dyn Clock, port: u16, timeout: Duration, show_url: &mut dyn FnMut(&str)) -> Result<String, String> {
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
    Ok(format!("{}\ncredential file: {}", describe(&creds), store.path().display()))
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
        fixture.respond(Method::Post, "/oauth/token", 200, &[], br#"{"access_token":"access-synthetic","refresh_token":"refresh-synthetic","expires_in":3600}"#.to_vec());

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
    fn login_needs_the_acknowledgement_and_status_logout_work_on_the_own_file() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join(".work");
        std::fs::create_dir_all(&base).unwrap();
        let dir = tempfile::Builder::new().prefix("s").tempdir_in(base).unwrap();
        let file = dir.path().join("auth.json").display().to_string();
        let env = MapEnv::from_pairs(&[(crate::store::PATH_ENV, &file)]);
        let clock = FixedClock(0);
        assert!(run("login", &env, &clock).unwrap_err().contains(ACK_ENV));
        assert!(run("status", &env, &clock).unwrap().contains("not signed in"));
        CredentialStore::at(&file)
            .save(&Credentials { access: "a".into(), refresh: None, account_id: Some("acct".into()), expires_ms: None, last_refresh: "t".into() })
            .unwrap();
        assert!(run("status", &env, &clock).unwrap().contains("acct"));
        assert!(run("logout", &env, &clock).unwrap().contains("deleted"));
        assert!(run("logout", &env, &clock).unwrap().contains("nothing"));
        assert!(run("bogus", &env, &clock).is_err());
    }
}
