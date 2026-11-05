# Trailer sources

Every image the trailer presents as output was made by hoi4-imagegen on the ChatGPT login (plan quota, no money). Each one went through plan, then generate, then process. Wherever the trailer shows finished art, it shows the processed file the game loads, enlarged, so its detail matches vanilla art at the same size. Media files stay out of Git. `assets/` is ignored and rebuilt from the places below. Each asset is listed with the hoi4-imagegen build that made it.

## Vanilla-form regenerations (hoi4-imagegen 0.3.1, 2026-11-05)

The `imagegen-docs` session ran these through its queue on gpt-6-luna at the backend's default effort, because the maintainer asked for HOI4 style everywhere. `node src/pull-regen.mjs <hand-back folder>` copies a hand-back into `assets/regen/<asset>/` and records the family, build, plan id and time in `assets/regen/index.json`. `src/fit-regen.py` writes display copies trimmed to their visible pixels. The raw and processed files stay as the tool wrote them.

| Asset | Family | Plan |
|---|---|---|
| `military_reforms` | decision_category_icon | `plan-f59e32080d863fb8` |

## Regenerations on hoi4-imagegen 0.3.3 (2026-11-05)

The `imagegen-docs` queue made these on 0.3.3 for the families still missing from the cut. `src/pull-regen.mjs` reads their hand-backs in the same way. A building comes as a strip of frames (intact, then damaged), and the trailer shows the first. The focus icons are a later set of twelve, each made with its own three vanilla anchors from different style groups (wreaths, discs, winged badges, diamonds, plain plates), so they do not all copy one plaque. Their hand-backs name each file by the attempt that passed (`-v2`, `-v3`), and `pull-regen.mjs` stores it as `-v1` with the attempt in the index.

| Asset | Family | Plan |
|---|---|---|
| `embassy_flags_handshake_v033` | focus (attempt v3) | `plan-01bf7ce6519b9c45` |
| `signed_treaty_quill_v033` | focus (attempt v2, thumbnail only) | `plan-771dbfa7fc68b7be` |
| `japanese_five_tier_pagoda` | building | `plan-92e0b02d697ab0c3` |
| `laurel_wreath_v033` | medal | `plan-d2eaf370aec79ea0` |
| `platform_suitcase_swap` | operation_picture | `plan-d6dee7394fe110ec` |

## Images chosen from the gallery runs

Some families appear with an image chosen from a later imagegen-docs gallery run instead of their first run. `assets/picked/<family>/` holds each one's raw candidate and processed files, copied from the gallery scratch mod.

| Family | Image | Build | Plan |
|---|---|---|---|
| counter_large | `infantry_rifleman` | 0.3.1 | gallery run |
| division_emblem | `bear_standing` | 0.3.1 | gallery run |
| idea_spirit | `women_welders` | 0.3.1 | gallery run |
| idea_concern | `shell_filling_hall` | 0.3.1 | gallery run |
| commander_trait | `single_tank_side` | 0.3.1 | gallery run |
| officer_corps_spirit | `staff_college_desk` | 0.3.1 | `plan-daa44081f1fbaad3` |
| doctrine | `howitzer_battery_night` | 0.3.2 | `plan-c6e2ffa7a13fe399` |
| raid | `paratrooper_canopy` | 0.3.2 | `plan-35a1a83c8b80534c` |
| special_project_icon | `super_heavy_tank_hall` | 0.3.2 | `plan-bc1ae10144876a28` |
| operation_icon | `telephone_with_bug` | 0.3.2 | `plan-91054bbb04160f55` |

The trailer leaves out the MIO families and agency_upgrade. The 0.3.1 citadel and grand_parliament landmarks and the first TNO run (a Geiger counter that kept the vanilla plaque) are not used.

## The style step

It shows four of The New Order's own focus icons and the focus icon hoi4-imagegen made with them as style references: `geiger_counter`, a hoi4-imagegen 0.3.3 run (`plan-d5819cb50cb45583`) with eight TNO focus icons as references (`assets/tno/refs/`, the run's `result.json` lists each one's path and SHA-256). The step shows references 2, 3, 6 and 7, whose art fills its canvas. The maintainer cleared showing TNO's icons after speaking with the team, with no credit line needed. The build stops until a TNO run is pulled in, so no other mod stands in for it.

## The 40-family gallery (hoi4-imagegen 0.2.2, 2026-11-02)

The `imagegen-docs` session made these for PR klimPaskov/hoi4-imagegen-mcp#5. `docs/gallery/manifest.json` records each image's prompt, model, candidate id, source and processed SHA-256, and installed files. The trailer uses `source_png/` (raw candidates) and `processed_png/` (native finals) for the families whose first run already matched vanilla: the hook, the measured, generate, process, validate, review and install steps, the drop, and the rest of the "many more" grid.

## Portraits

The comparison uses the source photos of the ComfyUI HOI4 portrait workflow's before-and-after showcase (`comfyui-hoi4-portraits/examples/`). The hoi4-imagegen column is a `leader_portrait` run on hoi4-imagegen 0.3.2 with each photo as the subject reference, framed close on the vanilla portrait background: cosgrave `plan-ea9cb10b8cb4bc55` (a rerun with a stronger likeness line), obrien `plan-77f157c5a3313d9c`, mcquaid `plan-00aae284c83f6e90`, mulcahy `plan-6299f308c4729a99` (a rerun that keeps his cap and clean-shaven face), orahilly `plan-e19c07d7da44cfc7`, showcase04 `plan-2cc5b7904aeda302`. The trailer shows each processed 156×210 file. The ComfyUI column is cropped from the showcase, with its arrow strip trimmed. The painted source of orahilly is cropped to the canvas, without the museum's caption strip. Éamon de Valera is left out because the provider's moderation refused that run.

## Animated sprites

The four animated families come from PR klimPaskov/hoi4-imagegen-mcp#6. They play their real frames at the fps each `frameAnimatedSpriteType` registers.

## In-game captures

The W5 run made assets with hoi4-imagegen on 2026-11-01/02 (`docs/evidence/2026-11-02-e2e-all-families.json`, asset ids `w5e2e.*`). The in-game runner checked them in Hearts of Iron IV 1.19.4.1 on the night of 2026-11-02. The trailer shows two of them: the World News window and the agency logo. The W5 decision category icon is left out, because it was made before the vanilla-form fixes and does not follow the category icon's form. The screenshots come from `hoi4-factory/.work/in-game-checks/2026-11-02/` (W5, W5b and run28).

## Logo and links

`assets/logo_imagegen.png` is the painterly logo hoi4-imagegen made for the project (imagegen-docs, logo v2). The community links match the READMEs: Discord, YouTube @ChaosRedux and Ko-fi. `assets/logos/claude.svg` and `assets/logos/openai.svg` are the Claude and OpenAI marks from simple-icons 13.21.0 (`cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/`), shown to name the two clients.

## Sound effects

`assets/sfx/` holds the 19 effects bundled with the HyperFrames media skill, from Pixabay under the Pixabay Content License (free for commercial use, no attribution required). `assets/sfx/CREDITS.md` lists them. `src/config.json` places each cue on the beat grid.

## Music

"Fantasia Fantasia" by Kevin MacLeod (incompetech.com), ISRC USUAN1800006, 118 BPM. Licensed under Creative Commons: By Attribution 4.0 (https://creativecommons.org/licenses/by/4.0/). It is human-composed. The maintainer picked it on 2026-11-02. The YouTube description must carry the attribution:

> "Fantasia Fantasia" Kevin MacLeod (incompetech.com)
> Licensed under Creative Commons: By Attribution 4.0 License
> http://creativecommons.org/licenses/by/4.0/
