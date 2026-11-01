# Test layers

Evidence kinds stay separate. Nothing marked synthetic is provider evidence.

| Layer | Where | What it proves | Evidence kind |
|---|---|---|---|
| L0 unit | `imagegen-core` (204 tests) | The 40 families and their groups, **the drift test of every family's canvas, frames, accepted sizes, pixel class and grayscale flag against `data/vanilla-measurement.json` (1.19.4.1)** with commented exceptions, the family-lock size rule (`GFX-FAMILY-MISMATCH`), the pixel reuse guard (`GFX-FAMILY-REUSE`, ledger and folder scan), the 1.25 times icon upscale limit, per-group rubrics, the check-set hash and the gate's family and check-set refusals, profiles with the kit's real `profiles.json` shape and unknown-key tolerance, naming, containment (`..`, drive, UNC, stream, device and 8.3 names, symlinks), exclusive versioned writes under 24 concurrent writers, ledger chain and tamper detection, install gate, sprite registration, resize determinism, validation rules | Source parsing and synthetic images, **the processing chains (pre-step whitelist, the recipe of every family, the audit of a processed record, the one-hash bundle of companions)**, **the new ops (`report_card`, `selected_frame`, `disabled_frame`, `conform`)**, **the frame treatments measured against the installed game (skips without it)**, the card checks (`card_corners`, `near_opaque`), the gate refusing an incomplete chain, and the all-or-nothing batch writer (rollback after a failed move, nothing left after a failed staging) | Source parsing and synthetic images; frame measurements against vanilla files |
| L0 parity | `imagegen-core/tests/kit_parity.rs` | The Rust DDS writer is **byte-identical** to the kit's `write_bgra_dds` and passes the kit's `strict_dds` rule on 13 synthetic images; alpha statistics and the margin and undersized-subject rules agree with `gfx.py`. Skips with a visible `SKIPPED` line if Python, Pillow, numpy or the kit checkout is missing | Binary comparison with the kit's own code |
| L1 provider contract | each adapter's tests | Wire format, parsing, error mapping, retry classes, key redaction, host policy. **Synthetic fixtures**, labelled as such. No recordings of real responses exist yet | Synthetic fixtures (not provider evidence) |
| L1 client | `imagegen-providers/tests/ureq_loopback.rs` | The real HTTP client: bodies, headers, no redirects, size cap, refused connections | Loopback mock server |
| L2 family lock | `imagegen-mcp/tests/families.rs` (10 tests) | An idea artifact cannot be validated, reviewed or installed as another family (no parameter exists); a 65x67 import as focus is refused; installed idea pixels cannot become a decision or focus icon; a provider that echoes another family's art is refused before storage; icon upscale limit; opaque families never keep alpha; validation cannot skip checks and the gate rejects a short check set; every group tool refuses every other group's family at runtime and in its schema; `hoi4.image_families` | Synthetic provider transport |
| L2 pipeline | `imagegen-mcp/tests/pipeline.rs` (23 tests) | Plan to install into a temporary mod: placement, versions, idempotency, cost gate, policy gates (D3, D4), every install-gate refusal, flags in three sizes, imports, register mode, ledger | Synthetic provider transport |
| L2 chains | `imagegen-mcp/tests/chains.rs` (16 tests) | **Every family's processing runs by itself and cannot be skipped**: ten families through plan, generate and process with no steps asked for, each showing its finishing steps, finishers, companions, sizes and a complete recipe record; `steps: []` and every non-whitelisted step refused with nothing written; a processed artifact without its recipe (or fit step, or the right version) fails validation and the install gate, and a raw candidate cannot be installed; achievement installs three DDS under one approval (a changed companion withdraws it, validation names the state); flag installs three bottom-left TGA and validates each size; advisor card refused without the template (at plan and at process) and fail-closed on a template with no opening; report event has transparent corners and passes without an opacity check, finished cards are accepted unchanged; install rolls back when a file cannot be written; strips from two frame files; logos keep room for the glow and leave frame 1 alone; portraits need the face model or a manual crop. Two tests use the kit's real templates and YuNet model and **skip with a printed reason** when the kit checkout is missing (`HOI4_KIT_SKILL` names it) | Synthetic provider transport and synthetic templates; kit files for the two real-file tests |
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

`cargo test --workspace --no-fail-fast` on Windows, 2026-11-01 (debug build, worktree `w5/chains`): **386 passed, 0 failed**. imagegen-core 204 unit + face detect 1 + kit parity 1 + recipe parity 5; imagegen-mcp 5 unit + 16 chains + 10 families + 23 pipeline + 5 plugin gates + 7 protocol; imagegen-providers 69 + 6 loopback; ChatGPT login 27 + recorded replay 7; cli 0. Doc tests 0. The workspace shares one target folder with other sessions: a test binary that another session is running can fail with "being used by another process", and the `protocol` test starts the shared `hoi4-imagegen-mcp` binary, which another session's build can replace (its schema golden then fails); rerun those targets alone.

## Frame 2 against the installed game (HOI4 1.19.4.1)

Measured by `recipes::frames` tests that skip when the game is missing. The vanilla files are uncompressed 32-bit DDS, so the tests decode them directly.

**Selected state** (faction and agency logos). Frame 1 of each vanilla strip is the fitted icon; the test fits the constants (`stroke 2.91, cream (208,198,165), glow dilate 5.5, sigma 4.0, opacity 0.97, dark red (37,5,5)`) against frame 2. Mean alpha of frame 2 at a distance from frame 1's silhouette, ours against vanilla (the first 5 two-frame faction logos, then all 31):

| px | 1 | 2 | 3 | 4 | 5 | 6 | 8 | 10 | 12 |
|---|---|---|---|---|---|---|---|---|---|
| ours, first 5 | 1.000 | 1.000 | 0.841 | 0.628 | 0.522 | 0.422 | 0.244 | 0.111 | 0.045 |
| vanilla, first 5 | 1.000 | 0.973 | 0.846 | 0.697 | 0.552 | 0.449 | 0.273 | 0.133 | 0.055 |
| ours, all 31 | 1.000 | 1.000 | 0.843 | 0.628 | 0.522 | 0.423 | 0.243 | 0.111 | 0.045 |
| vanilla, all 31 | 0.973 | 0.929 | 0.807 | 0.662 | 0.530 | 0.432 | 0.262 | 0.130 | 0.055 |

Rim colour at 1 to 2 px: ours (208, 198, 165), vanilla (224, 213, 176) on the first 5 and (204, 194, 165) on all 31. The test fails when the alpha profile differs by more than 0.10 or a rim channel by more than 30. The 37 vanilla agency strips (234x119) have the same glow profile (alpha 1.00, 0.96, 0.84, 0.67, 0.54, 0.44 at 1 to 6 px on the first of them), measured with a throwaway script, not in a test. Two differences from the maintainer's description: the stroke is about 2.9 px and the glow is dark red (37, 5, 5) and wide (sigma 4), not a light warm glow of sigma 2; and vanilla's frame 2 is also brighter inside (mean ratio 1.05 to 1.55, different art for some logos), which the recipe does not copy (interior unchanged, as decided).

**Disabled state** (buildings). The recipe mixes colour 45 percent toward luminance, so saturation falls to about 0.67 of its value (HSV mean 24.4 to 16.4 on the vanilla `building_icon_strip.dds` frames). Vanilla cannot be matched pixel for pixel: the frame pairs of that strip are different art (correlation 0.07 to 0.47), and their mean saturation is 24.4 for even frames against 21.5 for odd ones over 15 pairs, not the 22 against 13 that was described. The landmark strips that the `building` family actually targets (`historical_buildings/large` and `small`, 128x64 and 64x32) use frame 2 for a ruined state with a fire marker, which no recipe can derive. So frame 2 of `building` is this tool's convention and is not a measured vanilla look (stated in the family's note and tracked as an open question).
