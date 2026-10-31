# Kit integration (proposal)

Status: **proposed, nothing in the kit has been changed.** The kit's `.tools/gfx/profiles.json` stays the single owner of canvas sizes, accepted sizes, margins, frames and evidence. `gfx.py` reads only `canvas`, `accepted`, `margin` and `frames` from a row and ignores other keys, so the optional `imagegen` object below is safe next to it. This repository reads the same file (built-in vanilla-measured defaults underneath, project overrides on top) and reports the difference with `hoi4-imagegen calibrate`.

## Row names

A row named `focus`, `decision`, `idea`, `category`, `report`, `news`, `counter-large` or `counter-small` maps to the category of the same meaning (these are the names `gfx.py` already uses). Any other row name that is a category id (`tech_icon_small`, `mio_icon`, `agency_logo`, `officer_spirit`, `event_super`, `leader_portrait`, `flag_set`, `tech_icon_medium`, `decision_category_picture`) maps to that category. An unknown name becomes a `custom` profile.

## The `imagegen` object

All keys optional; each overrides the built-in default for that row.

| Key | Meaning |
|---|---|
| `automation` | `generate`, `import_only` (another kit tool builds the art) or `user_run_only` |
| `background` | `transparent` or `opaque` |
| `container` | `dds` (uncompressed A8R8G8B8, one mip) or `tga` |
| `folder`, `prefix` | Final folder and file prefix with `{feature_slug}`, `{asset}` placeholders |
| `sprite` | `{gfx_file, name, companions}`; companions may include `shine` |
| `variants` | Extra size variants as `[{name, canvas, folder}]` (flags have three) |
| `checks` | Deterministic checks to run |
| `reference_dirs` | Folders of approved references |
| `rubric` | Review rubric id |
| `prompt` | `{rules, negative}` sentence lists; `{w}` and `{h}` are replaced by the canvas |
| `grayscale` | Convert to black and white (news pictures) |

## Proposed rows

Canvas values are the vanilla-measured defaults (HOI4 1.19.3.0 header survey). They change the kit's focus convention from 94x86 to 100x88, idea from 64x64 to 65x67 and decision from 32x32 to 33x32: that is a maintainer decision, recorded as "vanilla-measured, overridable per project" in the expansion plan.

```json
"focus": {
  "canvas": [100, 88], "accepted": [[100, 88], [102, 90], [99, 88], [98, 88]], "margin": 2, "frames": 1,
  "source": "HOI4 1.19.3.0 header survey: 2,681 of 2,849 focus_* DDS are 100x88. Margin is a review default.",
  "imagegen": {
    "automation": "generate", "background": "transparent", "container": "dds",
    "folder": "gfx/interface/goals/{feature_slug}", "prefix": "goal_",
    "sprite": {"gfx_file": "interface/{feature_slug}_goals.gfx", "name": "GFX_{prefix}{asset}", "companions": ["shine"]},
    "checks": ["size", "alpha", "opaque_canvas", "fringe", "margin", "fill", "checker", "ghost", "dds"],
    "rubric": "focus_icon_v1"
  }
},
"idea": {
  "canvas": [65, 67], "accepted": [[65, 67], [60, 68], [64, 64], [65, 64]], "margin": 1, "frames": 1,
  "source": "2,866 idea_* DDS: 65x67 43.6%, 60x68 35.8%, 64x64 17.6%.",
  "imagegen": {"folder": "gfx/interface/ideas/{feature_slug}", "prefix": "idea_",
    "sprite": {"gfx_file": "interface/{feature_slug}_ideas.gfx", "name": "GFX_{prefix}{asset}"}, "rubric": "idea_icon_v1"}
},
"decision": {
  "canvas": [33, 32], "accepted": [[33, 32], [38, 35], [38, 38], [38, 37], [33, 31], [32, 32]], "margin": 1, "frames": 1,
  "source": "503 decision_* DDS: 33x32 is 236 files; 32x32 under 5.",
  "imagegen": {"folder": "gfx/interface/decisions/{feature_slug}", "prefix": "decision_",
    "sprite": {"gfx_file": "interface/{feature_slug}_decisions.gfx", "name": "GFX_{prefix}{asset}"}, "rubric": "decision_icon_v1"}
},
"tech_icon_small": {"canvas": [64, 64], "accepted": [[64, 64], [62, 62]], "margin": 2, "frames": 1,
  "source": "1,318 technology DDS: 64x64 is the most common size (328)."},
"tech_icon_medium": {"canvas": [146, 54], "accepted": [[146, 54], [131, 52], [132, 52]], "margin": 2, "frames": 1,
  "source": "LOW CONFIDENCE: 146x54 269 files, 131x52 55; the kit's 132x52 is not among the top sizes."},
"mio_icon": {"canvas": [48, 48], "accepted": [[48, 48], [60, 60], [63, 63]], "margin": 2, "frames": 1,
  "source": "425 MIO textures: 48x48 381, 60x60 20, 63x63 4."},
"agency_logo": {"canvas": [117, 119], "accepted": [[117, 119]], "margin": 2, "frames": 2,
  "source": "about 70 agency logos are 233x119 or 234x119 with noOfFrames = 2.", "imagegen": {"automation": "import_only"}},
"officer_spirit": {"canvas": [45, 45], "accepted": [[45, 45]], "margin": 1, "frames": 1,
  "source": "149 of 222 officer corps spirits are 45x45."},
"event_super": {"canvas": [457, 328], "accepted": [[457, 328]], "margin": 0, "frames": 1,
  "source": "Not vanilla: the optional Super Events runtime template (457x328)."}
```

`hoi4-imagegen calibrate` re-measures all of these against the installed game, so the numbers can be refreshed rather than trusted.

## Other kit changes this implies (not made)

* `HOI4_MOD_SETUP:IMAGE_GENERATION_PROVIDER` marker text in `hoi4-icon-artist`, `hoi4-generated-feature-art`, `hoi4-portrait-creator`, `hoi4-feature-assets/SKILL.md` and `references/generation-and-review.md`: describe this route and the selected provider. `hoi4-portrait-creator` uses it only for fictional portraits and for validating user-run output.
* `hoi4-feature-assets` sections 5, 11, 24 and 25: one line each pointing at the MCP. The DDS and handoff rules stay authoritative.
* Skill: add `skill/hoi4-imagegen/` from this repository (4.6 KB) as a kit workflow component, with `examples/recipes.md` as its reference.
* D3 and D4 wording in the asset skills: a configured ComfyUI endpoint may be used for any asset; vanilla art is a reference only when the project opts in.
* Remove, after this MCP covers OpenAI, Gemini, ComfyUI and fal: the vendored Node gptimage copy, `default_provider = "gptimage"`, the `vertex_imagen` route (Imagen is discontinued) and the `gpt-image-1` text in `docs/image-generation-providers.md` (shut down 2026-10-23).

## What remains for later phases

* **HOI4 Mod Setup component:** a pinned release-binary component kind (decision: Setup release-binary component, not npm), manifest entries `mcp.hoi4_imagegen`, client config entries for each client with empty `env`, and credential access through Setup's keyring references (this phase reads environment variables only).
* **CI:** build and test on Windows, macOS arm64, Linux amd64 and arm64; `cargo deny` for licenses; the plugin isolation test on the release binaries; a model-id drift job.
* **Releases:** signed SHA-256 checksums for Windows x64, macOS arm64, Linux amd64 and arm64; SBOM and attestations.
* **Publishing:** creating `klimPaskov/hoi4-imagegen-mcp` is an outward action; nothing was created or pushed.
* **Evidence still missing:** real-provider recordings, the in-game check (shine sprite), macOS and Linux runs, the rights research (Paradox, Steam and provider output terms).
