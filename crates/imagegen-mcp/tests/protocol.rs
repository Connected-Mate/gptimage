//! L3: the real binary over real stdio.
//!
//! The server is started as a child process and spoken to in newline-delimited JSON-RPC, the way an MCP client
//! does. The "provider" is a mock HTTP server on 127.0.0.1 that returns a synthetic OpenAI-shaped reply: it
//! checks this tool's wire behaviour, and is not provider evidence.

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{openai_ok, repo_work_dir, synthetic_icon_png};
use imagegen_core::hashing::{canonical_hash, sha256_hex};
use serde_json::{json, Value};

/// A tiny OpenAI look-alike on loopback.
struct MockOpenAi {
    port: u16,
    hits: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl MockOpenAi {
    fn start(delay: Duration) -> MockOpenAi {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (h2, s2) = (hits.clone(), stop.clone());
        let body = openai_ok(&synthetic_icon_png());
        let handle = std::thread::spawn(move || {
            while !s2.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).ok();
                        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
                        read_http_request(&mut stream);
                        h2.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(delay);
                        let head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nx-request-id: req_mock\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(head.as_bytes());
                        let _ = stream.write_all(&body);
                        let _ = stream.flush();
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
        });
        MockOpenAi { port, hits, stop, handle: Some(handle) }
    }
}

impl Drop for MockOpenAi {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn read_http_request(stream: &mut TcpStream) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut header_end = None;
    while header_end.is_none() {
        let n = stream.read(&mut chunk).unwrap_or(0);
        if n == 0 {
            return;
        }
        buf.extend_from_slice(&chunk[..n]);
        header_end = buf.windows(4).position(|w| w == b"\r\n\r\n");
    }
    let end = header_end.unwrap();
    let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
    let length = head.lines().find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
    while buf.len() < end + 4 + length {
        let n = stream.read(&mut chunk).unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

struct Client {
    child: Child,
    stdin: Option<ChildStdin>,
    rx: Receiver<Value>,
    stderr: Arc<Mutex<String>>,
    next_id: u64,
    pending: Vec<Value>,
}

impl Client {
    fn start(root: &Path, mock_port: u16) -> Client {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_hoi4-imagegen-mcp"));
        cmd.arg("--project-root").arg(root);
        for var in ["GEMINI_API_KEY", "GOOGLE_API_KEY", "FAL_KEY", "COMFYUI_API_KEY", "HOI4_IMAGEGEN_ALLOWED_HOSTS"] {
            cmd.env_remove(var);
        }
        cmd.env("OPENAI_API_KEY", "sk-test-synthetic-proto")
            .env("HOI4_GAME_DIR", root.join("no-game"))
            .env("HOI4_IMAGEGEN_CHATGPT_AUTH", root.join("chatgpt-auth.json"));
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn().expect("the server starts");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let v: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("stdout carried a non-JSON line ({e}): {line}"));
                if tx.send(v).is_err() {
                    break;
                }
            }
        });
        let stderr = Arc::new(Mutex::new(String::new()));
        let mut err_pipe = child.stderr.take().unwrap();
        let sink = stderr.clone();
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = err_pipe.read_to_string(&mut s);
            sink.lock().unwrap().push_str(&s);
        });
        let _ = mock_port;
        Client { child, stdin: Some(stdin), rx, stderr, next_id: 1, pending: Vec::new() }
    }

    fn send(&mut self, v: &Value) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        writeln!(stdin, "{}", serde_json::to_string(v).unwrap()).unwrap();
        stdin.flush().unwrap();
    }

    /// Send a request without waiting; returns its id.
    fn send_request(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        id
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    /// Wait for the response to `id`; other messages are kept for later.
    fn wait_for(&mut self, id: u64, within: Duration) -> Value {
        if let Some(pos) = self.pending.iter().position(|m| m["id"] == json!(id)) {
            return self.pending.remove(pos);
        }
        let deadline = Instant::now() + within;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(m) if m["id"] == json!(id) => return m,
                Ok(m) => self.pending.push(m),
                Err(_) => panic!("no response to request {id} within {within:?}; server stderr: {}", self.stderr.lock().unwrap()),
            }
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.send_request(method, params);
        self.wait_for(id, Duration::from_secs(60))
    }

    fn initialize(&mut self) -> Value {
        let r = self.request(
            "initialize",
            json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "protocol-test", "version": "0.0.1"}}),
        );
        self.notify("notifications/initialized", json!({}));
        r
    }

    fn call(&mut self, tool: &str, args: Value) -> Value {
        let r = self.request("tools/call", json!({"name": tool, "arguments": args}));
        assert!(r.get("error").is_none(), "{tool} returned a protocol error: {r}");
        r["result"].clone()
    }

    /// Call a tool that is expected to succeed and return its structured content.
    fn ok(&mut self, tool: &str, args: Value) -> Value {
        let r = self.call(tool, args);
        assert_ne!(r["isError"], json!(true), "{tool} failed: {r}");
        r["structuredContent"].clone()
    }

    /// Call a tool that is expected to be refused and return the error object.
    fn refused(&mut self, tool: &str, args: Value) -> Value {
        let r = self.call(tool, args);
        assert_eq!(r["isError"], json!(true), "{tool} should have been refused: {r}");
        r["structuredContent"]["error"].clone()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.stdin.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn project(mock_port: u16) -> tempfile::TempDir {
    let dir = tempfile::Builder::new().prefix("m").tempdir_in(repo_work_dir()).unwrap();
    std::fs::create_dir_all(dir.path().join(".hoi4-imagegen")).unwrap();
    std::fs::write(
        dir.path().join(".hoi4-imagegen/config.toml"),
        format!("[defaults]\nprovider = \"openai\"\n[providers.openai]\nbase_url = \"http://127.0.0.1:{mock_port}\"\nquality = \"low\"\n"),
    )
    .unwrap();
    dir
}

fn golden_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("golden").join("tool_schemas.json")
}

#[test]
fn handshake_lists_exactly_nineteen_tools_with_stable_schemas() {
    let mock = MockOpenAi::start(Duration::from_millis(0));
    let dir = project(mock.port);
    let mut c = Client::start(dir.path(), mock.port);
    let init = c.initialize();
    assert_eq!(init["result"]["serverInfo"]["name"], "hoi4-imagegen-mcp");
    assert!(init["result"]["capabilities"]["tools"].is_object());
    assert!(init["result"]["instructions"].as_str().unwrap().contains("hoi4.image_icon"));

    let list = c.request("tools/list", json!({}));
    let tools = list["result"]["tools"].as_array().unwrap();
    let mut names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    names.sort_unstable();
    let mut want: Vec<&str> = imagegen_mcp::server::TOOL_NAMES.to_vec();
    want.sort_unstable();
    assert_eq!(names, want);
    assert_eq!(tools.len(), 19);

    // each generation tool's schema lists exactly the families of its group, and nothing else
    let mut seen: Vec<String> = Vec::new();
    for g in imagegen_core::Group::ALL {
        let t = tools.iter().find(|t| t["name"] == g.tool_name()).unwrap_or_else(|| panic!("{} is not listed", g.tool_name()));
        let listed: Vec<&str> = t["inputSchema"]["properties"]["family"]["enum"]
            .as_array()
            .unwrap_or_else(|| panic!("{}: no family enum", g.tool_name()))
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(listed, g.family_ids(), "{}", g.tool_name());
        assert!(t["inputSchema"]["required"].as_array().unwrap().iter().any(|r| r == "family"), "{}: family must be required", g.tool_name());
        seen.extend(listed.iter().map(|s| s.to_string()));
    }
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), imagegen_core::Family::ALL.len());
    for gone in ["hoi4.image_plan", "hoi4.image_profiles"] {
        assert!(tools.iter().all(|t| t["name"] != gone), "{gone} was replaced");
    }
    // validate has no way to skip checks and artifacts take no family in review or install
    let props = |n: &str| {
        tools.iter().find(|t| t["name"] == n).unwrap()["inputSchema"]["properties"].as_object().unwrap().keys().cloned().collect::<Vec<_>>()
    };
    assert!(!props("hoi4.image_validate").contains(&"checks".to_string()) && !props("hoi4.image_validate").contains(&"profile".to_string()));
    for n in ["hoi4.image_review", "hoi4.image_install"] {
        assert!(!props(n).iter().any(|k| k == "family" || k == "profile"), "{n}");
    }

    let mut hashes = serde_json::Map::new();
    for t in tools {
        let name = t["name"].as_str().unwrap();
        assert!(name.starts_with("hoi4.image_"));
        assert!(!t["description"].as_str().unwrap_or("").is_empty(), "{name} needs a description");
        assert_eq!(t["inputSchema"]["type"], "object", "{name}");
        // unknown fields are rejected everywhere a tool takes arguments
        if t["inputSchema"].get("properties").map(|p| !p.as_object().unwrap().is_empty()).unwrap_or(false) {
            assert_eq!(t["inputSchema"]["additionalProperties"], json!(false), "{name} must reject unknown fields");
        }
        hashes.insert(name.to_string(), json!(canonical_hash(&t["inputSchema"])));
    }
    assert_eq!(tools.iter().find(|t| t["name"] == "hoi4.image_status").unwrap()["annotations"]["readOnlyHint"], true);
    assert_eq!(tools.iter().find(|t| t["name"] == "hoi4.image_install").unwrap()["annotations"]["destructiveHint"], true);
    // the audit only reads the mod and the game
    let audit = tools.iter().find(|t| t["name"] == "hoi4.image_audit").unwrap();
    assert_eq!(audit["annotations"]["readOnlyHint"], true);
    assert_eq!(audit["annotations"]["destructiveHint"], false);
    assert!(props("hoi4.image_audit").iter().all(|k| ["allowlist", "families", "limit"].contains(&k.as_str())));

    // golden schema hashes: a change here is a change to the tool contract and must be deliberate
    let golden = golden_path();
    if golden.exists() && std::env::var("UPDATE_GOLDEN").is_err() {
        let stored: Value = serde_json::from_slice(&std::fs::read(&golden).unwrap()).unwrap();
        assert_eq!(Value::Object(hashes), stored, "tool schemas changed; rerun with UPDATE_GOLDEN=1 if that is intended");
    } else {
        std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
        std::fs::write(&golden, serde_json::to_vec_pretty(&Value::Object(hashes)).unwrap()).unwrap();
        eprintln!("wrote {}: commit it", golden.display());
    }
}

#[test]
fn status_never_leaks_the_key_and_bad_arguments_are_rejected() {
    let mock = MockOpenAi::start(Duration::from_millis(0));
    let dir = project(mock.port);
    let mut c = Client::start(dir.path(), mock.port);
    c.initialize();
    let r = c.call("hoi4.image_status", json!({}));
    assert_ne!(r["isError"], json!(true));
    assert!(!r.to_string().contains("sk-test-synthetic-proto"), "the key value must never appear");
    let openai = r["structuredContent"]["providers"].as_array().unwrap().iter().find(|p| p["id"] == "openai").expect("openai listed").clone();
    assert_eq!(openai["configured"], true);

    // unknown field: a protocol error or a refused tool call, never a silent accept
    let r = c.request(
        "tools/call",
        json!({"name": "hoi4.image_icon", "arguments": {"family": "focus", "feature_slug": "f", "asset": "a", "brief": "x", "surprise": 1}}),
    );
    assert!(r.get("error").is_some() || r["result"]["isError"] == json!(true), "unknown fields must be rejected: {r}");
    // unknown tool
    let r = c.request("tools/call", json!({"name": "hoi4.image_nope", "arguments": {}}));
    assert!(r.get("error").is_some() || r["result"]["isError"] == json!(true));
    // wrong type
    let r = c.request("tools/call", json!({"name": "hoi4.image_icon", "arguments": {"family": 7}}));
    assert!(r.get("error").is_some() || r["result"]["isError"] == json!(true));
    // unknown category and a traversal attempt come back as structured refusals
    let e = c.refused("hoi4.image_icon", json!({"family": "focus", "feature_slug": "../x", "asset": "a", "brief": "x"}));
    assert_eq!(e["code"], "invalid_argument");
    let e = c.refused("hoi4.image_icon", json!({"family": "focus", "feature_slug": "f", "asset": "a", "brief": "x", "references": [{"path": "../../outside.png", "role": "style_anchor"}]}));
    assert_eq!(e["code"], "path_rejected");
    assert!(e["retry_class"].is_string());
}

#[test]
fn the_whole_loop_runs_over_stdio() {
    let mock = MockOpenAi::start(Duration::from_millis(0));
    let dir = project(mock.port);
    let mut c = Client::start(dir.path(), mock.port);
    c.initialize();

    let plan =
        c.ok("hoi4.image_icon", json!({"family": "focus", "feature_slug": "iron_pact", "asset": "steel_gate", "brief": "a riveted steel gate"}));
    assert_eq!(plan["generated"], false);
    assert_eq!(plan["plan"]["model"], "gpt-image-2.5-flare");
    let plan_id = plan["plan_id"].as_str().unwrap().to_string();

    let gen = c.ok("hoi4.image_generate", json!({"plan_id": plan_id, "idempotency_key": "proto-1"}));
    let cand = gen["candidates"][0]["candidate_id"].as_str().unwrap().to_string();
    assert_eq!(cand, "iron_pact.steel_gate-v1");
    assert_eq!(mock.hits.load(Ordering::SeqCst), 1);
    // an idempotent retry returns the same candidate without a second provider call
    let again = c.ok("hoi4.image_generate", json!({"plan_id": plan_id, "idempotency_key": "proto-1"}));
    assert_eq!(again["candidates"][0]["replayed"], true);
    assert_eq!(mock.hits.load(Ordering::SeqCst), 1);

    let proc = c.ok("hoi4.image_process", json!({"candidate_id": cand}));
    let art = proc["artifact_id"].as_str().unwrap().to_string();

    // installing before validation and review is refused
    let e = c.refused("hoi4.image_install", json!({"target": art, "approval_id": "apr-0000000000000000"}));
    assert_eq!(e["code"], "install_gate_validation_missing");

    let v = c.ok("hoi4.image_validate", json!({"targets": [art]}));
    assert_eq!(v["status"], "pass", "{v:#}");

    // the preview comes back as an image content block as well as structured data
    let pr = c.call("hoi4.image_preview", json!({"target": art}));
    assert_ne!(pr["isError"], json!(true));
    assert!(pr["content"].as_array().unwrap().iter().any(|b| b["type"] == "image" && b["mimeType"] == "image/png"), "{pr}");
    assert!(dir.path().join(pr["structuredContent"]["sheet"].as_str().unwrap()).exists());

    let profile = c.ok("hoi4.image_families", json!({"family": "focus"}));
    let answers: serde_json::Map<String, Value> =
        profile["family"]["rubric_questions"].as_array().unwrap().iter().map(|q| (q["key"].as_str().unwrap().to_string(), json!(true))).collect();
    let rev = c.ok("hoi4.image_review", json!({"target": art, "verdict": "approved", "reviewer": "protocol test", "answers": answers}));
    let approval = rev["approval_id"].as_str().unwrap().to_string();

    let inst = c.ok("hoi4.image_install", json!({"target": art, "approval_id": approval}));
    assert_eq!(inst["final_files"][0]["path"], "gfx/interface/goals/iron_pact/focus_steel_gate.dds");
    let dds = std::fs::read(dir.path().join("gfx/interface/goals/iron_pact/focus_steel_gate.dds")).unwrap();
    assert_eq!(&dds[0..4], b"DDS ");
    assert_eq!(dds.len(), 128 + 100 * 88 * 4);

    // an overwrite attempt is refused and the file is unchanged
    let before = sha256_hex(&dds);
    let e = c.refused("hoi4.image_install", json!({"target": art, "approval_id": approval}));
    assert_eq!(e["code"], "final_path_exists");
    assert_eq!(sha256_hex(&std::fs::read(dir.path().join("gfx/interface/goals/iron_pact/focus_steel_gate.dds")).unwrap()), before);

    // the ledger verifies and shows the lineage
    let l = c.ok("hoi4.image_ledger", json!({"artifact_id": art, "lineage": true}));
    assert_eq!(l["chain"]["ok"], true);
    assert_eq!(l["lineage"], json!([art, cand]));

    // an unknown job cannot be cancelled
    let e = c.refused("hoi4.image_cancel", json!({"idempotency_key": "never-used"}));
    assert_eq!(e["code"], "not_found");
}

#[test]
fn a_running_generation_can_be_cancelled_from_a_second_request() {
    let mock = MockOpenAi::start(Duration::from_millis(2500));
    let dir = project(mock.port);
    let mut c = Client::start(dir.path(), mock.port);
    c.initialize();
    let plan = c.ok("hoi4.image_icon", json!({"family": "focus", "feature_slug": "f", "asset": "a", "brief": "x"}));
    let gen_id =
        c.send_request("tools/call", json!({"name": "hoi4.image_generate", "arguments": {"plan_id": plan["plan_id"], "idempotency_key": "slow-1"}}));
    std::thread::sleep(Duration::from_millis(600));
    let cancel_id = c.send_request("tools/call", json!({"name": "hoi4.image_cancel", "arguments": {"idempotency_key": "slow-1"}}));
    let cancel = c.wait_for(cancel_id, Duration::from_secs(20));
    assert_eq!(cancel["result"]["structuredContent"]["status"], "cancel_requested", "{cancel}");
    assert!(c.pending.iter().all(|m| m["id"] != json!(gen_id)), "the cancel was answered while the generation was still running");
    // a synchronous provider cannot be interrupted once its request is out: the generation still completes
    let gen = c.wait_for(gen_id, Duration::from_secs(60));
    assert_ne!(gen["result"]["isError"], json!(true), "{gen}");
    assert!(cancel["result"]["structuredContent"]["note"].as_str().unwrap().contains("synchronous"));
}

#[test]
fn closing_stdin_ends_the_server_cleanly() {
    let mock = MockOpenAi::start(Duration::from_millis(0));
    let dir = project(mock.port);
    let mut c = Client::start(dir.path(), mock.port);
    c.initialize();
    c.stdin.take();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = c.child.try_wait().unwrap() {
            assert!(status.success(), "exit status {status}; stderr: {}", c.stderr.lock().unwrap());
            break;
        }
        assert!(Instant::now() < deadline, "the server did not exit after stdin closed");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_missing_project_is_a_clean_startup_error() {
    let out = Command::new(env!("CARGO_BIN_EXE_hoi4-imagegen-mcp")).arg("--project-root").arg("definitely/not/a/folder").output().unwrap();
    assert!(!out.status.success());
    assert!(out.stdout.is_empty(), "stdout is reserved for protocol messages");
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot open the project"));
}

#[test]
fn a_group_tool_refuses_another_groups_family_over_the_wire() {
    let mock = MockOpenAi::start(Duration::from_millis(0));
    let dir = project(mock.port);
    let mut c = Client::start(dir.path(), mock.port);
    c.initialize();
    let base = json!({"feature_slug": "f", "asset": "a", "brief": "x"});
    for (tool, family) in [
        ("hoi4.image_icon", "report_event"),
        ("hoi4.image_picture", "focus"),
        ("hoi4.image_flag", "decision"),
        ("hoi4.image_unit", "leader_portrait"),
    ] {
        let mut args = base.clone();
        args["family"] = json!(family);
        let r = c.request("tools/call", json!({"name": tool, "arguments": args}));
        let refused = r["result"]["isError"] == json!(true);
        assert!(r.get("error").is_some() || refused, "{tool} accepted {family}: {r}");
        if refused {
            assert_eq!(r["result"]["structuredContent"]["error"]["code"], "family_not_in_group");
        }
    }
    // the right group answers with a free plan and spends nothing
    let mut args = base.clone();
    args["family"] = json!("report_event");
    let plan = c.ok("hoi4.image_picture", args);
    assert_eq!(plan["generated"], false);
    assert_eq!(plan["plan"]["canvas"], json!([210, 176]));
    assert_eq!(mock.hits.load(Ordering::SeqCst), 0);
}
