# Recipes

Each recipe is the sequence of tool calls (or `hoi4-imagegen call <tool> '<json>'`). Replace the names.

## 1. Five focus icons with shine sprites from one style

```
hoi4.image_references  {action: add, path: references/oil_a.png, role: style_anchor, note: "own work"}
hoi4.image_references  {action: style_save, style: {name: iron_oil, description: "aged oil painting", prompt_fragments: ["visible brush strokes"], references: [{path: references/oil_a.png, sha256: "", note: ""}], approved_by: "me"}}
for each focus:
  hoi4.image_icon      {family: focus, feature_slug: iron_pact, asset: steel_gate, brief: "...", style: iron_oil}      # free plan
  hoi4.image_icon      {same request, approve: true, idempotency_key: "iron_pact-steel_gate-1"}                          # or plan + generate in one call
  hoi4.image_generate  {plan_id, idempotency_key: "iron_pact-steel_gate-1"}                                              # runs a stored plan
  hoi4.image_process   {candidate_id: "iron_pact.steel_gate-v1"}
  hoi4.image_validate  {targets: ["iron_pact.steel_gate-v1-p1"]}
  hoi4.image_preview   {target: ...}      # look at the native 100x88 panels
  hoi4.image_review    {target, verdict: approved, reviewer, answers}
  hoi4.image_install   {target, approval_id}   # writes focus_steel_gate.dds and the sprite plus _shine snippet
```

## 2. A national spirit set (idea pictures, 60x68)

Same loop with `family: idea_spirit` on `hoi4.image_icon`. Concerns and manufacturers are `idea_concern` (64x64) and advisors are `advisor_card` (65x67, import-only): the size alone cannot make one into another. Use one style for the set and `variants: 3` to pick the best of three candidates per spirit. Install with `mode: register` when the mod has one `.gfx` file per feature.

## 3. A report and a news picture

`hoi4.image_picture` with `family: report_event` (210x176) and `news_event` (397x153, converted to black and white by the chain). Opaque pictures: the chain flattens provider alpha and validation checks that no pixel is translucent. The kit's report-card tool applies the paper, sepia and tilt treatment afterwards; run it on the installed or processed file.

## 4. A flag set

Import a finished 82x52 or larger design: `hoi4.image_process {import: {path, family: flag, feature_slug, asset: "IRP_fascism"}}`. Install writes `gfx/flags/IRP_fascism.tga`, `medium/` and `small/` as bottom-origin 32-bit TGAs; the smaller sizes are Lanczos copies of the first.

## 5. Refining a rejected candidate

```
hoi4.image_review {target, verdict: changes_requested, answers: {silhouette_clear: false, ...}, notes: "the banner is lost at native size"}
hoi4.image_edit   {candidate_id: "iron_pact.steel_gate-v1", instruction: "make the banner larger and brighter red", idempotency_key: "...-edit-1"}
hoi4.image_process / validate / preview / review / install on the new candidate (v2)
```

Version 1 stays on disk and in the ledger; `hoi4.image_ledger {artifact_id, lineage: true}` shows v2 pointing at v1.

## 6. Validating an existing mod's art

`hoi4.image_validate {targets: ["gfx/interface/goals/my_feature/"], family: "focus"}` reads every DDS, TGA and PNG in the folder and reports findings with codes. Files without a ledger record get an advisory `GFX-PROVENANCE-MISSING`.
