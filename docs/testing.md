# Test layers

Evidence kinds stay separate. Nothing marked synthetic is provider evidence.

| Layer | Where | What it proves | Evidence kind |
|---|---|---|---|
| L0 unit | `imagegen-core` (100+ tests) | Profiles with the kit's real `profiles.json` shape and unknown-key tolerance, naming, containment (`..`, drive, UNC, stream, device and 8.3 names, symlinks), exclusive versioned writes under 24 concurrent writers, ledger chain and tamper detection, install gate, sprite registration, resize determinism, validation rules | Source parsing and synthetic images |
| L0 parity | `imagegen-core/tests/kit_parity.rs` | The Rust DDS writer is **byte-identical** to the kit's `write_bgra_dds` and passes the kit's `strict_dds` rule on 13 synthetic images; alpha statistics and the margin and undersized-subject rules agree with `gfx.py`. Skips with a visible `SKIPPED` line if Python, Pillow, numpy or the kit checkout is missing | Binary comparison with the kit's own code |
| L1 provider contract | each adapter's tests | Wire format, parsing, error mapping, retry classes, key redaction, host policy. **Synthetic fixtures**, labelled as such. No recordings of real responses exist yet | Synthetic fixtures (not provider evidence) |
| L1 client | `imagegen-providers/tests/ureq_loopback.rs` | The real HTTP client: bodies, headers, no redirects, size cap, refused connections | Loopback mock server |
| L2 pipeline | `imagegen-mcp/tests/pipeline.rs` | Plan to install into a temporary mod: placement, versions, idempotency, cost gate, policy gates (D3, D4), every install-gate refusal, flags in three sizes, imports, register mode, ledger | Synthetic provider transport |
| L3 protocol | `imagegen-mcp/tests/protocol.rs` | The real binary over stdio: handshake, thirteen tools, schema hashes, argument rejection, the full loop, preview as an image block, cancel from a second request, clean exit | Loopback mock server |
| L8 isolation | `imagegen-mcp/tests/plugin_isolation.rs` | The ChatGPT-login plugin is absent from default builds | Source and binary checks |

## Unavailable, and why

| Layer | Why |
|---|---|
| L4 real-provider end to end; recordings for L1 | No provider key with balance is known to exist (the maintainer's OpenAI keys have had no balance) |
| L5 in game, including the `_shine` question | Needs a user-operated game session |
| L6 other platforms | This phase ran on Windows only. Path rules and keyring behaviour need a macOS and Linux run |
| L7 HOI4 Factory gates | Factory integration is a design only (D-13) |
| L9 model-id drift | A scheduled job comparing the tables with provider model lists; not built |

## Running

```sh
export CARGO_TARGET_DIR=<a shared target folder>   # disk is tight; share one build cache
cargo test --no-fail-fast                         # default members
cargo test -p provider-chatgpt-login              # the plugin, which is not a default member
cargo test -p imagegen-mcp --features unofficial-chatgpt-login   # plugin wiring
```
