# Upstream

`hoi4-imagegen-mcp` is a derivative of [Connected-Mate/gptimage](https://github.com/Connected-Mate/gptimage), released under the MIT license, copyright (c) 2026 Alex-Connected-Mate. This repository was created by cloning that project with its full history and then rewriting it in Rust. The original license text and copyright line are kept in `LICENSE`; ours is added below it.

| | |
|---|---|
| Upstream | https://github.com/Connected-Mate/gptimage |
| Pinned commit | `dfb4948b6877b5b7febf73e1342fcfacd862a1ed` (2026-10-05, "AGENT-INSTALL: French version of the starter prompt") |
| Upstream package | `gptimage` 0.1.0, JavaScript (ESM), Node 20 or newer, 20 files, 1,420 lines of source, no tests |
| Upstream history | Kept: `git log` shows the eight upstream commits below our first commit. The remote is named `upstream` and its push URL is disabled |
| Relationship | Rewrite. The upstream JavaScript is removed from the tree in the commit "Remove the upstream JavaScript implementation" and stays readable with `git show dfb4948:<path>` |

## Where each upstream file went

| Upstream file | What it did | Replacement here |
|---|---|---|
| `src/server.js` (136 lines) | MCP stdio server with `generate_image`, `list_references`, `image_auth_status` | `crates/imagegen-mcp` (`src/server.rs`, on `rmcp`): thirteen `hoi4.image_*` tools. The tool shape (prompt, reference images, one image per call, automatic versioning) is kept in spirit; `out` is no longer a free path but derives from the category profile |
| `src/gen.js` (48 lines) | Command-line image generation | `crates/imagegen-cli` (`hoi4-imagegen`): `doctor`, `calibrate`, `plan`, `generate`, `validate`, `install`, `call` |
| `src/images.js` (96 lines) | Read references from any path; save PNGs with `-vN` naming via an existence check | `crates/imagegen-core/src/fsio.rs` and `refs.rs`. Containment (no `..`, drive, UNC, stream, device-name, 8.3 or symlink escapes), real format detection, size and pixel caps, clean re-encoding, and exclusive create (temp file plus hard link) so two writers cannot pick the same `-vN` |
| `src/auth.js` (211 lines) | Token store with Codex CLI fallback and write-back, JWT claims, refresh | `crates/provider-chatgpt-login/src/{oauth,store,provider}.rs`, **opt-in and not in default builds**. The `~/.codex/auth.json` fallback and write-back and the token environment variable are dropped |
| `src/login.js` (255 lines) | Browser sign-in: OAuth with PKCE, loopback callback on port 1455 | `crates/provider-chatgpt-login/src/{oauth,cli}.rs` (same flow, same plugin) |
| `src/codex.js` (119 lines) | `POST` to the ChatGPT Codex `responses` endpoint, force the image tool, parse the event stream | `crates/provider-chatgpt-login/src/provider.rs` (same plugin). The documented providers are in `crates/imagegen-providers` |
| `src/banner.js`, `src/banner.txt`, `scripts/build-banner.mjs`, `assets/*.png` | Terminal banner and artwork | Dropped. The two PNGs have no separate license statement and are not copied |
| `install.sh`, `AGENT-INSTALL.md`, `QUICKSTART.md` | Per-client installers and a paste-in agent prompt | Dropped. Replaced by the HOI4 Mod Setup component (a later phase) and `hoi4-imagegen doctor` |
| `skill/gptimage/SKILL.md` | Reference-first workflow for the agent | `skill/hoi4-imagegen/SKILL.md`. The workflow text (ask for references, keep them in `references/`, feed feedback back in, one image per call) is reused in spirit. The Pinterest and Dribbble sourcing advice is not reused: it conflicts with the kit's source and rights rules |
| `README.md`, `package.json`, `package-lock.json`, `.gitignore` | Docs and Node packaging | `README.md` and the Cargo workspace |
| `LICENSE` | MIT | Kept, with our line added |

## What is not carried over, and why

* The upstream default is an impersonation of the OpenAI Codex CLI against an undocumented endpoint. Here the defaults are documented APIs with API keys. The upstream route survives only as the opt-in plugin, with its risks written down in `docs/chatgpt-login.md`.
* Upstream writes wherever the caller points it. This project writes only inside the mod project, never overwrites, and records provenance.
* Upstream has no tests. This workspace tests every layer that can run without a paid call.

## Attribution rules for new files

A file that ports an upstream file carries a header naming that file and the pinned commit (the plugin crate does). Code with no upstream counterpart (profiles, processing, validation, the DDS and TGA writers, the ledger, the install gate, the provider adapters other than the plugin) is original to this project.
