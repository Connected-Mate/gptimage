//! L8: the opt-in ChatGPT-login plugin is never part of a default build.
//!
//! Four independent checks, so a mistake in one still leaves the others standing:
//!  1. `cargo metadata`: the plugin is a workspace member but not a default member;
//!  2. `cargo tree` with default features lists it for none of the default crates, and does list it when the
//!     feature is on (the positive control that proves the check can see it);
//!  3. the compiled default `hoi4-imagegen-mcp` binary contains none of the plugin's identifying strings;
//!  4. no default crate's source or manifest mentions the plugin's endpoints or crate.
//!
//! The binary and source scans are skipped when this test itself is built with the feature on, because then the
//! binary legitimately contains the plugin.

#![cfg_attr(feature = "unofficial-chatgpt-login", allow(dead_code))]

#![cfg_attr(feature = "unofficial-chatgpt-login", allow(dead_code))]

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// Strings that identify the plugin's backend and client impersonation.
const FORBIDDEN: [&str; 4] = ["chatgpt.com", "auth.openai.com", "codex_cli_rs", "backend-api/codex"];
const PLUGIN: &str = "provider-chatgpt-login";

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn cargo() -> Command {
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.current_dir(workspace_root()).env_remove("RUSTFLAGS");
    c
}

/// `features` are written `package/feature`, which works in a virtual workspace without relying on `-p` feature selection.
fn tree(package: &str, features: &[&str]) -> String {
    let mut cmd = cargo();
    cmd.args(["tree", "--offline", "--locked", "-e", "normal", "--prefix", "none", "-p", package]);
    if !features.is_empty() {
        cmd.args(["-F", &features.join(",")]);
    }
    let out = cmd.output().expect("cargo tree runs");
    assert!(out.status.success(), "cargo tree -p {package} failed:\n{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn the_plugin_is_a_workspace_member_but_not_a_default_member() {
    let out = cargo().args(["metadata", "--offline", "--locked", "--no-deps", "--format-version", "1"]).output().expect("cargo metadata runs");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let meta: Value = serde_json::from_slice(&out.stdout).unwrap();
    let ids = |key: &str| -> Vec<String> { meta[key].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect() };
    assert!(ids("workspace_members").iter().any(|m| m.contains(PLUGIN)), "the plugin must stay in the workspace so it shares the lock file and has tests");
    assert!(!ids("workspace_default_members").iter().any(|m| m.contains(PLUGIN)), "the plugin must not be a default member");
    assert_eq!(ids("workspace_default_members").len(), 4);
}

#[test]
fn no_default_crate_depends_on_the_plugin() {
    for pkg in ["imagegen-core", "imagegen-providers", "imagegen-mcp", "imagegen-cli"] {
        let t = tree(pkg, &[]);
        assert!(!t.contains(PLUGIN), "{pkg} pulls in the plugin with default features:\n{t}");
    }
}

#[test]
fn the_feature_pulls_it_in_proving_the_check_can_see_it() {
    let t = tree("imagegen-mcp", &["imagegen-mcp/unofficial-chatgpt-login"]);
    assert!(t.contains(PLUGIN), "positive control failed: cargo tree does not show the plugin even with the feature on:\n{t}");
    let t = tree("imagegen-cli", &["imagegen-cli/unofficial-chatgpt-login"]);
    assert!(t.contains(PLUGIN));
}

#[cfg(not(feature = "unofficial-chatgpt-login"))]
#[test]
fn the_default_binary_contains_none_of_the_plugins_identifying_strings() {
    let exe = env!("CARGO_BIN_EXE_hoi4-imagegen-mcp");
    let bytes = std::fs::read(exe).unwrap();
    assert!(bytes.len() > 100_000, "suspiciously small binary: {exe}");
    for needle in FORBIDDEN {
        assert!(!contains(&bytes, needle.as_bytes()), "the default binary contains '{needle}'");
        // UTF-16 too: Windows string tables
        let wide: Vec<u8> = needle.encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert!(!contains(&bytes, &wide), "the default binary contains '{needle}' as UTF-16");
    }
    // the scan itself works
    assert!(contains(b"xx chatgpt.com xx", b"chatgpt.com"));
    assert!(!contains(b"xx chatgpt xx", b"chatgpt.com"));
}

#[cfg(not(feature = "unofficial-chatgpt-login"))]
#[test]
fn no_default_crate_mentions_the_plugin_or_its_endpoints() {
    let root = workspace_root();
    let mut offenders = Vec::new();
    for krate in ["imagegen-core", "imagegen-providers", "imagegen-mcp", "imagegen-cli"] {
        scan(&root.join("crates").join(krate), krate, &mut offenders);
    }
    assert!(offenders.is_empty(), "default crates mention the plugin's identifiers:\n{}", offenders.join("\n"));
}

#[cfg(not(feature = "unofficial-chatgpt-login"))]
fn scan(dir: &Path, krate: &str, offenders: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if name != "tests" && name != "target" {
                scan(&path, krate, offenders);
            }
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !matches!(ext, "rs" | "toml" | "json") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        for needle in FORBIDDEN {
            if text.contains(needle) {
                offenders.push(format!("{}: contains '{needle}'", path.display()));
            }
        }
        // only the two adapters that gate it may name the crate, and only imagegen-mcp and imagegen-cli as an optional dependency
        let is_manifest = name == "Cargo.toml";
        let may_name_crate = matches!(krate, "imagegen-mcp" | "imagegen-cli") && (is_manifest || name == "plugin.rs" || name == "lib.rs" || name == "server.rs");
        if text.contains(PLUGIN) && !may_name_crate {
            offenders.push(format!("{}: names {PLUGIN}", path.display()));
        }
        if is_manifest && text.contains(PLUGIN) {
            let line = text.lines().find(|l| l.contains(PLUGIN)).unwrap_or("");
            assert!(line.contains("optional = true") || line.contains("dep:") || line.contains("unofficial-chatgpt-login"), "{}: the plugin must be an optional dependency: {line}", path.display());
        }
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    needle.is_empty() || hay.windows(needle.len()).any(|w| w == needle)
}
