# Test layers

Evidence kinds stay separate. Nothing marked synthetic is provider evidence.

| Layer | Where | What it proves | Evidence kind |
|---|---|---|---|
| L0 unit | `imagegen-core` (189 tests) | Style descriptors (colour, alpha, outline), deterministic k-medoids, drift model, measured paragraph, sprite parsing and the family shelf table, the 40 families and their groups, **the drift test of every family's canvas, frames, accepted sizes, pixel class and grayscale flag against `data/vanilla-measurement.json` (1.19.4.1)** with commented exceptions, the family-lock size rule (`GFX-FAMILY-MISMATCH`), the pixel reuse guard (`GFX-FAMILY-REUSE`, ledger and folder scan), the 1.25 times icon upscale limit, per-group rubrics, the check-set hash and the gate's family and check-set refusals, profiles with the kit's real `profiles.json` shape and unknown-key tolerance, naming, containment (`..`, drive, UNC, stream, device and 8.3 names, symlinks), exclusive versioned writes under 24 concurrent writers, ledger chain and tamper detection, install gate, sprite registration, resize determinism, validation rules | Source parsing and synthetic images |
| L0 parity | `imagegen-core/tests/kit_parity.rs` | The Rust DDS writer is **byte-identical** to the kit's `write_bgra_dds` and passes the kit's `strict_dds` rule on 13 synthetic images; alpha statistics and the margin and undersized-subject rules agree with `gfx.py`. Skips with a visible `SKIPPED` line if Python, Pillow, numpy or the kit checkout is missing | Binary comparison with the kit's own code |
| L1 provider contract | each adapter's tests | Wire format, parsing, error mapping, retry classes, key redaction, host policy. **Synthetic fixtures**, labelled as such. No recordings of real responses exist yet | Synthetic fixtures (not provider evidence) |
| L1 client | `imagegen-providers/tests/ureq_loopback.rs` | The real HTTP client: bodies, headers, no redirects, size cap, refused connections | Loopback mock server |
| L2 family lock | `imagegen-mcp/tests/families.rs` (10 tests) | An idea artifact cannot be validated, reviewed or installed as another family (no parameter exists); a 65x67 import as focus is refused; installed idea pixels cannot become a decision or focus icon; a provider that echoes another family's art is refused before storage; icon upscale limit; opaque families never keep alpha; validation cannot skip checks and the gate rejects a short check set; every group tool refuses every other group's family at runtime and in its schema; `hoi4.image_families` | Synthetic provider transport |
| L2 pipeline | `imagegen-mcp/tests/pipeline.rs` (23 tests) | Plan to install into a temporary mod: placement, versions, idempotency, cost gate, policy gates (D3, and D4 as amended: vanilla references need no flag and the old key is only a deprecation warning), every install-gate refusal, flags in three sizes, imports, register mode, ledger | Synthetic provider transport |
| L2 style references | `imagegen-mcp/tests/style.rs` (11 tests) | Automatic mod-first references: a mod with its own `idea_spirit` icons uses tier 3 with reasons and never vanilla; a pinned style wins (tier 1); a copy of the asset skill shelf is tier 2 and recorded as vanilla origin; an empty mod falls to vanilla from a fake game folder and sends it with no flag (origin `installed_game` in the ledger, nothing copied into the mod); tiers with fewer than 3 images fall through; same images in differently named folders give the same choice and order; statistics and the prompt paragraph follow a red versus a grey mod; `GFX-STYLE-DRIFT` warns for an off-style candidate, not for an on-style one, and never blocks the install; `style` is in every family's check set | Synthetic images made by the tests, fixture provider |
| L2 real data (local) | `style.rs` `real_game_and_chaos_redux_choose_references`, `real_vanilla_leave_one_out_distances_per_family` | Reads the installed game (1.19.4.1) and the real Chaos Redux checkout, read only: prints the chosen references, reasons and statistics for focus, idea_spirit and report_event, and the leave-one-out distances of every family with vanilla art. Both skip with a reason when the game or the checkout is absent | Real game files, observed |
| L3 protocol | `imagegen-mcp/tests/protocol.rs` | The real binary over stdio: handshake, eighteen tools, each group tool's `family` enum listing exactly its group, schema hashes, argument rejection, the full loop, preview as an image block, cancel from a second request, clean exit | Loopback mock server |
| L8 gates | `imagegen-mcp/tests/plugin_gates.rs` (5 tests) and `provider-chatgpt-login` (27 tests) | Default build ignores a config request and says why; with the feature, each of the four gates is needed on its own; PKCE (RFC 7636 vector), token requests, SSE parsing, token refresh, the sign-in flow against a local socket, own-file credential store. No request leaves the machine | Synthetic fixtures and loopback |

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
cargo test -p imagegen-mcp --test plugin_gates   # ChatGPT-login wiring (D-12a): local default, hosted refusal

## Last full run (Windows 11, debug build)

`cargo test --no-fail-fast`: core 102 + kit parity 1, providers 69 + loopback 6, mcp 9 + pipeline 23 + plugin gates 1 + plugin isolation 5 + protocol 6, cli 0. `cargo test -p provider-chatgpt-login`: 24. `cargo test -p imagegen-mcp --features unofficial-chatgpt-login`: all pass (pipeline 23, gates 1, isolation 3, protocol 6, unit 9). Zero failures.
```

## Counts

`cargo test --workspace --no-fail-fast` on Windows, 2026-11-01: 291 passed, 0 failed (core 134 + parity 1, mcp 9 unit + 10 families + 23 pipeline + 5 plugin gates + 7 protocol, providers 69 + 6 loopback, ChatGPT login 27).

## Style drift threshold: the measurement

`real_vanilla_leave_one_out_distances_per_family` (installed game 1.19.4.1, debug build, descriptors as in `styledesc.rs`) leaves each vanilla image out in turn, measures its RMS z-score against the rest of its family and reports the 95th percentile. The threshold of `GFX-STYLE-DRIFT` is that percentile, never below 1.75.

| Families with vanilla art | 38 (`gui_texture` and `super_event` have none) |
|---|---|
| Per-family p95, minimum / median / maximum | 0.60 / 1.53 / 6.70 |
| Large families (250 or more measured) | 1.02 to 1.65 |
| Small families | up to 2.15; `agency_upgrade` (15 images, one far outlier) 6.70 |

Examples (images measured, p95, threshold): focus 256, 1.57, 1.75; idea_spirit 255, 1.48, 1.75; decision 253, 1.90, 1.90; medal 41, 2.15, 2.15; report_event 256, 1.06, 1.75; flag 253, 1.52, 1.75. The floor of 1.75 sits just above the large families' values, so a family drawn from many images warns about roughly the outer 5 percent, and a family with few images (a wide spread in its own set) is judged by its own spread. Portrait families share one vanilla pool (all of `gfx/leaders`) because vanilla does not separate them by folder.
