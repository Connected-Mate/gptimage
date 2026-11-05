# Reference

The full reference for `hoi4-imagegen-mcp`. The [README](../README.md) is the short tour, and the [asset gallery](families.md) shows every family with a real generated example.

## What it does

```
status -> families -> icon | picture | portrait | flag | unit | interface   (free plan, or plan + generate with approve)
       -> process -> validate -> preview -> review -> install
```

* Every asset has a **family**. The family alone selects the canvas, frames, naming, references, processing and checks. An asset keeps its family from the plan to the install record.
* The canvas sizes are measured from the installed game (HOI4 1.19.4.1), not guessed.
* Processing cannot be skipped. Each family has a fixed chain that the tool appends itself.
* Install is gated: validation passed, a review approved this exact file, and nothing changed since. An approval needs the file to measure like its family's vanilla art.
* Nothing leaves the mod project and nothing is overwritten.

## Tools

| Tool | What it does |
|---|---|
| `hoi4.image_status` | Providers (key present or not, never the value), model tables with their date, game folder, profile source, ledger chain, ChatGPT-login state |
| `hoi4.image_families` | The catalogue: every family (or one) with its group and tool, the vanilla-measured canvas and frames, accepted sizes, the mod's override, naming, checks, review rubric, final paths and the style sources that exist for it. `calibrate: true` compares the table with the installed game's file headers |
| `hoi4.image_references` | Reference images with role and rights note, and project style profiles pinned by SHA-256 |
| `hoi4.image_icon`, `hoi4.image_picture`, `hoi4.image_portrait`, `hoi4.image_flag`, `hoi4.image_unit`, `hoi4.image_interface`, `hoi4.image_animation` | One tool per group of families. Each has a **required `family`** whose schema enum lists only that group's families. Without `approve`: a free plan (prompt, references, provider, model, size, cost estimate with its basis, output paths) and a `plan_id`. With `approve: true` and an `idempotency_key`: plan and generate in one call |
| `hoi4.image_generate` | Runs a stored plan. One provider call per variant. The same `idempotency_key` replays instead of paying again |
| `hoi4.image_edit` | Refines a candidate with an instruction, an optional mask and extra references. Also makes the ruined second frame of a building and the further key frames of an `anim_blend_keyframes` strip |
| `hoi4.image_process` | Runs the family's processing chain. Also imports art made elsewhere. `frame_candidates` names the edits that become frames 2 onward of a key-frame strip |
| `hoi4.image_validate` | The family's full check set, with stable finding codes |
| `hoi4.image_preview` | A contact sheet: native size and enlarged, on dark, light and checker backgrounds, beside the references. For an artifact also the vanilla side-by-side sheet and the measured vanilla-match result. An animated strip also gets a frame sheet and a looping GIF |
| `hoi4.image_review` | The visual review against the family's rubric, asked against the vanilla references. The review runs the measured vanilla-match check itself. An approval returns the `approval_id` that install needs |
| `hoi4.image_install` | Writes the DDS or TGA files and the sprite. Refuses unless the gate is open |
| `hoi4.image_ledger` | Which references, settings and outputs belong together, lineage, and the chain check |
| `hoi4.image_cancel` | Cancels a running job, reconciles one that a crashed run left behind, or retires an uncertain idempotency key (`abandon: true`) |
| `hoi4.image_login` | Opens the ChatGPT sign-in page in the user's browser and returns at once. The sign-in finishes in the background, and `hoi4.image_status` reports it. Signing in accepts the risk note, which the result includes. Local only |
| `hoi4.image_audit` | Audits the whole mod on the shared engine (hoi4-agent-tools): which families use each sprite and texture, cross-family reuse (`GFX-FAMILY-REUSE`, a warning, with a per-mod allowlist), missing textures, undefined sprites, missing focus shine (an error: the game then draws a `?`), wrong canvas sizes and report cards without transparent corners. Read only |

That is 21 tools.

## Families

There are 44 families in seven groups. The group names the tool that plans them.

| Tool | Families |
|---|---|
| `hoi4.image_icon` | focus, idea_spirit, idea_concern, officer_corps_spirit, decision, decision_category_icon, state_modifier, special_project_icon, raid, mio_department, mio_trait, mio_policy, bop_side, medal, agency_upgrade, operation_icon, commander_trait, personal_trait, doctrine |
| `hoi4.image_picture` | report_event, news_event, decision_category_picture, operation_picture, technology, achievement |
| `hoi4.image_portrait` | leader_portrait, commander_portrait, operative_portrait, scientist_portrait, advisor_card |
| `hoi4.image_flag` | flag |
| `hoi4.image_unit` | counter_large, counter_small, division_emblem, building, faction_logo, agency_logo |
| `hoi4.image_interface` | modifier_stat_icon, gui_texture, super_event |
| `hoi4.image_animation` | anim_blend_glow, anim_blend_keyframes, anim_scroll, anim_spin |

### Measured from the game

The built-in canvas, frames, accepted sizes and pixel class of every family come from the installed game, version 1.19.4.1. The data is in `crates/imagegen-core/data/vanilla-measurement.json`, made by `scripts/measure_families.py`. A unit test fails when a family stops matching it. Examples: focus 100x88, idea_spirit 60x68, advisor_card 65x67, decision 33x32, report_event 210x176, news_event 397x153, flags 82x52, 41x26 and 10x7.

The emblem inside an icon is fitted at vanilla's size too, measured as the median over the family's vanilla files. mio_trait and agency_upgrade keep an 8 px margin and operation_icon 6 px, the clear margin every vanilla file of those families has. raid keeps 4 px and idea_concern 3 px, the margins closest to the vanilla median. A test (it needs the game) checks these and focus, decision_category_icon, mio_department, doctrine and commander_trait against the vanilla median, within 1 px.

A mod can override the canvas and naming in its own `.tools/gfx/profiles.json` (the kit's file) or in `.hoi4-imagegen/config.toml`. Without an override the vanilla size is used. `hoi4.image_families` shows the override next to the vanilla value, and `hoi4-imagegen calibrate` shows where a built-in value is not the dominant vanilla size. `gui_texture` and `super_event` have no vanilla texture to measure, so the mod defines their canvas.

### Animated sprites

The four animated families are measured from the 160 `frameAnimatedSpriteType` sprites of 1.19.4.1 (`scripts/measure_animated.py` writes the `animated_sprites` section and the four `anim_*` entries of the measurement file). Each family copies the frame count, `animation_rate_fps`, `looping`, `play_on_show`, `pause_on_loop` and effect file of the vanilla sprites it is measured from, and a unit test keeps them equal.

| Family | Frames and rate | Vanilla model | How the frames are made |
|---|---|---|---|
| `anim_blend_glow` | 2 at 1 fps, `buttonstate_blendframes.lua` | 70 two-frame glows (alerts, buttons, highlights) | The model draws the bright glow on black. The tool turns the light into alpha and makes frame 1 the same glow at 0.17 of its alpha, the vanilla median |
| `anim_blend_keyframes` | 3 at 1 fps, `buttonstate_blendframes.lua` | the ongoing-focus and spirit glows | Frame 1 is generated. Frames 2 and 3 are `hoi4.image_edit` candidates of it, passed as `frame_candidates`, and all three get one shared crop and scale |
| `anim_scroll` | 9 at 15 fps | 38 currently-researching highlights | The picture is made seamless across its width and moves right by a ninth of it per frame |
| `anim_spin` | 20 at 7 fps | the turning cogs of the leader-group and navy views | The subject is fitted by its radius and turned a full circle, each frame at the source resolution |

Vanilla sizes these strips to the element they sit on (66 glow textures come in 47 sizes), so a plan may pass `canvas`, the size of one frame. The `animation` check measures what is left to drift. `GFX-ANIM-STATIC` (an error) means nothing moves. `GFX-ANIM-ALIGN`, `GFX-ANIM-SILHOUETTE` and `GFX-ANIM-PALETTE` compare each frame's centroid, silhouette and mean colour with frame 1. `GFX-ANIM-SEAM` means the loop visibly restarts, and `GFX-ANIM-JUMP` means one step is much larger than the rest. The limits are set from the vanilla strips of the same kind, and a test checks that those strips pass. The audit adds `GFX-ANIM-FRAMES` for a mod `frameAnimatedSpriteType` whose strip does not divide into its frames or does not have its family's frame count and effect file.

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
| `icon` | transparent icons, technology, modifier_stat_icon, gui_texture | trim, fit to the canvas with the profile margin, grayscale where the family needs it. A focus icon also gets its `_shine` sprite entry at install. A technology picture gets its tier numeral when `overlay` asks for one (see below) | one DDS |
| `raid` | `raid` | trim, fit, then `gold_gradient`: the subject recoloured on vanilla's gold ramp, pale gold at the top to brown at the bottom, with the black outline on its edge | one DDS |
| `doctrine` | `doctrine` | trim, fit 2 px further in, then `doctrine_look`: the units filled black with vanilla's pale outline and glow, warm orange and yellow accents kept with a dark shadow | one DDS |
| `line_art` | `special_project_icon` | `line_art`: any image redrawn as vanilla's thin, pale white lines with nothing filled, then grayscale | one DDS |
| `picture` | decision pictures, news, super_event | flatten on black, cover-resize, grayscale for news | one DDS |
| `report_card` v2 | `report_event` | flatten, then the report card: sepia, grain, tilt and a soft shadow fitted to vanilla, 210x176 with transparent corners | one DDS |
| `phase_card` | `operation_picture` | flatten, then the report card on vanilla's phase picture mask: tilted clockwise and 4 px lower | one DDS |
| `achievement` | `achievement` | fit the subject, then the grey state and the not-eligible state on the mod's template backgrounds | three DDS: `<key>`, `<key>_grey`, `<key>_not_eligible` |
| `advisor_card` | `advisor_card` | flatten, then the portrait is composited into the mod's advisor template, with an alpha-coverage audit that fails closed | one 65x67 DDS |
| `portrait` | the four portraits | flatten, then a close head-and-shoulders crop to 156x210 around the face YuNet finds (pure-Rust `tract-onnx`, model from the mod's skill copy). The crop is framed like vanilla's leader portraits: the face box is 0.543 of the height with its top at 0.181, measured on 70 vanilla files. The advisor card keeps the kit's looser framing. `portrait_crop` sets the crop by hand | one opaque DDS |
| `flag` | `flag` | flatten, then the 82x52, 41x26 and 10x7 ladder | three TGA, bottom-left origin |
| `selected_strip` | `faction_logo`, `agency_logo` | fit with a margin, then frame 2 as the selected state. A faction logo also gets its 32x32 `_miniature` | one two-frame DDS, plus the miniature for a faction logo |
| `landmark_strip` | `building` | fit, then frame 2 as the ruined landmark, made from a second candidate of the same asset (`hoi4.image_edit` on the first, passed as `second_frame_candidate`). A 64x64 landmark also gets the 32x32 `_small` strip, a flat orange pictogram like vanilla's small landmarks | one or two two-frame DDS |
| `unit_counter` | `counter_large`, `counter_small` | `unit_glyph`: the silhouette in vanilla's green (or the on-map grey) material, then `nato_frame`: the NATO box as frame 2. The on-map counter also gets its text icon | one DDS, or two |
| `division_symbol` | `division_emblem` | `unit_glyph` in vanilla's green material, and the 30x12 small symbol | two DDS |

A medal is made as one 56x84 frame, registered as `GFX_medal_icon_<asset>_large`, and its 28x42 `GFX_medal_icon_<asset>` is derived from it. The medal is fitted into vanilla's 33x56 box in that frame (`fit_in_box`), so it spans two thirds of the frame as vanilla's do.

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

Art that already has its final accepted size is taken as it is when imported, which the chain records as `conform`. Counters and division emblems are stricter: imported art of the final size is kept only when it already is vanilla's material (its frame 1 green or grey as vanilla's, its frame 2 the NATO box). Any other import, at any size, is drawn again from its frame 1 by the unit recipe, and the process result says so in `notes`. The `unit_material` check refuses such art if it reaches validation another way, so every counter and division emblem ends in the unit recipe.

On an opaque source, the unit recipe reads light coloured marks inside a dark shape as holes, the same as on real alpha. A gold bolt on a blue shield therefore stays a bolt cut into the shield. Black, grey and white drawings are not affected.

A technology picture can carry vanilla's tier numeral. Vanilla bakes the numeral into the texture: no `.gui` layer or sprite draws it (the tech tree item in `interface/countrytechtreeview.gui` shows one `GFX_technology_medium` icon). Pass `overlay` to `hoi4.image_process`, for example `{"tier": 2}`, or `{"tier": 3, "gem": true}` for the blue gem that battleship, cruiser and carrier pictures have before the numeral. The tool stamps vanilla's serif numeral in its gold and shadow, in vanilla's place at the top right (`position` moves it). The stamp is a recorded `tech_overlay` step.

Some checks guard a family's look:

* `line_art` (special_project_icon) refuses a filled area larger than 250 px, `GFX-FILLED-INTERIOR`. Vanilla's largest is 171 px and its median 4.
* `unit_material` (counters and division emblem) refuses finished art that is not vanilla's material, `GFX-UNIT-MATERIAL`.
* `side_view` (technology) warns when the subject is narrower than vanilla's side views, `GFX-TECH-VIEW`. Nine in ten vanilla pictures are at least 2.3 times as wide as tall at 146x54. A three-quarter view is narrower, so the review should look at it.

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

A pinned style profile is tier 1 only when it applies to the family. A plan's `style: <name>` always applies it. Otherwise a profile applies by itself only when its `families` list names the family, or when it has `default: true` (then it covers the families it lists, or every family when it lists none). A profile with an empty `families` list and no `default` is available by name to any family but never applied by itself. `default: false` turns automatic use off. A profile that names the family ranks above a `default: true` profile for all families. The plan says which profile it applied and why.

Tier 4 keeps each family to the vanilla files of its one form. Commander traits leave out the operative traits. operation_icon takes only the 59x58 phase icons and operation_picture only the 210x176 phase pictures, building takes only the 128x64 painted landmark strips, and bop_side leaves out the coloured `_improved` and `_max` state variants. A `gui_texture` takes vanilla references only when the request names its `texture_kind`: `button` (`gfx/interface/button_*` and `gfx/interface/buttons/`), `frame` (`gfx/interface/*_frame*`), `panel` (`gfx/interface/tiles/tiled_*`) or `background` (`gfx/interface/*_bg` of at least 256 px a side). The kind also adds its vanilla form to the prompt.

When the mod's own references (tier 2 or 3) show a form, such as a frame or a background, that form wins over the vanilla form the family rules describe. References the request sends itself with the role `style_anchor` win the same way. Then no vanilla reference and no vanilla style paragraph go with them, because vanilla art beside them pulls the image back to the vanilla form.

The first tier with at least 3 usable images is used. A pinned style is always sent, even with fewer images, but then gives no statistics. The plan reports every reference with its tier, reason, SHA-256 and whether it goes to the provider (`style_sources` in the plan, also in `hoi4.image_families`).

Selection is deterministic. Each candidate is measured (colour, saturation, edge density, alpha, outline share, hue histogram), and a fixed k-medoids picks up to 4 images to send, within the model's reference limit. The same images give the same choice. The measured statistics also become a short style paragraph in the prompt.

`style` is part of every family's check set. It warns with `GFX-STYLE-DRIFT` when a candidate is far from the family's style distribution. It is a warning, never an error, and never blocks an install.

Vanilla references may always be sent to a provider (a maintainer decision of 2026-11-01), and the tool never copies a vanilla file into a mod. Vanilla art is left out of the contact sheet that is written into the mod. The vanilla side-by-side sheet (see the review below) goes to the user's cache folder instead.

## Validation and install

Install is the last step and the strictest.

1. `hoi4.image_validate` runs the family's full check set, never fewer, with stable finding codes (size, alpha, margins, fill, ghosting, chroma fringe, fake checkerboard, frames, DDS or TGA layout, style). The record carries a hash of the check set.
2. The gate compares that hash and audits the processing chain against the family's recipe. A record with a missing step is refused.
3. `hoi4.image_preview` makes the contact sheet and the vanilla side-by-side sheet. `hoi4.image_review` answers the family's rubric and runs the vanilla-match check (next section). Agents may approve. The reviewer's name is recorded with the review.
4. `hoi4.image_install` needs the `approval_id` of an approval of this exact file, and refuses if any hash changed since.
5. The write is atomic: every file of an asset (companions, `.gfx` text or the handoff note) is written together and all are undone if one fails.

`mode: handoff` (default) writes the sprite snippet into `docs/assets/<feature>/gfx_handoff.md` for the main agent to wire. `mode: register` inserts the entries into the suggested `.gfx` file as text, keeping comments and layout. Families without a sprite (achievements and flags) are loaded by the game from the file name, so both modes install the files alone. A final file is replaced only if this tool installed it and it is unchanged (`on_exists: replace`).

Deterministic checks are not a style verdict. Composition and readability at game size go to the review. In-game appearance can only be checked by running the game, which this tool never does. It says "installed", not "verified in game".

## Review and the vanilla-match check

Until 0.3.1 an approval needed only every rubric answer `true`, so a caller that sent blanket `true` answers approved anything. Checked against vanilla, several approved gallery images were wrong. Since 0.3.2 the answers alone never approve.

An approval needs all of these, in this order:

1. a validation of this exact file that ran the family's full check set and passed (`review_needs_validation`),
2. a preview of this exact file (`review_needs_preview`) that put it beside the family's vanilla references (`review_needs_vanilla_preview`),
3. a passing vanilla-match check, which the review runs itself (`review_vanilla_mismatch` when it fails, `review_needs_vanilla_check` when it cannot run, for example without the installed game),
4. every rubric answer `true` (`review_incomplete`).

**The vanilla side-by-side sheet.** For an artifact, `hoi4.image_preview` also makes a second sheet: the candidate under a light bar, then the family's six most typical vanilla images, at native size on the top row and enlarged on the bottom row. It holds vanilla pixels, so it is written outside the mod, to `HOI4_IMAGEGEN_CACHE_DIR` or the platform cache folder (`%LOCALAPPDATA%/hoi4-imagegen/cache` on Windows, `$XDG_CACHE_HOME/hoi4-imagegen` or `~/.cache/hoi4-imagegen` elsewhere). The MCP reply returns it as a second image. The preview record lists every vanilla file shown by path and SHA-256.

**The vanilla-match check.** The family's vanilla art is read from the installed game, through the same folders and name rules as the vanilla style tier, at the family's vanilla sizes (large medals are cut out of their strips). Each image is measured: palette (lightness, saturation, value, their spread, dark, bright and middle shares, colours, gold tones), outline share, edge density, alpha coverage, soft alpha, background fill, and form (the subject's size, position and aspect, how much of its box, convex hull and inscribed ellipse it fills, left-right symmetry, the saturation of its top band where a ribbon sits). Each family gets an accepted range per feature from its own vanilla files: the 1st to 99th percentile, widened on each side by half of that width.

Thirteen features can fail a candidate: the form features and the two medium features that separate painted, line and grey art (the middle-value share and the saturation). The others are measured and reported only, because the art labelled good differs from vanilla in them as much as the rejected art does. A failure names each feature with its value, vanilla's range and vanilla's median. On HOI4 1.19.4.1 between 97.6 and 100 percent of each family's own vanilla files pass its check. The check is version `vanilla_match_v1`, recorded with every result.

The check is a gate, not a verdict. Art that looks wrong in a way these features cannot see still passes it, and must then fail the rubric. For example, on the gallery the loose steel gate (focus), the war economy cluster (idea spirit) and the bread basket (industrial concern) measure inside vanilla's own spread. The v3 rubric questions are asked against the vanilla sheet for that reason.

**Families without vanilla art** (`gui_texture`, `super_event` and the animation families) cannot be checked. Approve them with an override.

**Override.** A deliberate mod style can be approved past a failing or unavailable check with `"override_vanilla_check": true` and an `"override_reason"` of 10 to 1000 characters. Both are recorded in the ledger and the reply (`vanilla_override`, with the status it overrode). Every review, approving or not, records the check it ran (`vanilla_check`).

**Rubric v3.** Every group's questions are asked against the vanilla references in the preview: same form, same medium and palette, same framing (icons), same composition and angle (pictures), and so on. `hoi4.image_families` lists them. Reviews recorded under v1 and v2 stay readable.

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
* Every image runs on `gpt-6-luna`. The route asks for the highest reasoning effort the backend accepts and, when the backend refuses the named efforts, sends the request with no effort so the backend uses its default. The ledger records the effort asked for and the one the backend reported for every image. A response that reports another model, or an effort other than the one asked for, is refused and the image is not used. There is no other model and no override.
* **The route cannot force image quality.** The pixels come from the backend's own image tool, reported as `gpt-image-2-codex`. The backend replaces the tool settings we send with `quality = auto` and `size = auto`, and chooses quality and size itself. One recorded probe returned a medium-quality 1337x1176 image. The ledger records what the backend reported for each image, so check the pixels in the preview.
* A refusal by the provider's safety system is reported as a moderation block, with advice to change the brief or the references.
* **Up to three requests at once.** The route sends at most three requests at a time on your machine, across every process (maintainer decision W5-8 of 2026-11-05, a trial). Set `HOI4_IMAGEGEN_CHATGPT_CONCURRENCY` from 1 to 6 to change it, or `concurrency` under `[providers.chatgpt_login]` in the project config. The variable wins, and 1 means one at a time. Each slot waits at least 20 seconds between its own requests.
* After a sign-out by the server, a 429 or the backend's usage-limit message, the route drops to one request at a time on the whole machine for six hours. Generations, `doctor`, `login-status` and `hoi4.image_status` show a warning with the reason and the time while it holds.
* Spend is recorded as unknown, because there is no per-image price and no usage report.

Details, the sign-in steps and the recorded evidence are in [docs/chatgpt-login.md](chatgpt-login.md).

## Install

The releases are on GitHub: <https://github.com/klimPaskov/hoi4-imagegen-mcp/releases>. Version 0.3.3 has one archive per platform:

| Platform | Archive |
|---|---|
| Windows x64 | `hoi4-imagegen-v0.3.3-x86_64-pc-windows-msvc.zip` |
| macOS arm64 | `hoi4-imagegen-v0.3.3-aarch64-apple-darwin.tar.gz` |
| macOS x64 | `hoi4-imagegen-v0.3.3-x86_64-apple-darwin.tar.gz` |
| Linux x64 | `hoi4-imagegen-v0.3.3-x86_64-unknown-linux-gnu.tar.gz` |
| Linux arm64 | `hoi4-imagegen-v0.3.3-aarch64-unknown-linux-gnu.tar.gz` |

Each archive holds both programs, `LICENSE`, `NOTICE`, `README.md` and `UPSTREAM.md`. The release also has a `SHA256SUMS` file. The binaries are **unsigned**, so Windows SmartScreen and macOS Gatekeeper may warn. The archives and `SHA256SUMS` carry GitHub build attestations. To check a download:

```sh
sha256sum -c SHA256SUMS --ignore-missing
gh attestation verify <archive> --repo klimPaskov/hoi4-imagegen-mcp
```

### With HOI4 Mod Setup

[HOI4 Mod Setup](https://github.com/klimPaskov/HOI4-Mod-Setup) installs the release per project and writes the project-level MCP entries for your coding tools, under the server id `hoi4_imagegen`. From Setup 0.9.2 the image step is one **Install hoi4-imagegen** checkbox, other backends sit under **Use another backend**, and after setup the Ready screen opens the ChatGPT sign-in by itself. It keeps API keys in the credential store.

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

[providers.chatgpt_login]
concurrency = 3                      # requests at once on the machine, 1 to 6 (HOI4_IMAGEGEN_CHATGPT_CONCURRENCY wins)

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

Read [docs/rules-and-terms.md](rules-and-terms.md). It summarises what Paradox, Steam and the image providers say about AI-generated mod assets, and which parts could not be verified. The short version:

* No ban on AI art in HOI4 mods was found, but the general rules apply: keep the mod free, do not infringe, and upload only what you have the right to upload.
* **Disclosing** that a mod uses AI-generated art is recommended. The ledger records provider, model, date and prompt for every asset, and `hoi4.image_ledger` can list them for your mod description.
* **Real people.** Portraits of real historical figures raise personal-rights questions, and the Steam warranty covers them. Providers may refuse such images. Prefer stylised or fictional portraits. The portrait tools warn about this, and a project can forbid a category with `forbidden_families`.
* Assets must be new works in the vanilla style, not near-copies of Paradox art.

## Status and limits

* Verified by tests on Windows, macOS and Linux in CI: see [docs/testing.md](testing.md). The test layers are labelled by evidence kind.
* One signed-in ChatGPT-login probe was recorded on 2026-11-01. Its response is a recorded-real fixture, and faults are injected into it for tests. See [docs/evidence/](evidence/).
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

MIT. See [LICENSE](../LICENSE), [NOTICE](../NOTICE) and [UPSTREAM.md](../UPSTREAM.md).
