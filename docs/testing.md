# Test layers

Evidence kinds stay separate. Nothing marked synthetic is provider evidence.

| Layer | Where | What it proves | Evidence kind |
|---|---|---|---|
| L0 unit | `imagegen-core` | Style descriptors (colour, alpha, outline), deterministic k-medoids, drift model, measured paragraph, sprite parsing and the family shelf table, the 44 families and their seven groups, the animated families (vanilla fields and derivation constants against the measurement, the glow, scroll, spin and key-frame derivations, frame metrics and limits that admit the vanilla strips, the `frameAnimatedSpriteType` block, the looping GIF), **the drift test of every family's canvas, frames, accepted sizes, pixel class and grayscale flag against `data/vanilla-measurement.json` (1.19.4.1)** with commented exceptions, the family-lock size rule (`GFX-FAMILY-MISMATCH`), the pixel reuse guard (`GFX-FAMILY-REUSE`, ledger and folder scan), the 1.25 times icon upscale limit, per-group rubrics, the check-set hash and the gate's family and check-set refusals, profiles with the kit's real `profiles.json` shape and unknown-key tolerance, naming, containment (`..`, drive, UNC, stream, device and 8.3 names, symlinks), exclusive versioned writes under 24 concurrent writers, ledger chain and tamper detection, install gate, sprite registration with the lossless `hoi4-syntax` parser (CRLF, LF, BOM, Windows-1252, braces in comments, nested and repeated `spriteTypes` blocks, duplicates, byte-identical untouched text, frame strips, the shine block), resize determinism, validation rules | Source parsing and synthetic images |
| L0 parity | `imagegen-core/tests/kit_parity.rs` | The Rust DDS writer is **byte-identical** to the kit's `write_bgra_dds` and passes the kit's `strict_dds` rule on synthetic images; alpha statistics and the margin and undersized-subject rules agree with `gfx.py`. `recipe_parity.rs` compares the report card, achievement triplet, advisor card and portrait recipes with the kit's Python tools. Skips with a visible `SKIPPED` line if Python, Pillow, numpy or the kit checkout is missing, unless `IMAGEGEN_REQUIRE_PARITY=1` (CI sets it) | Binary comparison with the kit's own code |
| L1 provider contract | each adapter's tests | Wire format, parsing, error mapping, retry classes, key redaction, host policy. **Synthetic fixtures**, labelled as such. Recordings of real responses exist only for the ChatGPT login (see L8) | Synthetic fixtures (not provider evidence) |
| L1 client | `imagegen-providers/tests/ureq_loopback.rs` | The real HTTP client: bodies, headers, no redirects, size cap, refused connections | Loopback mock server |
| L2 family lock | `imagegen-mcp/tests/families.rs` | An idea artifact cannot be validated, reviewed or installed as another family (no parameter exists); a 65x67 import as focus is refused; installed idea pixels cannot become a decision or focus icon; a provider that echoes another family's art is refused before storage; icon upscale limit; opaque families never keep alpha; validation cannot skip checks and the gate rejects a short check set; every group tool refuses every other group's family at runtime and in its schema; `hoi4.image_families` | Synthetic provider transport |
| L2 pipeline | `imagegen-mcp/tests/pipeline.rs` | Plan to install into a temporary mod: placement, versions, idempotency, cost gate, policy gates (D3, and D4 as amended: vanilla references need no flag and the old key is only a deprecation warning), every install-gate refusal, flags in three sizes, imports, register mode, ledger | Synthetic provider transport |
| L2 style references | `imagegen-mcp/tests/style.rs` | Automatic mod-first references: a mod with its own `idea_spirit` icons uses tier 3 with reasons and never vanilla; a pinned style wins (tier 1); a copy of the asset skill shelf is tier 2 and recorded as vanilla origin; an empty mod falls to vanilla from a fake game folder and sends it with no flag (origin `installed_game` in the ledger, nothing copied into the mod); tiers with fewer than 3 images fall through; same images in differently named folders give the same choice and order; statistics and the prompt paragraph follow a red versus a grey mod; `GFX-STYLE-DRIFT` warns for an off-style candidate, not for an on-style one, and never blocks the install; `style` is in every family's check set | Synthetic images made by the tests, fixture provider |
| L2 real data (local) | `style.rs` `real_game_and_chaos_redux_choose_references`, `real_vanilla_leave_one_out_distances_per_family` | Reads the installed game (1.19.4.1) and the real Chaos Redux checkout, read only: prints the chosen references, reasons and statistics for focus, idea_spirit and report_event, and the leave-one-out distances of every family with vanilla art. Both skip with a reason when the game or the checkout is absent | Real game files, observed |
| L2 chains | `imagegen-mcp/tests/chains.rs` | Every family's processing runs by itself and cannot be skipped, for all recipes. Templates are small images drawn by the tests, not Paradox files | Synthetic images and provider transport |
| L2 animation | `imagegen-mcp/tests/animation.rs` | The four animated families end to end: a glow asked for on black gets its dim frame at 0.17 alpha and a plan's own canvas, a spin turns 20 frames, a scroll loops without a seam, key frames are edits fitted with one crop (and need `frame_candidates` of the same asset), the preview writes a frame sheet and a looping GIF, register mode writes the `frameAnimatedSpriteType` with the vanilla fields, and a still or jumping strip fails the `animation` check | Synthetic images and provider transport |
| L2 audit | `imagegen-mcp/tests/audit.rs` | `hoi4.image_audit` on a synthetic mod: traced uses per family, `GFX-FAMILY-REUSE` as a warning with every use, `GFX-CANVAS-SIZE` from headers, `GFX-CARD-CORNERS` from decoded pixels, missing textures, shine companions, the allowlist (default file and named file), family filter, limit; install refuses a sprite name another family already uses and passes with an allowlist alias or the same family | Synthetic mod, shared engine trace |
| L3 protocol | `imagegen-mcp/tests/protocol.rs` | The real binary over stdio: handshake, twenty tools, each group tool's `family` enum listing exactly its group, schema hashes, argument rejection, the full loop, preview as an image block, cancel from a second request, clean exit | Loopback mock server |
| L8 ChatGPT login | `imagegen-mcp/tests/plugin_gates.rs`, `provider-chatgpt-login` and its `tests/recorded_replay.rs` | The route registers in local modes and never in hosted mode, even when signed in and named in the project file. PKCE (RFC 7636 vector), token requests, SSE parsing, token refresh, the sign-in flow against a local socket, the credential store, and gpt-6-luna at high effort enforced: another model, a lower effort or a missing report is refused. A moderation refusal is reported as one. Faults are injected into **recorded-real** responses (`tests/fixtures/recorded-real/`, captured 2026-11-01). No request leaves the machine | Recorded-real replay, synthetic fixtures and loopback |

## Unavailable, and why

| Layer | Why |
|---|---|
| L4 real-provider end to end for OpenAI, Gemini, fal and ComfyUI; recordings for their L1 | No provider key with balance was available. The ChatGPT-login route had one signed-in probe (`docs/evidence/2026-11-01-luna-probe.json`) |
| L5 in game, including the `_shine` question | Needs a user-operated game session |
| L6 keychain behaviour on macOS and Linux | CI runs the test suite on Windows, macOS and Linux, but the real credential stores of macOS and Linux were not exercised by a person |
| L7 HOI4 Factory gates | Hosted-mode tests on the Factory side belong to that repository |
| L9 model-id drift | A scheduled job comparing the tables with provider model lists; not built |

## Running

```sh
export CARGO_TARGET_DIR=<a shared target folder>   # optional; share one build cache when disk is tight
cargo test --workspace --no-fail-fast --locked
cargo test -p imagegen-core --test face_detect -- --nocapture   # YuNet on real portraits; skips without the model
```

CI (`.github/workflows/ci.yml`) runs format, clippy with warnings denied and the workspace tests on Windows, macOS and Linux, plus the Python parity tests against the kit on Linux. For the current test counts, see CI. Counts are not copied here because they go stale.

## Style drift threshold: the measurement

`real_vanilla_leave_one_out_distances_per_family` (installed game 1.19.4.1, debug build, descriptors as in `styledesc.rs`) leaves each vanilla image out in turn, measures its RMS z-score against the rest of its family and reports the 95th percentile. The threshold of `GFX-STYLE-DRIFT` is that percentile, never below 1.75.

| Families with vanilla art | 38 (`gui_texture` and `super_event` have none) |
|---|---|
| Per-family p95, minimum / median / maximum | 0.60 / 1.53 / 6.70 |
| Large families (250 or more measured) | 1.02 to 1.65 |
| Small families | up to 2.15; `agency_upgrade` (15 images, one far outlier) 6.70 |

Examples (images measured, p95, threshold): focus 256, 1.57, 1.75; idea_spirit 255, 1.48, 1.75; decision 253, 1.90, 1.90; medal 41, 2.15, 2.15; report_event 256, 1.06, 1.75; flag 253, 1.52, 1.75. The floor of 1.75 sits just above the large families' values, so a family drawn from many images warns about roughly the outer 5 percent, and a family with few images (a wide spread in its own set) is judged by its own spread. Portrait families share one vanilla pool (all of `gfx/leaders`) because vanilla does not separate them by folder.
