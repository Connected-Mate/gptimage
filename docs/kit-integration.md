# Kit integration (proposal)

Status: **proposed, nothing in the kit has been changed.** The kit's `.tools/gfx/profiles.json` stays the single owner of canvas sizes, accepted sizes, margins, frames and evidence. `gfx.py` reads only `canvas`, `accepted`, `margin` and `frames` from a row and ignores other keys, so the optional `imagegen` object below is safe next to it. This repository reads the same file (built-in vanilla-measured family defaults underneath, project overrides on top) and reports the difference with `hoi4-imagegen calibrate`.

## Row names

A row named `focus`, `decision`, `idea` (spirits), `category` (decision category icons), `report`, `news`, `counter-large` or `counter-small` maps to the family of the same meaning (these are the names `gfx.py` already uses). Any other row name that is a family id (`idea_spirit`, `idea_concern`, `advisor_card`, `technology`, `mio_department`, `agency_logo`, `officer_corps_spirit`, `super_event`, `leader_portrait`, `flag`, and the rest of the 40) maps to that family. A row with any other name is ignored with a note: there is no `custom` family, because an asset without a family cannot be checked or kept apart from the others.

## The `imagegen` object

All keys optional; each overrides the built-in default for that row.

| Key | Meaning |
|---|---|
| `automation` | `generate`, `import_only` (another step builds the art: frame strips, cards) or `user_run_only` |
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

## Rows

The built-in defaults are the vanilla-measured values of HOI4 1.19.4.1 (`crates/imagegen-core/data/vanilla-measurement.json`), one row per family. A kit row overrides any of them; a mod keeps the sizes it chose (for example 94x86 for focus, 64x64 for ideas and 32x32 for decisions) by listing them in its own `.tools/gfx/profiles.json`, and `hoi4.image_families` then shows the override next to the vanilla value. Examples:

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
