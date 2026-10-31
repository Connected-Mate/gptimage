# hoi4-imagegen-mcp

An [MCP](https://modelcontextprotocol.io) server and command-line tool for making **Hearts of Iron IV graphical assets**: focus icons, idea and national spirit pictures, decision icons, technology icons, event pictures, flags and more. It plans a job and shows its cost, calls a documented image API, fits the result to the exact HOI4 canvas, checks it deterministically, lets a reviewer approve it, and installs a real DDS or TGA file with a sprite snippet and a provenance record.

It is a Rust rewrite of [Connected-Mate/gptimage](https://github.com/Connected-Mate/gptimage) (MIT). See [UPSTREAM.md](UPSTREAM.md) and [NOTICE](NOTICE).

> **Status: phase one, local only.** Nothing here has made a real provider call. Provider code is tested against synthetic fixtures (labelled as such), the HTTP client against a loopback mock server, and the DDS writer for byte parity against the Agentic HOI4 Modding Kit's Python writer. Real-provider recordings and the in-game check are **unavailable** until there are provider keys with balance and a user-operated game session. See [Status and limits](#status-and-limits).

## The loop

```
status -> profiles -> plan -> generate -> process -> validate -> preview -> review -> install
                                   \-> edit (refine a candidate) --/            \-> ledger
```

| Tool | What it does |
|---|---|
| `hoi4.image_status` | Providers (key present or not, never the value), model tables with their date, game reference, profile source, ledger chain, plugin state |
| `hoi4.image_profiles` | Category profiles with evidence, canvas, final paths and review rubric. `calibrate` compares them with the installed game's file headers and proposes a diff |
| `hoi4.image_references` | Reference images with role and rights note; project style profiles pinned by SHA-256 |
| `hoi4.image_plan` | Free dry run: profile, prompt, references, provider, model, size, **cost estimate with its basis**, output paths, approvals needed. Returns a `plan_id` |
| `hoi4.image_generate` | Runs a plan: one paid call per variant, stored unchanged as a versioned candidate. `idempotency_key` makes a retry free |
| `hoi4.image_edit` | Refines a candidate with an instruction, an optional mask and extra references |
| `hoi4.image_process` | Deterministic chain: trim, fit with margin, crop, Lanczos resize, defringe, grayscale. Also imports art made elsewhere. Refuses to invent transparency |
| `hoi4.image_validate` | Size, alpha, opaque canvas, ghosting, chroma fringe, margins, fill, fake checkerboard, frames, DDS/TGA layout, with stable finding codes |
| `hoi4.image_preview` | Contact sheet: native size and enlarged, on dark, light and checker backgrounds, beside the references |
| `hoi4.image_review` | The visual review against the profile's rubric. An approval returns the `approval_id` install needs |
| `hoi4.image_install` | Writes the DDS/TGA and a sprite snippet. **Refuses unless validation passed, a review approved this exact file and the hashes are unchanged** |
| `hoi4.image_ledger` | Which references, settings and outputs belong together; lineage; chain check |
| `hoi4.image_cancel` | Cancels a running job, or reconciles one a crashed run left behind |

### Rules the tools enforce

* **Documented APIs, keys from the environment only.** `OPENAI_API_KEY`, `GEMINI_API_KEY` (or `GOOGLE_API_KEY`), `FAL_KEY`, and optionally `COMFYUI_API_KEY`. A project file can choose models and endpoints but cannot send a key to a new host.
* **Transparency is proven from pixels**, never from the prompt. An opaque result cannot become a transparent icon, and a drawn checkerboard is rejected.
* **Nothing is overwritten.** Candidates are `name-v1`, `name-v2`...; processed art is `-p1`, `-p2`...; an installed file is replaced only if this tool installed it and it is unchanged since.
* **Everything stays inside the mod project.** No `..`, drive, UNC, stream, device-name, short-name or symlink escapes.
* **Deterministic checks are not a style verdict.** Composition, style and readability at game size go to the review, recorded separately.
* **Model ids retire quickly** (three in eight months). Ids, limits and prices live in dated tables under `crates/imagegen-providers/data/`; a retired model is refused before any request.

### Maintainer decisions built in

| | |
|---|---|
| D3 | A ComfyUI endpoint you configured may be used for any asset category, including user-run ones. It is a plain HTTP client; it never starts or stops pods |
| D4 | Paradox art (the installed game, the kit's vanilla shelf) is sent to a provider as a reference only when the project sets `policy.allow_vanilla_references = true`. Off by default |
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
allow_vanilla_references = false     # D4
forbidden_categories = []            # for example ["leader_portrait"]
max_usd_per_call = 1.0

[providers.openai]
quality = "low"                      # the cheapest tier; raise it when review says so

[providers.comfyui]                  # D3
endpoint = "http://127.0.0.1:8188"
workflow = ".hoi4-imagegen/comfyui/text_to_image.json"

[profiles.focus_icon]                # per-project canvas override
canvas = [94, 86]
```

### Profiles

Canvas conventions come from the kit's `.tools/gfx/profiles.json` when it exists, over built-in defaults measured from HOI4 1.19.3.0 (focus 100x88, idea 65x67, decision 33x32, report 210x176, news 397x153, flags 82x52 / 41x26 / 10x7, and more). Every value is overridable and `hoi4-imagegen calibrate` shows where one is not the dominant vanilla size. The built-in defaults are headers-only measurements; style cannot be measured and stays a review matter.

## Layout

```
crates/imagegen-core        categories, profiles, processing, validation, DDS/TGA, naming, ledger. No network
crates/imagegen-providers   one trait; OpenAI, Gemini, fal, ComfyUI; destination-checked HTTP
crates/imagegen-mcp         the operations and the rmcp stdio server (binary hoi4-imagegen-mcp)
crates/imagegen-cli         hoi4-imagegen: doctor, calibrate, plan, generate, validate, install, call
crates/provider-chatgpt-login   OPT-IN plugin, not a default member, see below
skill/hoi4-imagegen         the agent skill (under 8 KB)
docs/                       provider notes, the plugin's terms and risks, test layers
```

## The opt-in ChatGPT login

The upstream project's route signs in with a ChatGPT account and calls an **undocumented** backend while presenting itself as the Codex CLI. It can break or be blocked without notice, may not be covered by OpenAI's terms, and heavy use can risk account restrictions. It is therefore **not** in default builds, **not** a default workspace member, and never part of hosted HOI4 Factory. It needs all of: a source build with `--features unofficial-chatgpt-login`, `[providers.chatgpt_login] enabled = true` in the project config, `HOI4_IMAGEGEN_ACCEPT_UNOFFICIAL=1` in your own environment, and local stdio or CLI mode. A test proves a default build contains none of it. Read [docs/unofficial-chatgpt-login.md](docs/unofficial-chatgpt-login.md) first.

## Status and limits

* Verified by running tests: see the test layers in [docs/testing.md](docs/testing.md).
* **Unavailable, with the reason:** recordings of real provider responses (no API key with balance is known to exist), end-to-end runs with real providers, the in-game check including whether a focus without a `_shine` sprite logs an error (needs a user-operated game session), and macOS and Linux runs (this phase ran on Windows only).
* The Gemini response envelope is read from the documented content schema; the docs show no raw image response. The OpenAI 2.5 per-image price is unpublished, so estimates use the `gpt-image-2` row as a labelled proxy until a recorded call reports usage.
* Rights: Paradox's and Steam's rules on AI-generated mod assets and the providers' output terms were not researched. Nothing here promises anything about them.
* `register` mode edits `.gfx` files as text (comments and layout are kept). A parser-based version waits for the shared Rust HOI4 engine.

## License

MIT. See [LICENSE](LICENSE), [NOTICE](NOTICE) and [UPSTREAM.md](UPSTREAM.md).
