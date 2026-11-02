# Kit integration

The [Agentic HOI4 Modding Kit](https://github.com/klimPaskov/Agentic-HOI4-Modding) and this repository work together. This page says what the integration is, how the pieces connect, and what is still pending. The kit is a separate repository, so check its `main` for the current state.

## What the integration is

* **Profile rows.** The kit's `.tools/gfx/profiles.json` has one row per family, 40 rows in all, with the canvas for new art, accepted sizes, minimum margin, frame count and the vanilla evidence for each number. The values are the sizes measured on HOI4 1.19.4.1. The kit stays the single owner of those sizes. This repository reads the same file over its built-in defaults (also measured from 1.19.4.1) and under the project's own overrides, and reports the difference with `hoi4-imagegen calibrate`.
* **`family_ids.json`.** `.tools/gfx/family_ids.json` is the kit's list of the 40 family ids. The kit's `shelf` check verifies that its profiles, shelf catalog and shelf builder input name exactly that list.
* **The shelf.** The kit's asset skill ships a vanilla reference shelf (`assets/vanilla_reference/` with a `CATALOG.md` that lists the canvas of every family). A mod's copy of the shelf is style tier 2, and the advisor, achievement and portrait templates and the YuNet face model come from the mod's copy of the same skill (see the [reference](reference.md)).
* **Skills and agents.** The kit's `hoi4-imagegen` skill (a downstream copy of `skill/hoi4-imagegen/SKILL.md` in this repository) and its icon, portrait and generated-feature-art agents call the `hoi4.image_*` tools. They pick the `family`, run the loop and never build a second image pipeline. The kit's `hoi4-feature-assets` skill keeps the source, naming and handoff rules that the loop follows.
* **HOI4 Mod Setup.** Setup installs the release binary per project and writes the project-level MCP entry under the server id `hoi4_imagegen`. The kit's setup manifest lists the skill and the runtime.

Changes to the skill are made here first and copied to the kit. The two copies should differ only in kit-specific lines (the kit's copy points to `hoi4-feature-assets`, `hoi4-frame-animation` and the kit's own tools).

## Still pending

* The kit's rewrite of its `hoi4-imagegen` skill and its profile and shelf work were on the kit branch `claude/hoi4-imagegen` when this page was written. Check that they are merged.
* `hoi4.image_audit` runs the shared engine's sprite trace (hoi4-agent-tools) over the whole mod and reports cross-family reuse as a warning, with a per-mod allowlist. New reuse is refused at import and install.
* This repository does not read `family_ids.json`. Its own list of 40 families is in the code, and a test keeps it equal to the measurement data. A check that the two lists agree is not built.
* The kit's profile rows carry geometry only. None has an `imagegen` object yet, so every family uses the built-in sprite, naming and check defaults.
* Nothing here has been checked in game. That needs a user-operated session.

## Row names

A row named `focus`, `decision`, `idea` (spirits), `category` (decision category icons), `report`, `news`, `counter-large` or `counter-small` maps to the family of the same meaning (these are the names `gfx.py` uses). Any other row name that is a family id (`idea_spirit`, `advisor_card`, `technology`, and the rest of the 40) maps to that family. A row with any other name is ignored with a note. There is no `custom` family, because an asset without a family cannot be checked or kept apart from the others.

## The `imagegen` object

`gfx.py` reads only `canvas`, `accepted`, `margin`, `frames` and `off_size_level` from a row and ignores other keys, so the optional `imagegen` object is safe next to it. All its keys are optional, and each overrides the built-in default for that row.

| Key | Meaning |
|---|---|
| `automation` | `generate` (generated, processed, validated and installed here), `import_only` (another step makes the art, as for the counters and the division emblem) or `user_run_only` (a person runs the generator) |
| `background` | `transparent` or `opaque` |
| `container` | `dds` (uncompressed A8R8G8B8, one mip) or `tga` |
| `folder`, `prefix`, `alt_prefixes` | Final folder and file prefix with `{feature_slug}`, `{asset}` placeholders; other prefixes the game accepts (focus accepts `goal_`) |
| `sprite` | `{gfx_file, name, companions}`; companions may include `shine` |
| `variants` | Extra size variants as `[{name, canvas, folder}]` (flags have three) |
| `checks` | Deterministic checks of the family (every one always runs; `hoi4.image_validate` cannot skip any) |
| `companion_files` | Companion file suffixes the engine expects (`_grey`, `_not_eligible`, `_small`) |
| `reference_dirs` | Folders of approved references |
| `rubric` | Review rubric id (one per group: `icon_v1`, `picture_v1`, `portrait_v1`, `flag_v1`, `unit_v1`, `interface_v1`) |
| `prompt` | `{rules, negative}` sentence lists; `{w}` and `{h}` are replaced by the canvas |
| `grayscale` | Convert to black and white (news pictures) |
| `source_canvas` | Size the model is asked for when it is not the canvas (the advisor card is made from a tall 156x210 portrait) |

The processing recipe of a family (`icon`, `report_card`, `achievement`...) is not a profile key: it follows from the family and cannot be overridden, so a kit row cannot switch the finishing off. Templates and the face model are found inside the project (`.claude/skills/*/assets/*/`, `.claude/skills/*/tools/models/`) or through `[templates] dir` and `face_model` in `.hoi4-imagegen/config.toml`.

## Rows

The built-in defaults are the vanilla-measured values of HOI4 1.19.4.1 (`crates/imagegen-core/data/vanilla-measurement.json`), one row per family. A kit row overrides any of them. A mod keeps the sizes it chose (for example 94x86 for focus, 64x64 for ideas and 32x32 for decisions) by listing them in its own `.tools/gfx/profiles.json`, and `hoi4.image_families` then shows the override next to the vanilla value. Examples:

```json
"focus": {
  "canvas": [94, 86], "accepted": [[94, 86], [100, 88]], "margin": 2, "frames": 1,
  "source": "project decision D1",
  "imagegen": {"prefix": "goal_"}
},
"idea_spirit": {"canvas": [64, 64], "accepted": [[64, 64], [60, 68]], "margin": 1, "frames": 1, "source": "project decision D2"},
"gui_texture": {"canvas": [300, 100], "accepted": [[300, 100]], "margin": 0, "frames": 1, "source": "the mod's own window art"}
```

`gui_texture` and `super_event` have no vanilla texture to measure, so a mod defines their canvas this way (or per request with `canvas`). `hoi4-imagegen calibrate` re-measures the families against the installed game, and `hoi4-imagegen families` lists every family with its measured spec and the mod's override.
