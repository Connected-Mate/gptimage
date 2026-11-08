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
| `hoi4.image_references` | Reference images with role and rights note, project style profiles pinned by SHA-256, and style packs (`pack_list`, `pack_get`, `pack_save`, `pack_validate`, `pack_derive`) |
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
| `hoi4.image_set` | Set mode: plans (and with `generate` makes) every icon of one focus tree file as one set. See [set mode](#set-mode) |

That is 22 tools.

### Set mode

`hoi4.image_set` (`hoi4-imagegen call hoi4.image_set '{"focus_tree": "common/national_focus/GER.txt"}'`) reads one focus tree file with the shared HOI4 engine, the same index and trace as the audit. It keeps the `icon` of every focus in that file and gives every distinct sprite one `focus` asset, so focuses that share a sprite share one asset and nothing is made twice. Each brief names the focus by its English name and description from the mod's localisation, and says the icon is one of a set with distinct subjects.

Every icon gets the same `style_pack`, style anchors (`references`), `provider`, `model` and `quality`, so the set shares one look. The surest way is to generate one icon first, approve it, and pass it as a `style_anchor` for the rest. By default only icons the game cannot show yet (an undefined sprite or a missing texture) are planned, and `only_missing: false` plans every one. Plans are free. With `generate: true` and an `idempotency_key`, each icon is generated under `<idempotency_key>-<asset>`, so a rerun after an interruption repeats nothing that finished. A failed icon is reported and the rest go on.

The reply lists every icon: the sprite the tree names, the asset, its focuses and their names, the plan id, the cost estimate and the sprite name its install registers (`installs_as`). The install names focus sprites `GFX_focus_<asset>`, so where `tree_names_it` is false, point the focus's `icon` at `installs_as`. `shared_sprites` lists the sprites several focuses share, and `dynamic` the icons built at run time, which cannot be planned.

## Families

There are 45 families in seven groups. The group names the tool that plans them.

| Tool | Families |
|---|---|
| `hoi4.image_icon` | focus, idea_spirit, idea_concern, officer_corps_spirit, decision, decision_category_icon, state_modifier, special_project_icon, raid, mio_department, mio_trait, mio_policy, bop_side, medal, agency_upgrade, operation_icon, commander_trait, personal_trait, doctrine |
| `hoi4.image_picture` | report_event, news_event, decision_category_picture, operation_picture, technology, achievement |
| `hoi4.image_portrait` | leader_portrait, commander_portrait, operative_portrait, scientist_portrait, advisor_card |
| `hoi4.image_flag` | flag |
| `hoi4.image_unit` | counter_large, counter_small, division_emblem, building, faction_logo, agency_logo |
| `hoi4.image_interface` | modifier_stat_icon, gui_texture, super_event, model_reference (the input picture of an image-to-3D tool, not game art: install refuses it) |
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
| `doctrine` | `doctrine` | trim, fit 3 px inside the 1 px checked margin (the reach of the dark shadow around an accent, so the glow keeps the margin; 73 of 105 vanilla icons keep 1 px or less), then `doctrine_look`: the units filled black with vanilla's pale outline and glow, warm orange and yellow accents kept with a dark shadow | one DDS |
| `vignette` | `agency_upgrade` | trim, then `round_vignette`: the subject cut to the disc inscribed in its centred square (clear pixels laid over vanilla's median disc colour), its mean saturation lowered to vanilla's median 0.249, its mean value moved to vanilla's median 0.423 to 75th percentile 0.466, gold and bronze tones held to vanilla's 75th percentile 0.218, and the edge darkened to vanilla's profile (0.47 of the inner value at the rim, back to 0.99 nine pixels in), then fit with the 8 px margin. An opaque picture works as a source too | one DDS |
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
| `upright_print` | `report_event`, `operation_picture` with a style pack's `upright_print` | the art fitted to the pack's canvas as it is: no tilt, mask, tone or shadow (see [style packs](#processing-switches)) | one DDS |
| `kept_unit_art` v2 | counters and `division_emblem` with a style pack's `keep_colours` | the art fitted into vanilla's glyph box in its own colours (scaled to the size of the pack's references inside it), `palette_snap` to the references' palette, a counter's NATO box as an outline in the glyph's colour, then `edge_glow`, the references' soft glow | one DDS, or two |
| `filled_icon` | transparent icon families with a style pack's `fill_canvas` | the art cover-resized to the whole canvas, opaque allowed | one DDS |
| `unit_counter` | `counter_large`, `counter_small` | `unit_glyph`: the silhouette in vanilla's green (or the on-map grey) material, then `nato_frame`: the NATO box as frame 2. The on-map counter also gets its text icon | one DDS, or two |
| `division_symbol` | `division_emblem` | `unit_glyph` in vanilla's green material, and the 30x12 small symbol | two DDS |

A style pack's `crisp` switch keeps the recipe and swaps each Lanczos3 fit for an area-filter fit (`area_fit`, `area_cover`), with no defringe.

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
| `building` | `GFX_<x>_icon_small` (province window) and `GFX_<x>_icon` (raid window) | `special_icon = GFX_<asset>_icon_small`, and an entity `building_<building key>` in `gfx/entities` that you add yourself |
| `modifier_stat_icon` | the name a `.gui` gives, upper case like vanilla | `spriteType = "GFX_MODIFIER_<ASSET>"` |

A division emblem's number `<n>` is one past the highest of vanilla's numbers (0 to 43 and 65 to 121) and every number the mod's `interface/*.gfx` files or an earlier install already use. Install records it in the asset record. Achievements and flags have no sprite: the game loads them by file name.

Art that already has its final accepted size is taken as it is when imported, which the chain records as `conform`. Counters and division emblems are stricter: imported art of the final size is kept only when it already is vanilla's material (its frame 1 green or grey as vanilla's, its frame 2 the NATO box). Any other import, at any size, is drawn again from its frame 1 by the unit recipe, and the process result says so in `notes`. The `unit_material` check refuses such art if it reaches validation another way, so every counter and division emblem ends in the unit recipe.

On an opaque source, the unit recipe reads light coloured marks inside a dark shape as holes, the same as on real alpha. A gold bolt on a blue shield therefore stays a bolt cut into the shield. Black, grey and white drawings are not affected.

A technology picture can carry vanilla's tier numeral. Vanilla bakes the numeral into the texture: no `.gui` layer or sprite draws it (the tech tree item in `interface/countrytechtreeview.gui` shows one `GFX_technology_medium` icon). Pass `overlay` to `hoi4.image_process`, for example `{"tier": 2}`, or `{"tier": 3, "gem": true}` for the blue gem that battleship, cruiser and carrier pictures have before the numeral. The tool stamps vanilla's serif numeral in its gold and shadow, in vanilla's place at the top right (`position` moves it). The stamp is a recorded `tech_overlay` step.

Some checks guard a family's look:

* `line_art` (special_project_icon) refuses a filled area larger than 250 px, `GFX-FILLED-INTERIOR`. Vanilla's largest is 171 px and its median 4.
* `unit_material` (counters and division emblem) refuses finished art that is not vanilla's material, `GFX-UNIT-MATERIAL`.
* `unit_glyph_shape` (counters and division emblem) refuses unit art whose glyph is not shaped like one glyph, `GFX-UNIT-GLYPH-SHAPE`. It measures frame 1's visible art (the left frame of a counter strip, the whole image of an emblem) and fails when its aspect is outside the range a glyph can have. Vanilla's glyphs run from 0.81 to 4.64 times as wide as tall on large counters (111 strips), 0.50 to 4.00 on on-map counters (83) and 0.48 to 6.10 on division symbols (101). The limits are 0.6 to 5.0, 0.4 to 4.5 and 0.35 to 6.5, and every vanilla file passes. With `keep_colours` the wide limit is stricter, at most twice the glyph box's aspect and never wider than vanilla's widest (4.1, 4.0 and 3.9). A drawing wider than that is a unit beside its own NATO box or another element, which the fit shrinks into a sliver. The `kept_unit_art` recipe also refuses such a source before it fits it (`unit_art_holds_more_than_the_glyph`).
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
| 4 | Vanilla art from the installed game and its DLC folders (`dlc/<dlc>/gfx/...`, read the same way; a zipped DLC is not read) |

A pinned style profile is tier 1 only when it applies to the family. A plan's `style: <name>` always applies it. Otherwise a profile applies by itself only when its `families` list names the family, or when it has `default: true` (then it covers the families it lists, or every family when it lists none). A profile with an empty `families` list and no `default` is available by name to any family but never applied by itself. `default: false` turns automatic use off. A profile that names the family ranks above a `default: true` profile for all families. The plan says which profile it applied and why.

Tier 4 keeps each family to the vanilla files of its one form. Commander traits leave out the operative traits. operation_icon takes only the 59x58 phase icons and operation_picture only the 210x176 phase pictures, building takes only the 128x64 painted landmark strips, and bop_side leaves out the coloured `_improved` and `_max` state variants. A `gui_texture` takes vanilla references only when the request names its `texture_kind`: `button` (`gfx/interface/button_*` and `gfx/interface/buttons/`), `frame` (`gfx/interface/*_frame*`), `panel` (`gfx/interface/tiles/tiled_*`) or `background` (`gfx/interface/*_bg` of at least 256 px a side). The kind also adds its vanilla form to the prompt.

When the mod's own references (tier 2 or 3) show a form, such as a frame or a background, that form wins over the vanilla form the family rules describe.

**Caller anchors.** References the request sends itself with the role `style_anchor` replace every automatic tier (`style_mode: "replace"`, the default). Only the caller's references are sent, a pinned style stays only when the request names it, and each tier's entry says why nothing of it was sent. The family's form sentence in the prompt gives way to a neutral one (take the frame and form from the anchors), while the size, transparency, margin and readability rules stay. `style_mode: "add"` sends the automatic references beside the anchors instead. The plan's `style_selection.style_mode` says which mode applied. Until 0.3.3 anchors dropped only tier 4, so a mod with the kit's shelf copy sent four shelf files ahead of the anchors.

The first tier with at least 3 usable images is used. A pinned style is always sent, even with fewer images, but then gives no statistics. The plan reports every reference with its tier, reason, SHA-256 and whether it goes to the provider, and the candidates it left out with the reason (`style_sources` in the plan, also in `hoi4.image_families`).

**Statistics.** Each candidate of the used tier is measured (colour, saturation, edge density, alpha, outline share, hue histogram). A fixed k-medoids over an even sample of up to 256 of them picks up to 24 images for the statistics and the drift model. These do not depend on the request, so the measured style stays stable. The statistics become a short style paragraph in the prompt, which goes only with at least one sent automatic reference.

**What is sent (since 0.3.4).** Until 0.3.3 the same k-medoids picked the images to send, so every request in a family got the same few files, nation-specific art included. For tiers 2 to 4 the sent set now depends on the request:

1. **Nation-specific files are left out** unless the brief names the country. A file is nation-specific when its name starts with a country tag (`GER_`), has the tag after the family word (`focus_GER_`, `goal_JAP_`, `idea_SOV_`, and in lower case `focus_jap_` as the kit's shelf copy names them), lies in a tag-named folder (`gfx/leaders/JAP/`), or names a country or its adjective (`focus_generic_attack_romania`). Tags come from the game's `common/country_tags/*.txt` plus a built-in list of the vanilla tags, names from the game's `countries_l_english.yml` plus built-in names of the major powers. A lower-case tag that is also an ordinary word (`cat`, `can`, `car`, `gen`) is not read as a tag. A brief names a country by its tag in capitals or by its name or adjective (`JAP`, `Japan`, `Japanese`). On 1.19.4.1 only 274 of the 2,733 focus files are not nation-specific. Since 0.3.5 the kit's shelf copy (tier 2) counts for a request only when at least 4 of its files suit the brief. Otherwise the request falls through to the next tier instead of getting other countries' art, and the tier's note says so. The mod's own art (tier 3) keeps its tags, because a total conversion's own country files are its style.
2. **The sub-family** (below) limits the pool when the request has one.
3. **About half the slots are theme matches.** The brief's words are matched with each file's name and folder tokens, directly (3 points) and through a built-in synonym table (2 points per shared theme: industry, naval, air, army, politics, research, economy, intelligence, diplomacy, war, defence, infrastructure, agriculture, population, religion). A brief that names a country adds 3 points to that country's files. The picks are drawn from the best matches and must lie within the family's style distribution.
4. **The other slots are style-typical**, drawn from the more typical half of the eligible files (generic files such as `goal_generic_*` first). Each pick takes, of the next six, the one furthest in style from the images already chosen.
5. **A hash of the family, asset and brief seeds every draw.** The same request gives the same set and the same plan. Two different briefs rarely share a set. Over 50 focus briefs on 1.19.4.1 the mean overlap between sets is 0.016, where the fixed set of 0.3.3 gave 1. The mean pairwise spread of the sent images (64-bit dHash bits, frame shape, palette) is 22.9, 34.9 and 0.25, against 23.7, 38.2 and 0.30 for vanilla and 17.7, 20.8 and 0.24 for the 0.3.3 set.

With automatic references (tiers 2 to 4) the prompt adds a line: they show the style only, so the model never copies their frame, emblem, subject or composition and varies them within the family's range. The line is left out beside caller anchors.

**Sub-families (since 0.3.4).** Large families split into kinds: `naval`, `air`, `industry`, `economy`, `research`, `intelligence`, `politics`, `diplomacy`, `society`, `army` and `other`. A file's sub-family is the group its name and folder tokens hit most in the theme table. `crates/imagegen-core/data/subfamilies.json` holds, for the 17 families with at least 60 vanilla files and themed names, each sub-family's file count, example names, style groups (a k-medoids of the style descriptors) and measured form and palette numbers, with a form note written from them. Flags, portraits, advisor cards and achievements are not split. The `subfamilies_data` test regenerates the file from the game and fails when it is stale (`UPDATE_SUBFAMILIES=1` rewrites it).

* A plan or a group tool takes `subfamily`. Without it the brief's words pick one, or none when they match no theme. The plan then warns and uses the whole family.
* References come from that sub-family only. When it has too few eligible files the whole family is used, and `style_sources.subfamily` says so.
* The sub-family's form note goes into the prompt after the family's form sentence, except beside caller anchors.
* The vanilla-match check compares with the sub-family's own spread when it has at least 20 measured vanilla files, else with the whole family, and records which in `compared_with`.
* The plan (`subfamily`) and the candidate record, and so the ledger, keep it. `hoi4.image_families` lists each family's sub-families with counts, examples and form notes.

`style` is part of every family's check set. It warns with `GFX-STYLE-DRIFT` when a candidate is far from the family's style distribution. It is a warning, never an error, and never blocks an install.

**Near-duplicates (since 0.3.4).** Processing and validation compare an output with the project's earlier outputs of the same family: the latest processed artifact of each other asset, up to 200, or the latest raw candidate for a raw one. The measure is a 128-bit dHash of the image over mid grey at its own size, which sees the frame and the placement of the large shapes. The threshold is the family's own: the 1st percentile of the distances between distinct images of the used reference set, at least 6 bits (16 when there is no set). On 1.19.4.1 it is 28 bits for focus, 31 for national spirits, 27 for decisions, 15 for decision categories, 41 for report events, 22 for unit leader traits and 24 for technologies. An output that close to an earlier one gets the warning `GFX-NEAR-DUPLICATE`, naming it and the reference images both used. It is a warning, never an error. The candidate record lists the references each image was made with, and the recorded validation keeps the finding.

Vanilla references may always be sent to a provider (a maintainer decision of 2026-11-01), and the tool never copies a vanilla file into a mod. Vanilla art is left out of the contact sheet that is written into the mod. The vanilla side-by-side sheet (see the review below) goes to the user's cache folder instead.

## Style packs

A mod with a house style of its own needs that style in many families at once, not one style profile at a time. A total conversion such as The New Order (TNO) draws decisions, counters, commander traits, technology and GUI panels as a flat teal terminal, its operations and super events as teal archival photographs, and its events on an upright print or a tall press strip. Vanilla's processing and checks would pull every one of those back to the vanilla look. A **style pack** carries the mod's look for every family it covers, in one named file.

A pack lives in `.hoi4-imagegen/packs/<name>/pack.toml`, with its reference images in the mod, usually in the same folder. It has no authority of its own: it changes which references, prompt text, canvas and finishing a plan uses, never what an agent may do.

```toml
schema = 1
name = "tno"
description = "TNO house style"

[families.decision]
look = "flat teal pictogram on transparent, crisp pixel edges, white and salmon red accents only, no frame and no shading"
form = ["A flat pictogram for a {w}x{h} canvas, drawn in thin teal lines (#4cc6c0) with white and salmon accents, with no frame, plate or shading."]
negative = ["painted texture", "a frame or plate", "gradients"]
processing = { crisp = true }

[[families.decision.references]]
path = ".hoi4-imagegen/packs/tno/decision/tno_decision_ref_1.png"
sha256 = "5f0c..."
note = "imported with the pack from decision/tno_decision_ref_1.png"

[families.report_event]
look = "black and white photograph printed on a slightly tilted white paper print with a soft shadow, upright frame"
form = ["A black and white photograph on a white paper print, slightly tilted, on a transparent background, filling a {w}x{h} upright frame."]
canvas = [175, 204]
processing = { upright_print = true }

[families.gui_texture]
look = "near-black navy panel with 1 px teal and green lines, terminal look, no ornament"

[families.gui_texture.kinds.panel]
look = "a near-black navy panel with a 1 px teal border"

[[families.gui_texture.kinds.panel.references]]
path = ".hoi4-imagegen/packs/tno/gui_texture/tno_gui_texture_ref_3.png"
sha256 = "9a41..."
```

Each family entry holds:

| Key | What it does |
|---|---|
| `references` | Images inside the mod, pinned by SHA-256. A plan checks every hash and refuses a changed file (`pack_reference_changed`) or a missing one (`pack_reference_missing`), as pinned styles do. At most 16 |
| `look` | One line appended to the prompt's style |
| `form` | Form rules that replace the family's vanilla form rules in the prompt, one sentence each (`{w}` and `{h}` are the canvas). Without it the vanilla rules stand |
| `negative` | Negatives that replace the family's vanilla negatives. Without it the vanilla negatives stand |
| `canvas` | The canvas of one frame, replacing the family's (the vanilla size stays accepted) |
| `processing` | Processing switches, off by default except `snap_palette` and `edge_glow`, which follow `keep_colours` (below) |
| `kinds` | `gui_texture` only: `references`, `look`, `form` and `negative` per texture kind (`button`, `frame`, `panel`, `background`). A kind without its own value uses the family's |

**Applying a pack.** A plan's `style_pack: "<name>"` applies a pack, and so does the project default:

```toml
[defaults]
style_pack = "tno"
```

`style_pack: "none"` plans without the default. With a pack that covers the family:

* the pack's references for the family (or the request's texture kind) go to the provider as style anchors, at most 8 and within the model's reference limit. They count as the caller's own anchors: the prompt says their form wins, and with `style_mode` `replace` (the default) no automatic reference goes with them, while `style_mode: "add"` sends the automatic tiers beside them. The sub-family is still inferred and recorded, but the pack decides the references, and its form note stays out when the pack's form replaces the family's,
* the look line goes into the prompt's style, and the form override replaces the vanilla form rules and negatives. A pack form also replaces a GUI texture kind's vanilla form,
* the pack's canvas and processing switches apply to the plan, the candidate, processing, validation, the review and the install. They are part of the effective profile, so its hash ties every step to the pack,
* references the request sends itself with the role `style_anchor` still win: the pack's references are then not sent, and its look, form and switches still apply,
* the plan reports the pack, why it applied (`explicit` or `project_default`), the entry (`decision`, or `gui_texture.panel` for a kind), the look, whether the form and negatives were replaced, the canvas, the switches and each reference with whether it was sent (`style_pack` in the plan, and a tier 0 contribution in `style_selection`).

A family the pack does not cover falls back to the normal tiers above, and the plan says so. A pack entry without references keeps the automatic references, but a pack form drops the vanilla ones. An import (`hoi4.image_process` with `import`) applies the pack's canvas and switches the same way, and takes `style_pack` too.

`hoi4.image_references` manages packs: `pack_list` (with the project default), `pack_get`, `pack_save` (pins every reference, keeps a backup of a replaced file), `pack_validate` and `pack_derive` (below). The validation checks the pinned hashes and the switches, reports each family's reference sizes beside its canvas (and suggests a canvas when three quarters of the references share another size), says which checks and recipe the family then has, and runs the pack-match check of each reference against ranges fitted on the others. On the command line: `hoi4-imagegen pack list`, `pack get <name>`, `pack validate <name>`, `pack derive` and `pack import` (see the [how-to](how-to.md#5-make-a-style-pack-for-a-mod-with-its-own-look)).

### Deriving a pack

`pack derive` (`hoi4.image_references` `pack_derive`) makes a pack from a mod's own art in one step (see the [how-to](how-to.md#5-make-a-style-pack-for-a-mod-with-its-own-look)). It reads only image files under the mod's `gfx/` folder and the installed game, and it needs the game for the near-copy filter. Per family:

| Step | Rule |
|---|---|
| Folder | The family's vanilla folder in the mod: `goals`, `ideas`, `decisions`, `event_pictures`, `leaders`, `technologies`, `counters/divisions_large`, `traits` (top level), `operations`, `achievements`, `super_events` and the top of `interface` for GUI textures. Covered families: `focus`, `idea_spirit`, `decision`, `decision_category_picture`, `report_event`, `news_event`, `leader_portrait`, `technology`, `counter_large`, `commander_trait`, `operation_picture`, `operation_icon`, `achievement`, `super_event` and `gui_texture` |
| Names | Left out: placeholder names (`placeholder`, `unknown`, `missing`, `blank`, `empty`, `dummy`, `todo`, `tbd`, `temp`, `debug`), interface parts outside `gui_texture` (`bg`, `background`, `frame`, `progress`, `bar` and similar), and other families' names (advisor, minister and portrait files in the ideas folder, operative and scientist files among leaders). Operation art needs a `phase` name, as vanilla's does |
| Shared folders | Decisions (sides up to 64) and category pictures (sides from 80). Operation icons (up to 99) and pictures (from 100). In the event folder the largest size cluster is report pictures and the next is news pictures, unless the paths of the larger one name `news` more often |
| Canvas | The dominant size cluster (each side within 25 percent and the aspect within 15 percent of a size), and its most common exact size. Only files in that cluster are measured. The canvas goes into the pack where it differs from the family's |
| Measuring | At most 1,500 files a family, sampled evenly by path. A 64-bit dHash and a style vector of 468 values: the alpha shape at 16x16, the colour layout at 8x8, a saturation-weighted hue histogram, and the saturation, brightness, coverage, aspect, edge density and grey share |
| Filters | Exact duplicates, blank or one-tone canvases, and near-copies of vanilla: within 12 of 64 dHash bits of any vanilla file of the family |
| Ranking | Distance from vanilla's style minus distance from the mod's other files (mean of the 10 nearest, each over its median), so the top is typical of the mod and unlike vanilla. Leader portraits are ranked by typicality alone |
| Picks | k-means (k = the picks, fixed seed) over the top 400, then one pick per cluster in the order of each cluster's best file, at most 2 from one sub-folder and none within 12 dHash bits of another pick. 16 alternates follow the same rules |
| Skips | Fewer than 8 usable files, or mostly vanilla's art: at least 40 percent of the files are near-copies and the rest keep vanilla's palette (the palette part of the style vector is less than 1.45 times vanilla's own spread from vanilla) |
| Draft look | Colour (greyscale, muted, moderate or saturated, with the leading hues), brightness, coverage, flat or detailed texture, the canvas shape where it differs, and saturation against vanilla |
| Switches | `upright_print` when the mod's report or operation pictures have their own canvas or coverage, `in_colour` when its news art is in colour (saturation 0.15 or more and under half grey), `fill_canvas` when its icon art is 90 percent opaque or more and vanilla's is not, `keep_colours` when its unit art's saturation differs from vanilla's by 0.2 or more, and `crisp` when 6 colours cover at least 80 percent of its pixels. A switch that needs a form line gets a draft one |

The references are copied into the mod as PNG (`.hoi4-imagegen/packs/<name>/<family>/<name>_<family>_ref_<n>.png`) and pinned, with the source path and its SHA-256 in each note. `derive.json` beside `pack.toml` records the version (`pack_derive_v1`), the rules, every family's counts, figures, picks and alternates, and the reasons for each skip. It names the mod by its `descriptor.mod` name, never by its path. The contact sheets hold vanilla art, so they go to the cache folder (`pack-derive/` under `HOI4_IMAGEGEN_CACHE_DIR` or the platform cache folder). The measurements are kept there too (`pack-derive/vanilla/`, `pack-derive/mod/` and `pack-derive/scan/`, keyed by each file's path, length and time), so a second run opens only files that changed.

`swap` (`focus:3=a2`, or `focus:3=<path in the mod>`) and `drop` (`focus:3`) change picks after review. A run repeats exactly, so the numbers on a sheet stay valid for the next run. `families` limits the run and keeps the other families of a replaced pack, and `picks` sets the references per family (1 to 16, default 8).

What the measurement cannot see is left to the review: extremist symbols, recognisable real people, and art that belongs to another family. The reply lists those checks under `review`.

**Measured on two total conversions** (HOI4 1.19.4.1, the mods' Workshop files read in place and never copied here):

| Mod | Derived | Skipped, with the reason given | Canvases found | Switches suggested |
|---|---|---|---|---|
| The Fire Rises | 12 families | `counter_large` (35 of 71 near-copies, palette 1.38), `commander_trait` (no art in the folder), `gui_texture` (3 usable files) | focus 170x150, report 500x250, news 400x150, category pictures 102x101, super events 982x594 | report `upright_print`, news `in_colour`, technology `fill_canvas` |
| The New Order | 14 families | `super_event` (no art in the folder) | focus 130x130, report 174x210, news 161x420, technology 239x117, operation pictures 150x150 | counters `keep_colours` and `crisp`, decisions and commander traits `crisp`, technology `fill_canvas` and `crisp`, report and operation pictures `upright_print`, GUI textures and operation icons `fill_canvas` |

These match the packs made by hand for the gallery: the same skipped TFR families, TNO's 239x117 blueprint panels and 161x420 news strips, and the same switches on TNO's counters, decisions and technology. TNO's leader portraits are the closest call (1,102 of 1,479 near-copies, palette 1.47), and they are kept. On the Windows test machine the first run of a mod took 10 to 12 minutes (583 seconds for TFR, 698 for TNO), most of it reading files from a cold disk. A rerun with the same files, for example to apply swaps, took 13 seconds for TNO and gave the same result.

### Processing switches

Each switch is off by default (`snap_palette` and `edge_glow` are on with `keep_colours` unless set to `false`) and applies only to the families named. Each was measured on the references of a mod whose art differs from vanilla processing (TNO's, 8 files per family, read in place and never copied into this repository) and on synthetic images in the tests (`crates/imagegen-mcp/tests/packs.rs`). The validation checks a switch would contradict follow it.

| Switch | Families | What changes | Why (measured) |
|---|---|---|---|
| `keep_colours` | `counter_large`, `counter_small`, `division_emblem` | The `kept_unit_art` recipe: no redraw in vanilla's green (or grey) material. Finished art at the accepted size is kept as it is (`conform`). Other art is fitted into vanilla's glyph box in its own colours, and a counter's frame 2 is the NATO box drawn as an outline in the glyph's colour, with no fill and no shadow. Size, frames and alpha still conform. The `unit_material` check is left out, and the `unit_glyph_shape` check stays with its stricter wide limit | TNO's 8 battalion strips are 152x42 like vanilla's, but their glyph is flat teal (median colour 89, 199, 194) beside an unfilled teal NATO box. None of their 3612 frame-1 glyph pixels has red within 14 levels of blue, which vanilla's material requires, so the vanilla chain would redraw every one in green |
| `upright_print` | `report_event`, `operation_picture` | The `upright_print` recipe: no tilt, mask, sepia tone or shadow. Art with alpha is fitted to the canvas with its own alpha kept, opaque art is cover-resized. The `card_corners` check is left out | 6 of TNO's 8 report pictures are 175x204, upright, with the photograph's print drawn in the art itself on transparency (16 percent of each is transparent around the print), and black and white (median saturation 0.03). The vanilla card would tilt, tone and mask it a second time. TNO's operation pictures are 150x150 opaque archival photographs |
| `fill_canvas` | transparent icon families such as `technology` and `gui_texture` | The `filled_icon` recipe: the art is cover-resized to the whole canvas and may be opaque, and the model is asked for an opaque picture. The margin, fill, opaque-corner and side-view checks are left out | TNO's 8 technology pictures are 239x117 blueprint panels, 93 percent opaque (dark navy with teal lines, median saturation 0.77). The vanilla chain refuses an opaque source (`no_transparency_in_source`), and the side-view check measures vanilla's painted form |
| `snap_palette` | the unit families with `keep_colours`. On by default with `keep_colours`, `false` turns it off, and it is refused without `keep_colours` | Each visible glyph pixel within 20 (CIE76) of a colour of the palette measured on the family's pack references is moved onto that colour (`palette_snap`), its alpha kept, so soft edges stay soft in the snapped colour. A pixel on the line between two palette colours (an anti-aliased border inside the glyph) becomes the matching blend of the two. Colours further from every palette colour are kept. The palette is k-means in Lab on the references' opaque frame-1 pixels (alpha 250 and up), with the smallest count from 1 to 4 at which 95 percent of those pixels lie within 6 of their colour, and colours under 2 percent left out. The step records the palette, the pack and each reference's SHA-256. It runs after the fit and before the outline, which takes the glyph's colour. Without references in the pack the glyph keeps its shades and the process reply says so. Imported finished art (`conform`) is kept as it is | TNO's counter references need one colour, (89, 199, 194), with 95 percent of their 3,612 opaque glyph pixels within 0.75 of it. The 0.3.4 gallery's 7 teal TNO counters drew it 4.1 to 7.3 darker, which put 6 of them in the middle value band and their saturation at 0.586 to 0.679 against the references' 0.555 to 0.558. Snapped, every one is (89, 199, 194). The naval salmon, 83 from that teal, is kept |
| `edge_glow` | the unit families with `keep_colours`. On by default with `keep_colours`, `false` turns it off | The pixels one and two steps (Chebyshev) outside each shape's core (alpha 200 and up) get at least the alpha the pack's references have there, per frame, in the colour of the core next to them (`edge_glow`, after the outline). The step records the glow, the pack and the reference hashes. It is left out when the references show no glow | TNO's references glow: one step outside the glyph alpha 73 and two steps 12, around the NATO box exactly 60 and 18 on all 8. Without it a generated strip had a third of a reference's visible pixels on frame 2, and alpha coverage failed on two thin glyphs (0.102 and 0.115 against at least 0.133). With it they measure 0.238 and 0.247 |
| `crisp` | transparent icon families, and the unit families with `keep_colours` | Every resize uses an area (box) filter (`area_fit`, `area_cover`) instead of Lanczos3, and no defringe erosion runs. The `fringe` check is left out | The icon chain adds no blur filter and no shading: it is a defringe (only when edge pixels carry a key colour), a trim, one resample and, for some families, grayscale. The resample is what changes flat art: a 640 px flat teal pictogram fitted to the 33x32 decision canvas keeps 148 visible pixels off its own colour with Lanczos3 (ringing at hard edges), and none with the area filter. Green line art, which the defringe reads as key colour, is refused by the vanilla chain ("no uncontaminated opaque interior"). Anti-aliasing stays: TNO's decision icons are soft-edged on 44 to 81 percent of their visible pixels |
| `in_colour` | families the vanilla chain turns grey: `news_event` and `special_project_icon` | No `grayscale` step: the art keeps its colour. The vanilla news rules ask for a black and white photograph, so pair it with a form line (`pack_validate` warns without one) | TFR draws its news strips as 400x150 colour photographs, median saturation 0.28 over its 527 strips that are not near-copies of vanilla, against vanilla's 0.00. The picture recipe always greyed them |

**The glyph's size with a pack (since 0.3.6).** With `keep_colours` and the family's references in the pack, the glyph is scaled to the references' median count of opaque frame-1 pixels, inside vanilla's glyph box, which stays the cap (`area_fit` or `fit_in_box` with the smaller box, and a note in the process reply). TNO's 8 counter glyphs span 26 to 62 by 18 to 29 px, so vanilla's 62x30 box is also their envelope, and they hold 342 to 604 opaque pixels (median 451). The gallery's solid APC and truck held 1,118 and 1,188 in the full box, and alpha coverage failed on them once the glow was added (0.415 and 0.398 against at most 0.384). Fitted to the references' size they are 38x19 and 38x17 and pass (0.301 and 0.272). A thin glyph that the box caps is not enlarged: the infantry (21x30, 243 opaque pixels) and the destroyer (62x17, 374) keep their size.

A news strip needs no switch: a pack `canvas = [161, 420]` makes the tall press strip (TNO's 8 news pictures are all 161x420, vanilla's 397x153), and the picture recipe cover-resizes and greys it at that size. Technology needs a form override too: the pack's form and look replace vanilla's painted side-view rule. `pack_validate` warns when `keep_colours`, `upright_print` or `fill_canvas` is on without a form override, because the vanilla rules then still ask for the vanilla form.

### The review with a pack

An artifact made with a pack is measured against the pack's references for its family (and texture kind), not vanilla's, because art in the mod's style rightly fails vanilla's ranges. Measured on TNO's references as raw candidates against the installed game: all 8 report pictures, battalion counters, news strips, technology pictures and operation pictures, and 7 of 8 commander traits, fail the vanilla-match check, while its decision icons and 7 of 8 focus icons pass it. The preview's side-by-side sheet shows the pack's references, and the review records the set it used (`reference_set` in `vanilla_check`: `kind` `style_pack`, the pack, the entry and the rule).

The features and the checked list are the vanilla-match check's, with the exceptions below. A pack has about 8 references a family, too few for percentiles, so the rule `pack_match_v2` takes each feature's minimum to maximum over the pack's references and widens it on each side by that whole width, never narrower than twice the vanilla rule's floor. For a normally distributed feature the range of 8 samples is about 2.85 standard deviations on average, so the accepted span is about 8.5 of them, close to the 9.3 of the vanilla rule (1st to 99th percentile widened by half on each side). It needs at least 3 measurable references. `pack_validate` reports, per family, how many references pass ranges fitted on the others: on the 14 TNO families with at least 4 references that is 104 of 110 references under `pack_match_v2`, as under version 1. The override with a reason stays available and is recorded as before.

**Flat references (since 0.3.6, `pack_match_v2`).** A pack whose references are one flat colour hardly spreads in the colour features, so the floor decides their range, and for a step function like the middle-value share one shade step flips the value from 0 to 1. Each reference is therefore also measured drawn one shade lighter and one shade darker (every channel moved by 16 levels). Where the floor decides a feature's range, its half-width is at least one and a half times the largest change one shade makes to any reference. A flat glyph one shade off then passes, and one three shades off does not. On TNO's counter references one shade moves the middle-value share by up to 0.988, the saturation by 0.048 and the top band's saturation by 0.049, so the saturation range is 0.484 to 0.629 (it was 0.497 to 0.617), and the top band keeps its floor (0.474 to 0.634). The reply's `reference_set` names the step and the margin (`shade_step`, `shade_margin`).

**Unit art against a pack.** For `counter_large`, `counter_small` and `division_emblem` the middle-value share is measured and reported but never fails (`unchecked` in `reference_set`). A kept-colours glyph is one flat colour, so the share is 0 or 1 by which side of the value 0.75 that colour lies. TNO's teal has the value 0.780, its references read 0 to 0.007, and one shade darker reads 1. The 0.3.4 gallery's 8 TNO counters read 0.927 to 1.0 on 6 of them, while they are flat and in the right medium. The saturation and the top band's saturation stay checked: they separate a flat teal glyph from a grey or painted one, and the palette snap puts a few shades off back on the pack's colour (snapped, the gallery's counters measure 0.540 to 0.561). Alpha coverage stays checked too: the two misses were real (a frame 2 without the references' glow) and the glow and the fit to the references' size fix them. On a two-frame strip the glowing NATO box holds most of the visible pixels, so a glyph drawn at half the size is caught by the strip's centre of mass, which leans toward frame 2, rather than by its coverage (see the synthetic tests in `crates/imagegen-mcp/tests/packs.rs`).

**The 8 TNO counters (0.3.4 gallery), before and after.** Before is the 0.3.5 check of the recorded processed file. After is the 0.3.6 chain run on each recorded raw candidate (`pack_look::tests::evaluate_pack_counters`, run by hand on the mod's files, which are only read):

| Counter | 0.3.5 | 0.3.6 |
|---|---|---|
| air assault helicopter | fail: middle-value share 1.0, saturation 0.675, top band 0.677 | pass |
| anti-air halftrack | fail: top band 0.747, saturation 0.673 | pass |
| assault rifle infantry | fail: middle-value share 1.0, alpha coverage 0.102, horizontal centre 0.503 | fail: horizontal centre 0.503 (at most 0.496) |
| cold war MBT | fail: middle-value share 1.0, saturation 0.628 | pass |
| mechanised APC | fail: middle-value share 0.927, ellipse, box and hull fill, saturation 0.620, symmetry | pass |
| motorised truck | fail: middle-value share 0.998, saturation 0.652, box, hull and ellipse fill, top band 0.645 | pass |
| naval destroyer | fail: alpha coverage 0.115 | pass |
| self-propelled artillery | fail: middle-value share 1.0, saturation 0.679, top band 0.681 | pass |

The infantry's remaining miss is real: a standing soldier fills the box's height with 243 opaque pixels, fewer than any reference (342 to 604), so the strip's centre of mass leans toward the NATO box on frame 2. The review's rubric or an override decides it.

## Validation and install

Install is the last step and the strictest.

1. `hoi4.image_validate` runs the family's full check set, never fewer, with stable finding codes (size, alpha, margins, fill, ghosting, chroma fringe, fake checkerboard, frames, DDS or TGA layout, style, and on icons with a side of 40 px or less the `small_detail` warning). The record carries a hash of the check set.
2. The gate compares that hash and audits the processing chain against the family's recipe. A record with a missing step is refused.
3. `hoi4.image_preview` makes the contact sheet and the vanilla side-by-side sheet. `hoi4.image_review` answers the family's rubric and runs the vanilla-match check (next section). Agents may approve. The reviewer's name is recorded with the review.
4. `hoi4.image_install` needs the `approval_id` of an approval of this exact file, and refuses if any hash changed since.
5. The write is atomic: every file of an asset (companions, `.gfx` text or the handoff note) is written together and all are undone if one fails.

**`legacy_lazy_load = no` (since 0.3.5).** The install writes this line into a sprite only where vanilla does. Counted on 1.19.4.1 with all DLC (25,769 sprites, 4,840 with the line), it is on every text icon (`GFX_unit_*_icon_small`, 201 of 201) and every focus `_shine` companion (2,930 of 2,930), which is why the install adds it to the `counter_small` text icon and the shine. A text icon without it drew as a blank next to the "1" in the division tooltip. Every other family is below 1% of its sprites (focus 46 of 3,070, national spirit and idea 2 of 3,742, trait 20 of 231, technology 6 of 2,892, building 0 of 52, and 0 for decisions, medals, portraits, events, counters, emblems, logos and the rest), so none of them gets it. The per-family counts are in `VANILLA_LEGACY_LAZY_LOAD` in `crates/imagegen-core/src/lookup.rs`.

**Landmark entity (since 0.3.5).** A `building` install writes the two icons and no entity. The game also needs an entry named `building_<building key>` under `gfx/entities`, where the key is the building's name in `common/buildings`. Without it error.log says "Building <key> doesn't have an entity" and the landmark cannot be placed. Vanilla's building `landmark_eiffel_tower` has the entity `building_landmark_eiffel_tower` in `gfx/entities/landmarks.asset` (its mesh is in `landmarks.gfx`), plus `building_landmark_eiffel_tower_destroyed`. The install output (`script`), the handoff note and the install receipt say this. The tool does not generate entities.

**Technology tree box (since 0.3.5).** A `technology` install writes the picture only. A technology gets a box in the tech tree only if it belongs to a tree: either a `path` from an existing vanilla technology (which needs a mod copy of that vanilla technology file under `common/technologies`) or the mod's own gridbox in a copy of `interface/countrytechtreeview.gui` (vanilla names its gridboxes like `tech_support_tree`). Without either, the game logs "Found no grid box for tech <key>" and never draws it, whatever its `x` and `y` are. The install output (`script`), the handoff note and the install receipt say this. The tool writes neither the technology file nor the gridbox.

**Technology picture size by tree item (since 0.3.5).** The game picks the tree item from the technology, not from the picture: a technology that enables equipment or sub-units gets the large item of its folder, any other gets the small 72x72 item. `techtree_<folder>_folder_small_item` is 72x72 in every folder, and `techtree_<folder>_folder_item` is 183x84 for the infantry, support, armour, NSB armour and both air folders and 204x72 for the naval, artillery, industry and electronics folders (`interface/countrytechtreeview.gui`). A 146x54 picture in the small item overflows it, so `hoi4.image_plan` takes `canvas` for a technology, limited to three vanilla sizes:

| `canvas` | Item | Technologies | Vanilla pictures of that kind |
|---|---|---|---|
| `[146, 54]` (default) | 183x84 | enables equipment or sub-units, land and air folders | 25 of 114 (64x64: 20, 173x77: 12, 131x52: 5, 123x38: 3) |
| `[64, 64]` | 72x72 | enables neither (or sub-units only) | 138 of 237 (62x62: 38, 60x60: 14, 62x61: 12) and all 18 sub-unit-only pictures |
| `[285, 56]` | 204x72 | enables equipment, naval, artillery and electronics folders | 39 of 64 (64x64: 5, 249x54: 4, 306x56: 2, 286x54: 2) |

Measured on 1.19.4.1 with all DLC: 549 technologies, 428 with a `GFX_<tech>_medium` sprite whose DDS header was read. Overall the equipment pictures are 173 (285x56: 39, 146x54: 26, 173x77: 12, 64x64: 10, 131x52: 5, 249x54: 4). The install note (`script`) says which item the art is sized for and that the game picks the item from the technology. A finished picture at one of the three sizes is kept at that size on import.

`mode: handoff` (default) writes the sprite snippet into `docs/assets/<feature>/gfx_handoff.md` for the main agent to wire. `mode: register` inserts the entries into the suggested `.gfx` file as text, keeping comments and layout. Families without a sprite (achievements and flags) are loaded by the game from the file name, so both modes install the files alone. A final file is replaced only if this tool installed it and it is unchanged (`on_exists: replace`).

**Small icons.** An icon family with a side of 40 px or less (decision, decision category icon, state modifier, raid, commander and personal traits) gets one more rule in its prompt: one bold object of a few large, solid shapes, with no thin parallel lines, lattices, groups of small figures or scene. It also gets the `small_detail` check. The check measures the share of the subject's opaque pixels that lie on its edge, which is high for thin lines and small parts, and warns with `GFX-SMALL-DETAIL` above the family's vanilla 95th percentile (raid 0.468, state modifier 0.364, decision 0.342, commander trait 0.347, personal trait 0.448, decision category icon 0.112). Edge density was measured too, but it is high for every small icon, readable or not, so it does not separate them. The warning never blocks an install. The review decides.

**Portrait backdrops.** Each portrait family has its own vanilla backdrop, measured in the two top corners of its own vanilla files: leaders and commanders a pale cream-grey template (L*a*b* 89.8, -1.9, 8.5), scientists a pale blue-grey one lit from the upper left (66.0, -7.2, -14.4, the `gfx/leaders/scientists` folder), and operatives dark warm charcoal (11.9, -0.1, 5.3) with a soft spotlight behind the figure (La Resistance's 163 operative portraits in the DLC folder). Each family's prompt rule names its own backdrop, its vanilla references and vanilla-match files keep to its own files, and the `backdrop` check warns with `GFX-PORTRAIT-BACKDROP` when the top corners are more than 15 (CIE76) from it, and for an operative also when the side band at two thirds of the height is less than 15 lighter than the corners (vanilla 17.1 to 29.2). The backdrop is not recoloured in processing: telling the backdrop from hair, hats and shoulders is not reliable enough to repaint it safely. A council or otherwise unusual portrait may have another backdrop when the brief asks for it.

Deterministic checks are not a style verdict. Composition and readability at game size go to the review. In-game appearance can only be checked by running the game, which this tool never does. It says "installed", not "verified in game".

## Review and the vanilla-match check

Until 0.3.1 an approval needed only every rubric answer `true`, so a caller that sent blanket `true` answers approved anything, including art of the wrong form. Since 0.3.2 the answers alone never approve.

An approval needs all of these, in this order:

1. a validation of this exact file that ran the family's full check set and passed (`review_needs_validation`),
2. a preview of this exact file (`review_needs_preview`) that put it beside the family's vanilla references (`review_needs_vanilla_preview`),
3. a passing vanilla-match check, which the review runs itself (`review_vanilla_mismatch` when it fails, `review_needs_vanilla_check` when it cannot run, for example without the installed game),
4. every rubric answer `true` (`review_incomplete`).

**The vanilla side-by-side sheet.** For an artifact, `hoi4.image_preview` also makes a second sheet: the candidate under a light bar, then the family's six most typical vanilla images, at native size on the top row and enlarged on the bottom row. It holds vanilla pixels, so it is written outside the mod, to `HOI4_IMAGEGEN_CACHE_DIR` or the platform cache folder (`%LOCALAPPDATA%/hoi4-imagegen/cache` on Windows, `$XDG_CACHE_HOME/hoi4-imagegen` or `~/.cache/hoi4-imagegen` elsewhere). The MCP reply returns it as a second image. The preview record lists every vanilla file shown by path and SHA-256.

**The vanilla-match check.** The family's vanilla art is read from the installed game and its DLC folders, through the same folders and name rules as the vanilla style tier, at the family's vanilla sizes (large medals are cut out of their strips). Each image is measured: palette (lightness, saturation, value, their spread, dark, bright and middle shares, colours, gold tones), outline share, edge density, alpha coverage, soft alpha, background fill, and form (the subject's size, position and aspect, how much of its box, convex hull and inscribed ellipse it fills, left-right symmetry, the saturation of its top band where a ribbon sits), and the lightness and yellow-blue tone of the two top corners, where a portrait shows its backdrop. Each family gets an accepted range per feature from its own vanilla files: the 1st to 99th percentile, widened on each side by half of that width.

Thirteen features can fail a candidate of any family: the form features and the two medium features that separate painted, line and grey art (the middle-value share and the saturation). The others are measured and reported only, because art of the right form spreads as widely in them as art of the wrong form, so a range on them would refuse right art without refusing wrong art. A family whose vanilla art is one consistent palette also has its own palette checks with a narrower range. `agency_upgrade` checks the mean saturation, the mean value and the share of gold and bronze tones, each widened by a quarter of the percentile range instead of a half: saturation 0.053 to 0.471, value 0.179 to 0.586 and gold tones up to 0.476, where vanilla's 15 files run 0.115 to 0.402, 0.239 to 0.518 and 0 to 0.399. With the general widening a saturated, warm painted medallion passed. The four portrait families check the lightness and yellow-blue tone of the backdrop in the two top corners against their own vanilla files (see Portrait backdrops below): leaders and commanders refuse the scientists' blue-grey, scientists refuse the leaders' cream, and operatives refuse both and also check the spotlight behind the figure (the side band at two thirds of the height against the top corners). Vanilla leaders include special dark backdrops, so their check passes a dark one, and the `backdrop` warning names it. A failure names each feature with its value, vanilla's range and vanilla's median. On HOI4 1.19.4.1 between 97.6 and 100 percent of each family's own vanilla files pass its check, and all 15 agency upgrades pass. Their own vanilla files pass at 268 of 271 for leaders and commanders (a regency council, a placeholder and a cut-out portrait fail), 217 of 217 for scientists and 163 of 163 for operatives, base game and DLC. The check is version `vanilla_match_v2`, recorded with every result. Version 1 had no family palette checks.

The check is a gate, not a verdict. Art that looks wrong in a way these features cannot see still passes it, and must then fail the rubric. For example, a loose object where vanilla focus icons have a plaque can be wide, convex and symmetric enough to sit inside vanilla's own spread on every feature. The v3 rubric questions are asked against the vanilla sheet for that reason.

**Families without vanilla art** (`gui_texture`, `super_event` and the animation families) cannot be checked against vanilla. Approve them with an override, or make them with a [style pack](#style-packs) whose references the review then measures against.

**With a style pack** the check measures the file against the pack's references instead of vanilla's, with a rule for small sets (see [the review with a pack](#the-review-with-a-pack)).

**Override.** A deliberate mod style can be approved past a failing or unavailable check with `"override_vanilla_check": true` and an `"override_reason"` of 10 to 1000 characters. Both are recorded in the ledger and the reply (`vanilla_override`, with the status it overrode). Every review, approving or not, records the check it ran (`vanilla_check`).

**Rubric v3.** Every group's questions are asked against the vanilla references in the preview: same form, same medium and palette, same framing (icons), same composition and angle (pictures), and so on. `hoi4.image_families` lists them. Every v3 rubric also starts with `shows_brief_subject`: does the image show the subject the brief asks for, with nothing that contradicts it? No measured check can judge the subject, so an approval needs this answer `true` like the others. The review reply quotes the brief under `subject` (for imported art, which has no brief, the asset's name). Reviews recorded under v1 and v2 stay readable.

## Providers

| Provider | Needs |
|---|---|
| ChatGPT login | `hoi4-imagegen login` once. Uses your own ChatGPT plan. No API key |
| OpenAI API (default gpt-image-2.6-flare, edits gpt-image-2.6-sunburst, 2.5 as fallback) | `OPENAI_API_KEY` |
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
* **"Unable to verify model access".** When the backend answers this (seen three times in three minutes on 2026-11-05, with successes in between), the failed request returns a retryable error and its slot waits 60 seconds before its next request. A second answer within 15 minutes, from any slot or process, drops the route to one request at a time for one hour after that answer, with the reason `model_access_unverified` and the same warning. Two answers more than 15 minutes apart do not. `doctor`, `login-status` and `hoi4.image_status` (`chatgpt_login.route.model_access_unverified`) show the count, the last time and the slots still paused.
* Spend is recorded as unknown, because there is no per-image price and no usage report.

Details, the sign-in steps and the recorded evidence are in [docs/chatgpt-login.md](chatgpt-login.md).

## Install

The releases are on GitHub: <https://github.com/klimPaskov/hoi4-imagegen-mcp/releases>. Version 0.3.6 has one archive per platform:

| Platform | Archive |
|---|---|
| Windows x64 | `hoi4-imagegen-v0.3.7-x86_64-pc-windows-msvc.zip` |
| macOS arm64 | `hoi4-imagegen-v0.3.7-aarch64-apple-darwin.tar.gz` |
| macOS x64 | `hoi4-imagegen-v0.3.7-x86_64-apple-darwin.tar.gz` |
| Linux x64 | `hoi4-imagegen-v0.3.7-x86_64-unknown-linux-gnu.tar.gz` |
| Linux arm64 | `hoi4-imagegen-v0.3.7-aarch64-unknown-linux-gnu.tar.gz` |

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
style_pack = "tno"                   # optional: the style pack every plan uses unless it names another or "none"

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

Project state lives in `.hoi4-imagegen/`: config, styles, style packs (`packs/`), plans, the ledger (`ledger.jsonl`) and records. Working files go in `docs/assets/<feature>/`, as the kit's asset skill describes.

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
* The OpenAI 2.5 and 2.6 per-image prices are unpublished (the 2.6 pages list token rates only), so its estimate uses the `gpt-image-2` row as a labelled proxy.
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
