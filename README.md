# hoi4-imagegen-mcp

`hoi4-imagegen-mcp` makes Hearts of Iron IV image assets. It has two programs:

* `hoi4-imagegen-mcp`, a Rust [MCP](https://modelcontextprotocol.io) stdio server for AI clients such as Claude Code and Codex.
* `hoi4-imagegen`, a command-line tool that runs the same operations.

It covers 40 asset families: focus icons, idea pictures, decision icons, technology pictures, event pictures, portraits, flags, counters, logos and more. For each asset it plans the job and shows the cost, calls an image provider, fits the result to the exact HOI4 canvas, runs the family's own processing, checks the file, lets a reviewer approve it and installs a real DDS or TGA file with a sprite snippet and a provenance record.

It is a Rust rewrite of [Connected-Mate/gptimage](https://github.com/Connected-Mate/gptimage) (MIT). See [UPSTREAM.md](UPSTREAM.md) and [NOTICE](NOTICE).

Quick links: [how-to guide](docs/how-to.md), [ChatGPT login](docs/chatgpt-login.md), [rules and terms](docs/rules-and-terms.md), [kit integration](docs/kit-integration.md), [test layers](docs/testing.md), [recipes](examples/recipes.md).

## What it does

```
status -> families -> icon | picture | portrait | flag | unit | interface   (free plan, or plan + generate with approve)
       -> process -> validate -> preview -> review -> install
```

* Every asset has a **family**. The family alone selects the canvas, frames, naming, references, processing and checks. An asset keeps its family from the plan to the install record.
* The canvas sizes are measured from the installed game (HOI4 1.19.4.1), not guessed.
* Processing cannot be skipped. Each family has a fixed chain that the tool appends itself.
* Install is gated: validation passed, a review approved this exact file, and nothing changed since.
* Nothing leaves the mod project and nothing is overwritten.

## Tools

| Tool | What it does |
|---|---|
| `hoi4.image_status` | Providers (key present or not, never the value), model tables with their date, game folder, profile source, ledger chain, ChatGPT-login state |
| `hoi4.image_families` | The catalogue: every family (or one) with its group and tool, the vanilla-measured canvas and frames, accepted sizes, the mod's override, naming, checks, review rubric, final paths and the style sources that exist for it. `calibrate: true` compares the table with the installed game's file headers |
| `hoi4.image_references` | Reference images with role and rights note, and project style profiles pinned by SHA-256 |
| `hoi4.image_icon`, `hoi4.image_picture`, `hoi4.image_portrait`, `hoi4.image_flag`, `hoi4.image_unit`, `hoi4.image_interface` | One tool per group of families. Each has a **required `family`** whose schema enum lists only that group's families. Without `approve`: a free plan (prompt, references, provider, model, size, cost estimate with its basis, output paths) and a `plan_id`. With `approve: true` and an `idempotency_key`: plan and generate in one call |
| `hoi4.image_generate` | Runs a stored plan. One provider call per variant. The same `idempotency_key` replays instead of paying again |
| `hoi4.image_edit` | Refines a candidate with an instruction, an optional mask and extra references. Also makes the ruined second frame of a building |
| `hoi4.image_process` | Runs the family's processing chain. Also imports art made elsewhere |
| `hoi4.image_validate` | The family's full check set, with stable finding codes |
| `hoi4.image_preview` | A contact sheet: native size and enlarged, on dark, light and checker backgrounds, beside the references |
| `hoi4.image_review` | The visual review against the family's rubric. An approval returns the `approval_id` that install needs |
| `hoi4.image_install` | Writes the DDS or TGA files and the sprite. Refuses unless the gate is open |
| `hoi4.image_ledger` | Which references, settings and outputs belong together, lineage, and the chain check |
| `hoi4.image_cancel` | Cancels a running job, or reconciles one that a crashed run left behind |

| `hoi4.image_audit` | Audits the whole mod on the shared engine (hoi4-agent-tools): which families use each sprite and texture, cross-family reuse (`GFX-FAMILY-REUSE`, a warning, with a per-mod allowlist), missing textures, undefined sprites, missing focus shine (an error: the game then draws a `?`), wrong canvas sizes and report cards without transparent corners. Read only |

That is 19 tools.

## Families

There are 40 families in six groups. The group names the tool that plans them.

| Tool | Families |
|---|---|
| `hoi4.image_icon` | focus, idea_spirit, idea_concern, officer_corps_spirit, decision, decision_category_icon, state_modifier, special_project_icon, raid, mio_department, mio_trait, mio_policy, bop_side, medal, agency_upgrade, operation_icon, commander_trait, personal_trait, doctrine |
| `hoi4.image_picture` | report_event, news_event, decision_category_picture, operation_picture, technology, achievement |
| `hoi4.image_portrait` | leader_portrait, commander_portrait, operative_portrait, scientist_portrait, advisor_card |
| `hoi4.image_flag` | flag |
| `hoi4.image_unit` | counter_large, counter_small, division_emblem, building, faction_logo, agency_logo |
| `hoi4.image_interface` | modifier_stat_icon, gui_texture, super_event |

### Measured from the game

The built-in canvas, frames, accepted sizes and pixel class of every family come from the installed game, version 1.19.4.1. The data is in `crates/imagegen-core/data/vanilla-measurement.json`, made by `scripts/measure_families.py`. A unit test fails when a family stops matching it. Examples: focus 100x88, idea_spirit 60x68, advisor_card 65x67, decision 33x32, report_event 210x176, news_event 397x153, flags 82x52, 41x26 and 10x7.

A mod can override the canvas and naming in its own `.tools/gfx/profiles.json` (the kit's file) or in `.hoi4-imagegen/config.toml`. Without an override the vanilla size is used. `hoi4.image_families` shows the override next to the vanilla value, and `hoi4-imagegen calibrate` shows where a built-in value is not the dominant vanilla size. `gui_texture` and `super_event` have no vanilla texture to measure, so the mod defines their canvas.

### The family lock

A family never changes after the plan. `process`, `validate`, `review` and `install` use the artifact's own family and take no family argument for it.

* An image whose size belongs to another family is refused: `GFX-FAMILY-MISMATCH`.
* An image whose pixels are already installed for another family is refused: `GFX-FAMILY-REUSE`.
* Each group tool refuses a family of another group, in its schema and at run time.
* Icons are never enlarged by more than 1.25 times.

## Processing chains

A chain is the caller's optional pre-steps followed by the family's mandatory finishing. The caller may add only `chroma_key`, `crop_to_aspect`, `defringe` and `trim_to_subject`, through `pre_steps` on `hoi4.image_process`. Any other step, or an empty list, is refused. The finishing is appended by the tool, recorded with a recipe id and version, and read back by validation and the install gate.

| Recipe | Families | Finishing (always runs) | Writes |
|---|---|---|---|
| `icon` | transparent icons, technology, modifier_stat_icon, gui_texture | trim, fit to the canvas with the profile margin, grayscale where the family needs it. A focus icon also gets its `_shine` sprite entry at install | one DDS |
| `picture` | decision and operation pictures, news, super_event | flatten on black, cover-resize, grayscale for news | one DDS |
| `report_card` v2 | `report_event` | flatten, then the report card: sepia, grain, tilt and a soft shadow fitted to vanilla, 210x176 with transparent corners | one DDS |
| `achievement` | `achievement` | fit the subject, then the grey state and the not-eligible state on the mod's template backgrounds | three DDS: `<key>`, `<key>_grey`, `<key>_not_eligible` |
| `advisor_card` | `advisor_card` | flatten, then the portrait is composited into the mod's advisor template, with an alpha-coverage audit that fails closed | one 65x67 DDS |
| `portrait` | the four portraits | flatten, then a head-and-shoulders crop to 156x210 found with YuNet face detection (pure-Rust `tract-onnx`, model from the mod's skill copy). `portrait_crop` sets the crop by hand | one opaque DDS |
| `flag` | `flag` | flatten, then the 82x52, 41x26 and 10x7 ladder | three TGA, bottom-left origin |
| `selected_strip` | `faction_logo`, `agency_logo` | fit with a margin, then frame 2 as the selected state. A faction logo also gets its 32x32 `_miniature` | one two-frame DDS, plus the miniature for a faction logo |
| `landmark_strip` | `building` | fit, then frame 2 as the ruined landmark, made from a second candidate of the same asset (`hoi4.image_edit` on the first, passed as `second_frame_candidate`). A 64x64 landmark also gets the 32x32 `_small` strip | one or two two-frame DDS |
| `imported_art` | `counter_large`, `counter_small`, `division_emblem` | import only: the exact strip, or one file per frame in `import.frames`. The on-map counter also gets its text icon, the division emblem its 30x12 small symbol | one DDS, or two |

A medal is made as one 56x84 frame, registered as `GFX_medal_icon_<asset>_large`, and its 28x42 `GFX_medal_icon_<asset>` is derived from it.

### Names the game looks up

Every sprite name follows Hearts of Iron IV 1.19.4.1, and `crates/imagegen-core/src/lookup.rs` records the vanilla evidence for each family. Where script names the sprite in full (a focus `icon = GFX_focus_<asset>`), the name is free and the install result gives the exact text to write. Where the game builds the name, the tool builds the same one, and the install result's `script` field says what to write:

| Family | The game looks up | Script |
|---|---|---|
| `idea_spirit`, `idea_concern` | `GFX_idea_<key>` | `picture = <asset>` |
| `officer_corps_spirit` | `GFX_idea_<idea key>` | name the spirit idea `<asset>` |
| `decision`, `decision_category_icon`, `decision_category_picture` | `GFX_decision_<key>`, `GFX_decision_category_<key>`, `GFX_decision_cat_picture_<key>` | `icon = <asset>` or `picture = <asset>` |
| `raid` | `GFX_raid_type_icon_<raid type>` | name the raid type `<asset>` |
| `commander_trait`, `personal_trait` | `GFX_trait_<trait key>` | name the trait `<asset>` |
| `technology` | `GFX_<technology key>_medium` | name the technology `<asset>` |
| `counter_large` | `GFX_unit_<sub-unit key>_icon_medium`, two frames | name the sub-unit `<asset>` (its key, not its `sprite`) |
| `counter_small` | `GFX_unit_<sub-unit key>_icon_medium_white` and the text icon `GFX_unit_<sub-unit key>_icon_small`, two frames each | name the sub-unit `<asset>` |
| `division_emblem` | `GFX_div_templ_<n>_large` and `GFX_div_templ_<n>_small` | none: the player picks symbol `<n>` |
| `faction_logo` | the logo and `<logo>_miniature` | `icon = GFX_faction_logo_<asset>` |
| `medal` | `GFX_medal_icon_<x>` and `GFX_medal_icon_<x>_large` | `icon = GFX_medal_icon_<asset>` with `frame = 1` |
| `building` | `GFX_<x>_icon_small` (province window) and `GFX_<x>_icon` (raid window) | `special_icon = GFX_<asset>_icon_small` |
| `modifier_stat_icon` | the name a `.gui` gives, upper case like vanilla | `spriteType = "GFX_MODIFIER_<ASSET>"` |

A division emblem's number `<n>` is one past the highest of vanilla's numbers (0 to 43 and 65 to 121) and every number the mod's `interface/*.gfx` files or an earlier install already use. Install records it in the asset record. Achievements and flags have no sprite: the game loads them by file name.

Art that already has its final accepted size is taken as it is when imported, which the chain records as `conform`. Counters and division emblems are not generated, because their second frame is different art. Every other family is generated end to end.

The advisor card, achievement and portrait chains need files from the mod's own copy of the asset skill, found inside the project:

* `.claude/skills/*/assets/*/portraits/advisors/advisor_template.png`
* `.claude/skills/*/assets/*/icons/achievements/template/`
* `.claude/skills/*/tools/models/face_detection_yunet_2026may.onnx`

`[templates] dir` and `face_model` in `.hoi4-imagegen/config.toml` override the search. A missing file is refused at plan time, before anything is spent, and the message lists the paths that were searched.

## Style references

A plan picks style references for the family on its own. The mod comes first and vanilla second. The tiers, in order:

| Tier | Source |
|---|---|
| 1 | The project's pinned style profile for the family (`.hoi4-imagegen/styles/`) |
| 2 | The mod's own copy of the asset skill's shelf, `.claude/skills/*/assets/*/<shelf folder>/`. It holds vanilla art but is the mod's copy |
| 3 | The mod's own installed assets: the family's texture folders, plus textures its `.gfx` files register under the family's sprite pattern |
| 4 | Vanilla art from the installed game |

The first tier with at least 3 usable images is used. A pinned style is always sent, even with fewer images, but then gives no statistics. The plan reports every reference with its tier, reason, SHA-256 and whether it goes to the provider (`style_sources` in the plan, also in `hoi4.image_families`).

Selection is deterministic. Each candidate is measured (colour, saturation, edge density, alpha, outline share, hue histogram), and a fixed k-medoids picks up to 4 images to send, within the model's reference limit. The same images give the same choice. The measured statistics also become a short style paragraph in the prompt.

`style` is part of every family's check set. It warns with `GFX-STYLE-DRIFT` when a candidate is far from the family's style distribution. It is a warning, never an error, and never blocks an install.

Vanilla references may always be sent to a provider (a maintainer decision of 2026-11-01), and the tool never copies a vanilla file into a mod. Vanilla art is also left out of the contact sheet.

## Validation and install

Install is the last step and the strictest.

1. `hoi4.image_validate` runs the family's full check set, never fewer, with stable finding codes (size, alpha, margins, fill, ghosting, chroma fringe, fake checkerboard, frames, DDS or TGA layout, style). The record carries a hash of the check set.
2. The gate compares that hash and audits the processing chain against the family's recipe. A record with a missing step is refused.
3. `hoi4.image_preview` makes the contact sheet. `hoi4.image_review` answers the family's rubric. Agents may approve. The reviewer's name is recorded with the review.
4. `hoi4.image_install` needs the `approval_id` of an approval of this exact file, and refuses if any hash changed since.
5. The write is atomic: every file of an asset (companions, `.gfx` text or the handoff note) is written together and all are undone if one fails.

`mode: handoff` (default) writes the sprite snippet into `docs/assets/<feature>/gfx_handoff.md` for the main agent to wire. `mode: register` inserts the entries into the suggested `.gfx` file as text, keeping comments and layout. Families without a sprite (achievements and flags) are loaded by the game from the file name, so both modes install the files alone. A final file is replaced only if this tool installed it and it is unchanged (`on_exists: replace`).

Deterministic checks are not a style verdict. Composition and readability at game size go to the review. In-game appearance can only be checked by running the game, which this tool never does. It says "installed", not "verified in game".

## Providers

| Provider | Needs |
|---|---|
| ChatGPT login | `hoi4-imagegen login` once. Uses your own ChatGPT plan. No API key |
| OpenAI API | `OPENAI_API_KEY` |
| Google Gemini API | `GEMINI_API_KEY` or `GOOGLE_API_KEY` |
| fal | `FAL_KEY` |
| ComfyUI | an endpoint in the project config, and optionally `COMFYUI_API_KEY` |

When a plan names no provider, the first ready one in this order is used: ChatGPT login, OpenAI, Gemini, fal, ComfyUI. A `provider` in the plan, a style profile or `[defaults]` in the project config wins.

Keys come from the environment first. If a variable is not set, the tool reads the key HOI4 Mod Setup saved in the operating system credential store. A project file can choose models and endpoints but cannot send a key to a new host. Model ids retire quickly, so ids, limits and prices live in dated tables under `crates/imagegen-providers/data/`, and a retired model is refused before any request.

### The ChatGPT login

Once you have signed in, the ChatGPT login is the default provider on your machine. It ships in every build and needs no feature flag, config entry or environment variable. It is local only and is never registered in a hosted deployment.

What to know before you use it:

* It calls a ChatGPT backend that OpenAI does not document for other tools, and it presents itself as the Codex CLI. It can change or be blocked without notice. Heavy use counts against your plan, and the consequences are yours. The risk note is shown once at sign-in.
* Every image runs on `gpt-6-luna` at high reasoning effort. The tool sends both and checks that the backend's response reports both. A response that reports something else, or nothing, is refused and the image is not used. There is no fallback and no override.
* **The route cannot force image quality.** The pixels come from the backend's own image tool, reported as `gpt-image-2-codex`. The backend replaces the tool settings we send with `quality = auto` and `size = auto`, and chooses quality and size itself. One recorded probe returned a medium-quality 1337x1176 image. The ledger records what the backend reported for each image, so check the pixels in the preview.
* A refusal by the provider's safety system is reported as a moderation block, with advice to change the brief or the references.
* Spend is recorded as unknown, because there is no per-image price and no usage report.

Details, the sign-in steps and the recorded evidence are in [docs/chatgpt-login.md](docs/chatgpt-login.md).

## Install

The releases are on GitHub: <https://github.com/klimPaskov/hoi4-imagegen-mcp/releases>. Version 0.2.1 has one archive per platform:

| Platform | Archive |
|---|---|
| Windows x64 | `hoi4-imagegen-v0.2.1-x86_64-pc-windows-msvc.zip` |
| macOS arm64 | `hoi4-imagegen-v0.2.1-aarch64-apple-darwin.tar.gz` |
| macOS x64 | `hoi4-imagegen-v0.2.1-x86_64-apple-darwin.tar.gz` |
| Linux x64 | `hoi4-imagegen-v0.2.1-x86_64-unknown-linux-gnu.tar.gz` |
| Linux arm64 | `hoi4-imagegen-v0.2.1-aarch64-unknown-linux-gnu.tar.gz` |

Each archive holds both programs, `LICENSE`, `NOTICE`, `README.md` and `UPSTREAM.md`. The release also has a `SHA256SUMS` file. The binaries are **unsigned**, so Windows SmartScreen and macOS Gatekeeper may warn. The archives and `SHA256SUMS` carry GitHub build attestations. To check a download:

```sh
sha256sum -c SHA256SUMS --ignore-missing
gh attestation verify <archive> --repo klimPaskov/hoi4-imagegen-mcp
```

### With HOI4 Mod Setup

[HOI4 Mod Setup](https://github.com/klimPaskov/HOI4-Mod-Setup) installs the release per project and writes the project-level MCP entries for your coding tools, under the server id `hoi4_imagegen`. It also offers the sign-in and keeps API keys in the credential store.

### By hand

Unpack the archive somewhere on your `PATH` (or note its full path), then check the setup:

```sh
hoi4-imagegen --project-root path/to/your/mod doctor
```

Add the server to the project's MCP config. Claude Code, `.mcp.json` in the mod folder:

```json
{
  "mcpServers": {
    "hoi4_imagegen": {
      "command": "hoi4-imagegen-mcp",
      "args": ["--project-root", "."]
    }
  }
}
```

Codex, `.codex/config.toml` in the mod folder:

```toml
[mcp_servers.hoi4_imagegen]
command = "hoi4-imagegen-mcp"
args = ["--project-root", "."]
```

Use the full path in `command` if the binary is not on `PATH`. Put no secret in these files. Standard output carries only protocol messages, and anything meant for a person goes to standard error.

To build from source, use Rust 1.88 or newer and run `cargo build --release`. The binaries are in `target/release/`.

### Project config (`.hoi4-imagegen/config.toml`)

```toml
[defaults]
provider = "openai"                  # optional; without it the provider order above applies

[policy]
forbidden_families = []              # for example ["leader_portrait"]
max_usd_per_call = 1.0

[providers.openai]
quality = "low"                      # the cheapest tier; raise it when review says so

[providers.comfyui]
endpoint = "http://127.0.0.1:8188"
workflow = ".hoi4-imagegen/comfyui/text_to_image.json"

[profiles.focus]                     # per-project canvas override, keyed by family id
canvas = [94, 86]
```

Project state lives in `.hoi4-imagegen/`: config, styles, plans, the ledger (`ledger.jsonl`) and records. Working files go in `docs/assets/<feature>/`, as the kit's asset skill describes.

## Rules the tools enforce

* **Documented APIs and keys outside the project.** No key is ever written into a project file.
* **Transparency is proven from pixels**, never from the prompt. An opaque result cannot become a transparent icon, and a drawn checkerboard is rejected.
* **Nothing is overwritten.** Candidates are `name-v1`, `name-v2`. Processed art is `-p1`, `-p2`. An installed file is replaced only if this tool installed it and it is unchanged.
* **Everything stays inside the mod project.** No `..`, drive, UNC, stream, device-name, short-name or symlink escapes.

## Rules and rights

Read [docs/rules-and-terms.md](docs/rules-and-terms.md). It summarises what Paradox, Steam and the image providers say about AI-generated mod assets, and which parts could not be verified. The short version:

* No ban on AI art in HOI4 mods was found, but the general rules apply: keep the mod free, do not infringe, and upload only what you have the right to upload.
* **Disclosing** that a mod uses AI-generated art is recommended. The ledger records provider, model, date and prompt for every asset, and `hoi4.image_ledger` can list them for your mod description.
* **Real people.** Portraits of real historical figures raise personal-rights questions, and the Steam warranty covers them. Providers may refuse such images. Prefer stylised or fictional portraits. The portrait tools warn about this, and a project can forbid a category with `forbidden_families`.
* Assets must be new works in the vanilla style, not near-copies of Paradox art.

## Status and limits

* Verified by tests on Windows, macOS and Linux in CI: see [docs/testing.md](docs/testing.md). The test layers are labelled by evidence kind.
* One signed-in ChatGPT-login probe was recorded on 2026-11-01. Its response is a recorded-real fixture, and faults are injected into it for tests. See [docs/evidence/](docs/evidence/).
* **Not verified:** the in-game check, including whether a focus icon without a `_shine` sprite logs an error, because it needs a user-operated game session. Runs with real OpenAI, Gemini, fal and ComfyUI keys are also unavailable, because no key with balance was available.
* The OpenAI 2.5 per-image price is unpublished, so its estimate uses the `gpt-image-2` row as a labelled proxy.
* `register` mode edits `.gfx` files as text. A parser-based version waits for a shared Rust HOI4 engine.
* The binaries are unsigned.

## Layout

```
crates/imagegen-core            families, profiles, the family lock, chains, processing, validation, DDS and TGA, naming, ledger. No network
crates/imagegen-providers       one trait; OpenAI, Gemini, fal, ComfyUI; destination-checked HTTP
crates/provider-chatgpt-login   the ChatGPT-login provider (OAuth, credential store, backend client)
crates/imagegen-mcp             the operations and the rmcp stdio server (binary hoi4-imagegen-mcp)
crates/imagegen-cli             hoi4-imagegen: doctor, calibrate, families, plan, generate, validate, install, call, login, login-status, logout
skill/hoi4-imagegen             the agent skill
docs/                           how-to, provider notes, rules and terms, kit integration, test layers
```

## License

MIT. See [LICENSE](LICENSE), [NOTICE](NOTICE) and [UPSTREAM.md](UPSTREAM.md).
