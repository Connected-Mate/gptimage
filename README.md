# hoi4-imagegen-mcp

An [MCP](https://modelcontextprotocol.io) server and command-line tool for making **Hearts of Iron IV graphical assets**: focus icons, idea and national spirit pictures, decision icons, technology icons, event pictures, flags and more. It plans a job and shows its cost, calls a documented image API, fits the result to the exact HOI4 canvas, checks it deterministically, lets a reviewer approve it, and installs a real DDS or TGA file with a sprite snippet and a provenance record.

It is a Rust rewrite of [Connected-Mate/gptimage](https://github.com/Connected-Mate/gptimage) (MIT). See [UPSTREAM.md](UPSTREAM.md) and [NOTICE](NOTICE).

> **Status: phase one, local only.** Nothing here has made a real provider call. Provider code is tested against synthetic fixtures (labelled as such), the HTTP client against a loopback mock server, and the DDS writer for byte parity against the Agentic HOI4 Modding Kit's Python writer. Real-provider recordings and the in-game check are **unavailable** until there are provider keys with balance and a user-operated game session. See [Status and limits](#status-and-limits).

## The loop

```
status -> families -> icon | picture | portrait | flag | unit | interface (plan, or plan + generate with approve)
       -> process -> validate -> preview -> review -> install      edit (refine a candidate) and generate (run a stored plan) are shared
```

| Tool | What it does |
|---|---|
| `hoi4.image_status` | Providers (key present or not, never the value), model tables with their date, game reference, profile source, ledger chain, plugin state |
| `hoi4.image_families` | Every family (or one) with its group and tool, the vanilla-measured canvas and frames (1.19.4.1 counts), accepted sizes, the mod's override, naming, checks, review rubric, final paths and the style sources that exist for it. `calibrate` compares them with the installed game's file headers |
| `hoi4.image_references` | Reference images with role and rights note (manual add and approve); project style profiles pinned by SHA-256. Style references are also chosen automatically, see below |
| `hoi4.image_icon`, `_picture`, `_portrait`, `_flag`, `_unit`, `_interface` | One tool per group of families, each with a **required `family`** whose schema enum lists only that group. Without `approve`: a free plan (prompt, references, provider, model, size, **cost estimate with its basis**, output paths, approvals needed) and a `plan_id`. With `approve: true` and an `idempotency_key`: plan and generate in one call |
| `hoi4.image_generate` | Runs a stored plan: one paid call per variant, stored unchanged as a versioned candidate. `idempotency_key` makes a retry free |
| `hoi4.image_edit` | Refines a candidate with an instruction, an optional mask and extra references |
| `hoi4.image_process` | Runs the family's own processing. The caller may add only the pre-steps `chroma_key`, `crop_to_aspect`, `defringe` and `trim_to_subject`; the family's finishing steps are always appended by the tool, recorded with a recipe id and version, and cannot be replaced, shortened or skipped (an empty step list or any other step is refused). Also imports art made elsewhere, with a required family (strip families may give one file per frame in `import.frames`): another family's size (`GFX-FAMILY-MISMATCH`) and pixels already installed for another family (`GFX-FAMILY-REUSE`) are refused, and icons are never enlarged beyond 1.25 times. Refuses to invent transparency |
| `hoi4.image_validate` | The family's full check set, never fewer (size, alpha, opaque, ghosting, chroma fringe, margins, fill, fake checkerboard, frames, DDS/TGA layout), with stable finding codes. The record carries a check-set hash the install gate compares |
| `hoi4.image_preview` | Contact sheet: native size and enlarged, on dark, light and checker backgrounds, beside the references |
| `hoi4.image_review` | The visual review against the family's rubric. An approval returns the `approval_id` install needs |
| `hoi4.image_install` | Writes the DDS/TGA and a sprite snippet. **Refuses unless validation passed, a review approved this exact file and the hashes are unchanged** |
| `hoi4.image_ledger` | Which references, settings and outputs belong together; lineage; chain check |
| `hoi4.image_cancel` | Cancels a running job, or reconciles one a crashed run left behind |

### Rules the tools enforce

* **Documented APIs, keys from the environment only.** `OPENAI_API_KEY`, `GEMINI_API_KEY` (or `GOOGLE_API_KEY`), `FAL_KEY`, and optionally `COMFYUI_API_KEY`. A project file can choose models and endpoints but cannot send a key to a new host.
* **Processing cannot be skipped.** Each family has a recipe (table below). Validation and the install gate read the processed record back and refuse an artifact whose recipe or required steps are missing.
* **Transparency is proven from pixels**, never from the prompt. An opaque result cannot become a transparent icon, and a drawn checkerboard is rejected.
* **Nothing is overwritten.** Candidates are `name-v1`, `name-v2`...; processed art is `-p1`, `-p2`...; an installed file is replaced only if this tool installed it and it is unchanged since.
* **Everything stays inside the mod project.** No `..`, drive, UNC, stream, device-name, short-name or symlink escapes.
* **Deterministic checks are not a style verdict.** Composition, style and readability at game size go to the review, recorded separately.
* **Model ids retire quickly** (three in eight months). Ids, limits and prices live in dated tables under `crates/imagegen-providers/data/`; a retired model is refused before any request.

### Processing chains

A chain is the caller's optional pre-steps (`chroma_key`, `crop_to_aspect`, `defringe`, `trim_to_subject`) followed by the family's mandatory finishing. Imported art that already has its final size is accepted as it is, which the chain records as `conform`. Counters and the division emblem stay import-only (their second frame is different art); every other family is generated end to end.

| Recipe | Families | Finishing (always runs) | Writes |
|---|---|---|---|
| `icon` | transparent icons, technology, modifier icon, GUI texture | trim, fit to the canvas with the profile margin, grayscale for special projects | one DDS |
| `picture` | decision category and operation pictures, news, super event | flatten on black, cover-resize, grayscale for news | one DDS |
| `report_card` v2 | `report_event` | flatten, then the report card (sepia, grain, tilt, soft shadow; seed from the candidate hash): 210x176 with transparent corners | one DDS with card alpha |
| `selected_strip` | `faction_logo`, `agency_logo` | fit with a 10 pixel margin, then frame 2 as the selected state (cream stroke, dark-red glow, fitted to vanilla) | one two-frame DDS |
| `disabled_strip` | `building` | fit, then frame 2 as the disabled state (colour mixed 45 percent toward grey) | one two-frame DDS |
| `imported_art` | `counter_large`, `counter_small`, `division_emblem` | the exact strip (or `import.frames`) is accepted | one DDS |
| `portrait` | the four portraits | flatten, then the face-detected head-and-shoulders crop to 156x210 (YuNet from the project, or `portrait_crop`) | one opaque DDS |
| `advisor_card` | `advisor_card` | flatten, then the portrait is composited into the mod's advisor template with the alpha-coverage audit (fails closed) | one 65x67 DDS |
| `achievement` | `achievement` | fit the subject to 64x64, then the grey state and the not-eligible state (grey plus the red cross) on the mod's template backgrounds | three DDS: `<key>`, `<key>_grey`, `<key>_not_eligible` |
| `flag` | `flag` | flatten, then the 82x52, 41x26 and 10x7 ladder | three TGA, bottom-left origin |

The templates and the face model come from the mod's own copy of the asset skill, found inside the project: `.claude/skills/*/assets/*/portraits/advisors/advisor_template.png`, `.../icons/achievements/template/` and `.claude/skills/*/tools/models/face_detection_yunet_2026may.onnx`. `[templates] dir` and `face_model` in `.hoi4-imagegen/config.toml` override the search. A missing template is a refusal that names the paths it looked for. Install writes every file of an asset (the companions, the `.gfx` text or the handoff note) in one batch and undoes it all if any step fails.

### Maintainer decisions built in

| | |
|---|---|
| D3 | A ComfyUI endpoint you configured may be used for any asset category, including user-run ones. It is a plain HTTP client; it never starts or stops pods |
| D4 (amended 2026-11-01) | Paradox art (the installed game, a vanilla shelf) may always be sent to a provider as a reference when it helps match the style. There is no switch: `policy.allow_vanilla_references` is still read but ignored, and `hoi4.image_status` says so. The origin of every reference is recorded, and the tool never copies vanilla files into a mod |
| D-12 | The ChatGPT-login route is a separate crate, off by default, local only, personal use. See below |

## Install and run

```sh
cargo build --release          # produces target/release/hoi4-imagegen-mcp and hoi4-imagegen
export OPENAI_API_KEY=...      # or GEMINI_API_KEY, FAL_KEY; keys are never stored in files
hoi4-imagegen --project-root path/to/your/mod doctor
```

MCP client entry (no secrets in it):

```json
{ "mcpServers": { "hoi4_imagegen": { "command": "hoi4-imagegen-mcp", "args": ["--project-root", "."] } } }
```

Standard output carries only protocol messages. Project state lives in `.hoi4-imagegen/` (config, styles, plans, ledger, records); working files in `docs/assets/<feature>/` as the kit's asset skill describes.

### Project config (`.hoi4-imagegen/config.toml`)

```toml
[defaults]
provider = "openai"

[policy]
forbidden_families = []              # for example ["leader_portrait"]
max_usd_per_call = 1.0

[providers.openai]
quality = "low"                      # the cheapest tier; raise it when review says so

[providers.comfyui]                  # D3
endpoint = "http://127.0.0.1:8188"
workflow = ".hoi4-imagegen/comfyui/text_to_image.json"

[profiles.focus]                     # per-project canvas override (a family id)
canvas = [94, 86]
```

### Families

There are 40 families in six groups. The family alone selects canvas, frames, naming, references, processing and validation, and an asset keeps its family from the plan to the install record: `process`, `validate`, `review` and `install` use the artifact's own family and take no family argument for it.

| Tool | Families |
|---|---|
| `hoi4.image_icon` | focus, idea_spirit, idea_concern, officer_corps_spirit, decision, decision_category_icon, state_modifier, special_project_icon, raid, mio_department, mio_trait, mio_policy, bop_side, medal, agency_upgrade, operation_icon, commander_trait, personal_trait, doctrine |
| `hoi4.image_picture` | report_event, news_event, decision_category_picture, operation_picture, technology, achievement |
| `hoi4.image_portrait` | leader_portrait, commander_portrait, operative_portrait, scientist_portrait, advisor_card |
| `hoi4.image_flag` | flag |
| `hoi4.image_unit` | counter_large, counter_small, division_emblem, building, faction_logo, agency_logo |
| `hoi4.image_interface` | modifier_stat_icon, gui_texture, super_event (the mod defines the canvas of the last two) |

### Automatic style references

A plan picks style references for the family on its own, **mod first** (maintainer decision of 2026-11-01), and reports each with its tier, reason, SHA-256 and whether it goes to the provider (`plan.style_sources`, also `hoi4.image_families { family }`):

| Tier | Source | Example reason |
|---|---|---|
| 1 | The project's pinned style profile for the family (`.hoi4-imagegen/styles/`) | `pinned style 'mine': reference 1 of 3` |
| 2 | The mod's copy of the asset skill's `assets/` shelf, `.claude/skills/*/assets/*/<shelf folder>/` (the kit's root is `vanilla_reference`, Chaos Redux's `gfx_references`). It holds vanilla art but is the mod's own copy | `mod asset folder copy: medoid 2 of 16 focus images (cluster of 6)` |
| 3 | The mod's own installed assets: the family's texture folders plus the textures its `.gfx` files register under the family's sprite name pattern | `mod asset: medoid 3 of 214 idea_spirit images (cluster of 9)` |
| 4 | Vanilla art from the installed game | `vanilla: medoid 1 of 742 report_event images (256 sampled evenly)` |

The first tier with at least 3 usable images is used. Files of the wrong size for the family (idea pictures share a folder with advisor and concern art) are dropped by header. Vanilla may always be sent to a provider; the tool never copies it into the mod, and the contact sheet leaves it out.

Selection is deterministic. Each candidate is measured: CIE Lab mean and spread, mean saturation, Sobel edge density, alpha coverage, dark-outline share on the alpha edge and a 6-bin hue histogram (16 numbers). They are standardised, candidates are ordered by content hash, and PAM k-medoids (greedy build, best-improvement swap, no random start) picks up to 4 images to send (the model's reference limit caps it) and up to 24 medoids for the statistics. At most 256 candidates are measured per tier (an even sample by path beyond that). The same image set gives the same choice in the same order, whatever the folder names are.

The statistics (palette, mean and spread of saturation and value, edge density, outline presence, alpha coverage, margin, subject span) become a short paragraph in the prompt, for example `Match vanilla Hearts of Iron IV national focus icon art: moderately muted, dark palette dominated by black, dark grey and brown; partial dark outline; ...`, after the style profile's own note when one applies.

**Style check.** `style` is part of every family's check set (so the check-set hash and the install gate treat it like the others). It measures the candidate the same way and warns with `GFX-STYLE-DRIFT` when its RMS z-score against the family's style distribution exceeds a threshold. It is a warning, never an error, and never blocks an install. Within the distribution it records the advisory `GFX-STYLE-MATCH`, so the stored validation carries the numbers either way. The threshold is the 95th percentile of the leave-one-out RMS z-scores of the distribution's own images, never below 1.75. Measured on the installed game (1.19.4.1), that percentile ran from 0.60 to 2.15 for 37 of the 38 families with vanilla art (median 1.53, large families 1.0 to 1.7; one 15-image family reached 6.7). `docs/testing.md` has the table.

Canvas conventions come from the kit's `.tools/gfx/profiles.json` when it exists, over built-in defaults measured from HOI4 1.19.4.1 (`crates/imagegen-core/data/vanilla-measurement.json`, produced by `scripts/measure_families.py`; a unit test fails when a family stops matching it). Examples: focus 100x88, idea_spirit 60x68, advisor_card 65x67, decision 33x32, report_event 210x176, news_event 397x153, flags 82x52 / 41x26 / 10x7. Every value is overridable and `hoi4-imagegen calibrate` shows where one is not the dominant vanilla size. Style cannot be measured and stays a review matter.

## Layout

```
crates/imagegen-core        families, profiles, the family lock, processing, validation, DDS/TGA, naming, ledger. No network
crates/imagegen-providers   one trait; OpenAI, Gemini, fal, ComfyUI; destination-checked HTTP
crates/imagegen-mcp         the operations and the rmcp stdio server (binary hoi4-imagegen-mcp)
crates/imagegen-cli         hoi4-imagegen: doctor, calibrate, plan, generate, validate, install, call
crates/provider-chatgpt-login   OPT-IN plugin, not a default member, see below
skill/hoi4-imagegen         the agent skill (under 8 KB)
docs/                       provider notes, the plugin's terms and risks, test layers
```

## The opt-in ChatGPT login

The upstream project's route signs in with a ChatGPT account and calls an **undocumented** backend while presenting itself as the Codex CLI. It can break or be blocked without notice, may not be covered by OpenAI's terms, and heavy use can risk account restrictions. It is therefore **not** in default builds, **not** a default workspace member, and never part of hosted HOI4 Factory. It needs all of: a source build with `--features unofficial-chatgpt-login`, `[providers.chatgpt_login] enabled = true` in the project config, `HOI4_IMAGEGEN_ACCEPT_UNOFFICIAL=1` in your own environment, and local stdio or CLI mode. A test proves a default build contains none of it. Read [docs/chatgpt-login.md](docs/chatgpt-login.md) first.

## Status and limits

* Verified by running tests: see the test layers in [docs/testing.md](docs/testing.md).
* **Unavailable, with the reason:** recordings of real provider responses (no API key with balance is known to exist), end-to-end runs with real providers, the in-game check including whether a focus without a `_shine` sprite logs an error (needs a user-operated game session), and macOS and Linux runs (this phase ran on Windows only).
* The Gemini response envelope is read from the documented content schema; the docs show no raw image response. The OpenAI 2.5 per-image price is unpublished, so estimates use the `gpt-image-2` row as a labelled proxy until a recorded call reports usage.
* Rights: Paradox's and Steam's rules on AI-generated mod assets and the providers' output terms were not researched. Nothing here promises anything about them.
* `register` mode edits `.gfx` files as text (comments and layout are kept). A parser-based version waits for the shared Rust HOI4 engine.

## License

MIT. See [LICENSE](LICENSE), [NOTICE](NOTICE) and [UPSTREAM.md](UPSTREAM.md).
