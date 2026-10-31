---
name: hoi4-imagegen
description: Make Hearts of Iron IV graphical assets (focus, idea, decision, tech and event pictures, flags, portraits) with the hoi4-imagegen MCP. Use for any new or replaced mod art that is generated or processed rather than hand-drawn. Covers plan, generate, process, validate, preview, review and install.
---

# hoi4-imagegen

Use the `hoi4_imagegen` MCP (tools `hoi4.image_*`). It owns canvas sizes, alpha, margins, DDS or TGA export, naming, versions and provenance. Do not build a second image pipeline and do not write DDS files or sprite names by hand.

## When not to use it
* Art another kit tool owns: advisor icons (`create_advisor_icon.py`), achievement triplets, report cards, counters, frame animation, 3D. Make them with that tool, then bring the result under the ledger with `hoi4.image_process` and `import`, and validate and install it here.
* Portraits of real people: only through a ComfyUI endpoint the maintainer configured. Never through a hosted API. A project may forbid the category outright.

## The loop (one asset at a time)
1. `hoi4.image_status`. Check the provider has a key (read from the environment, never typed into chat) and the model is not retired or deprecated.
2. `hoi4.image_profiles { id }`. Read the canvas, margin, evidence and the review questions. A canvas that differs from the vanilla default is a project choice: say so.
3. References first. Put approved images under `references/`, add them with `hoi4.image_references` (role and rights note), and save a style with `style_save`. Roles: `style_anchor`, `subject_anchor`, `geometry_anchor`. Never invent a source: ask the user for references when style matters.
4. `hoi4.image_plan`. Read the estimate, its basis, the warnings and `approvals`. Tell the user the cost before generating. A plan with the same inputs gives the same id.
5. `hoi4.image_generate { plan_id, idempotency_key }`. One asset per call. Reuse the key when retrying: it replays instead of paying again. Never retry blindly after an error with retry class `reconcile_first`: use `hoi4.image_cancel` with `reconcile`.
6. `hoi4.image_process { candidate_id }`. Deterministic fit. If it says `no_transparency_in_source`, the provider returned an opaque picture: regenerate with a model that has native alpha or use a matte workflow. Do not key a background unless you know it is one flat colour.
7. `hoi4.image_validate { targets: [artifact] }`. Fix errors, then validate again. A pass is geometry only.
8. `hoi4.image_preview`. **Look at the native-size panels first**, then compare with the reference panels.
9. `hoi4.image_review`. Answer every rubric question honestly. Approve only if all are true, otherwise `changes_requested`, then `hoi4.image_edit` with a precise instruction and go back to step 6.
10. `hoi4.image_install { target, approval_id }`. Default `mode: handoff` writes the sprite into `docs/assets/<feature>/gfx_handoff.md`; the main agent wires `.gfx`.

## What it refuses (and why)
* Install without a passing validation, an approving review of the exact file, and unchanged hashes (file, raw candidate, plan).
* Overwriting any file it did not install. Pick a new asset name.
* Paradox art as a reference unless the project sets `policy.allow_vanilla_references`. Do not copy game art into `references/` to get around it.
* Retired models, unknown models, extra references a model cannot take, and keys sent to unlisted hosts.
* Fake transparency: an opaque result, or a drawn grey and white checkerboard.

## Where things land
`docs/assets/<feature>/source_png|processed_png|prompts|contact_sheets`, finals in the kit's feature-scoped folders (for example `gfx/interface/goals/<feature>/goal_<asset>.dds`), state in `.hoi4-imagegen/` (config, styles, plans, `ledger.jsonl`, records). Names are lowercase snake_case; flags use the country tag.

## Facts to keep straight
* Focus icons have a paired `_shine` sprite in vanilla. The handoff includes one, marked "pattern observed, game behaviour untested". Do not claim it is required.
* Vanilla canvases: focus 100x88, idea 65x67, decision 33x32, report 210x176, news 397x153 (black and white), flags 82x52, 41x26 and 10x7. The kit's older 94x86, 64x64 and 32x32 are conventions, not vanilla.
* No text in art. Generated text is always wrong.
* In-game appearance is user-operated evidence this tool cannot give. Say "installed", not "verified in game".

## Examples
`examples/` has five focus icons with shine sprites from one style, a national spirit set, a report and a news picture, a three-size flag, and a refinement edit.
