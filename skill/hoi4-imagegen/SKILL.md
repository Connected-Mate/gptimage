---
name: hoi4-imagegen
description: Make Hearts of Iron IV graphical assets (40 families: focus, ideas, decisions, technology and event pictures, flags, portraits, counters and more) with the hoi4-imagegen MCP. Use for any new or replaced mod art that is generated or processed rather than hand-drawn. Covers plan, generate, process, validate, preview, review and install.
---

# hoi4-imagegen

Use the `hoi4_imagegen` MCP (tools `hoi4.image_*`). It owns canvas sizes, alpha, margins, DDS or TGA export, naming, versions and provenance. Do not build a second image pipeline and do not write DDS files or sprite names by hand.

## When not to use it
* Art whose second frame is different art: `counter_large`, `counter_small` and `division_emblem` (and frame animation, 3D). Make the frames elsewhere, then bring them under the ledger with `hoi4.image_process` and `import` (a required `family`, one strip or one file per frame in `import.frames`; an image of another family's size or the pixels of an installed asset of another family is refused), and validate and install it here.
* Every other family is generated end to end, including advisor cards, achievements (three DDS), report cards, portraits, flags (three TGA), faction and agency logos and buildings. Advisor cards, achievements and portraits need the mod's copy of the asset skill inside the project (its templates and face model): the plan says what is missing.
* Portraits of real people: the plan warns, providers may refuse, and the Steam warranty covers personal rights. Prefer stylised portraits. A project may forbid the category outright.

## The loop (one asset at a time)
1. `hoi4.image_status`. Check the provider has a key (read from the environment, never typed into chat) and the model is not retired or deprecated.
2. `hoi4.image_families { family }`. Read the group (it names the tool), canvas, margin, evidence, style sources and the review questions. A canvas that differs from the vanilla default is a project choice: say so.
3. References first. Put approved images under `references/`, add them with `hoi4.image_references` (role and rights note), and save a style with `style_save`. Roles: `style_anchor`, `subject_anchor`, `geometry_anchor`. Never invent a source: ask the user for references when style matters.
4. The group tool with a `family`: `hoi4.image_icon`, `_picture`, `_portrait`, `_flag`, `_unit` or `_interface`. Without `approve` it is the free plan. Read the estimate, its basis, the warnings and `approvals`. Tell the user the cost before generating. A plan with the same inputs gives the same id. The family is fixed from here on: pick the right one, an idea icon is not a focus or decision icon.
5. Call the group tool again with `approve: true` and an `idempotency_key` (or `hoi4.image_generate { plan_id, idempotency_key }`). One asset per call. Reuse the key when retrying: it replays instead of paying again. Never retry blindly after an error with retry class `reconcile_first`: use `hoi4.image_cancel` with `reconcile`.
6. `hoi4.image_process { candidate_id }`. The family's own processing runs by itself and cannot be skipped, replaced or shortened: you may add only the pre-steps `chroma_key`, `crop_to_aspect`, `defringe` and `trim_to_subject` (an empty list or any other step is refused). If it says `no_transparency_in_source`, the provider returned an opaque picture: regenerate with a model that has native alpha or use a matte workflow. Do not key a background unless you know it is one flat colour. Portraits use the face model from the project; pass `portrait_crop` only to choose the crop by hand.
7. `hoi4.image_validate { targets: [artifact] }`. The artifact's own family and its full check set always apply. Fix errors, then validate again. A pass is geometry only.
8. `hoi4.image_preview`. **Look at the native-size panels first**, then compare with the reference panels.
9. `hoi4.image_review`. Answer every rubric question honestly. Approve only if all are true, otherwise `changes_requested`, then `hoi4.image_edit` with a precise instruction and go back to step 6.
10. `hoi4.image_install { target, approval_id }`. Default `mode: handoff` writes the sprite into `docs/assets/<feature>/gfx_handoff.md`; the main agent wires `.gfx`.

## What it refuses (and why)
* Install without a passing validation, an approving review of the exact file, and unchanged hashes (file, raw candidate, plan).
* Overwriting any file it did not install. Pick a new asset name.
* Paradox art as a reference unless the project sets `policy.allow_vanilla_references`. Do not copy game art into `references/` to get around it.
* Retired models, unknown models, extra references a model cannot take, and keys sent to unlisted hosts.
* Using one family's asset as another's: `GFX-FAMILY-MISMATCH` (size belongs to another family) and `GFX-FAMILY-REUSE` (same pixels already installed for another family).
* Enlarging an icon more than 1.25 times.
* Fake transparency: an opaque result, or a drawn grey and white checkerboard.

## Where things land
`docs/assets/<feature>/source_png|processed_png|prompts|contact_sheets`, finals in the kit's feature-scoped folders (for example `gfx/interface/goals/<feature>/focus_<asset>.dds`), state in `.hoi4-imagegen/` (config, styles, plans, `ledger.jsonl`, records). Names are lowercase snake_case; flags use the country tag.

## Facts to keep straight
* Focus icons have a paired `_shine` sprite in vanilla. The handoff includes one, marked "pattern observed, game behaviour untested". Do not claim it is required.
* Vanilla canvases (1.19.4.1): focus 100x88, idea_spirit 60x68, advisor_card 65x67, idea_concern 64x64, decision 33x32, report 210x176, news 397x153 (black and white), flags 82x52, 41x26 and 10x7. A mod's own profile (`.tools/gfx/profiles.json`) may choose another accepted size, such as the older 94x86, 64x64 and 32x32 conventions.
* No text in art. Generated text is always wrong.
* In-game appearance is user-operated evidence this tool cannot give. Say "installed", not "verified in game".

## Examples
`examples/` has five focus icons with shine sprites from one style, a national spirit set, a report and a news picture, a three-size flag, and a refinement edit.
