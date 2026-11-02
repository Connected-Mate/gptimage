# How to: sign in, make a first image, fix problems

This is the short practical guide. The [reference](reference.md) has the full details and the [README](../README.md) is the short tour.

## 1. Sign in (ChatGPT login)

Skip this if you use an API key instead (`OPENAI_API_KEY`, `GEMINI_API_KEY`, `FAL_KEY`, or a ComfyUI endpoint).

```
hoi4-imagegen login
```

1. The risk note is printed. It says the route uses your own ChatGPT plan through a backend OpenAI does not document for other tools, that it can stop working without notice, and that heavy use counts against your plan. Type `yes` to go on. The answer is stored once per user, never in a project.
2. Your browser opens OpenAI's sign-in page. If it does not, open the address that is printed. Your password goes only into OpenAI's page.
3. The tokens are saved in your operating system's credential store: Windows Credential Manager, the macOS login Keychain, or the Linux Secret Service. A Linux machine without a keyring uses a private file in `~/.local/share/hoi4-imagegen/chatgpt-login/`.

Other commands:

* `hoi4-imagegen login-status` shows whether you are signed in, the account and the plan.
* `hoi4-imagegen logout` removes the tokens.
* `hoi4-imagegen login-risk-note` prints the note only. A setup tool can show it and then run `login --accept-risk`.

Once you are signed in, the ChatGPT login is the default provider for this machine. A plan that names a provider uses that one instead.

## 2. Check the setup

```
hoi4-imagegen --project-root path/to/your/mod doctor
```

`doctor` lists the providers (a key is reported as present or not, never shown), the game folder, the profile source and the ledger. The same data comes from the `hoi4.image_status` tool.

## 3. Make a first image

The example is one focus icon. An AI client calls the tools below. Each is also available on the command line as `hoi4-imagegen call <tool> '<json>'`.

1. **Find the family.** `hoi4.image_families { "family": "focus" }` shows the group (`icon`, so the tool is `hoi4.image_icon`), the canvas (100x88 unless the mod overrides it), the checks, the review questions and the style references it would use.

2. **Plan, for free.**

   ```json
   hoi4.image_icon {
     "family": "focus",
     "feature_slug": "iron_pact",
     "asset": "steel_gate",
     "brief": "A heavy steel gate in a mountain pass, dramatic side light, painted style"
   }
   ```

   The reply is a plan: the prompt, the style references with their tier and reason, the provider and model, the cost estimate with its basis, the output paths and a `plan_id`. Read it. Nothing has been spent yet.

3. **Approve.** Send the same request again with approval, or run the stored plan.

   ```json
   hoi4.image_icon { ...the same request..., "approve": true, "idempotency_key": "iron_pact-steel_gate-1" }
   ```

   or `hoi4.image_generate { "plan_id": "...", "idempotency_key": "iron_pact-steel_gate-1" }`. The result is a candidate with the id `iron_pact.steel_gate-v1`. Repeating the same key returns the stored candidate and does not pay again.

4. **Process.** `hoi4.image_process { "candidate_id": "iron_pact.steel_gate-v1" }` runs the family's own chain and returns `iron_pact.steel_gate-v1-p1`. If the background is one flat colour, you may add a pre-step: `"pre_steps": [{"op": "chroma_key", "key": [0, 255, 0]}]`. Only `chroma_key`, `crop_to_aspect`, `defringe` and `trim_to_subject` are allowed.

5. **Validate.** `hoi4.image_validate { "targets": ["iron_pact.steel_gate-v1-p1"] }`. Fix any error and validate again. A pass means the geometry is right. It does not say the art is good.

6. **Preview.** `hoi4.image_preview { "target": "iron_pact.steel_gate-v1-p1" }`. Look at the native-size panel first. Icons are judged at the size the game shows them.

7. **Review.**

   ```json
   hoi4.image_review {
     "target": "iron_pact.steel_gate-v1-p1",
     "verdict": "approved",
     "reviewer": "claude",
     "answers": {
       "readable_at_native_size": true, "silhouette_clear": true, "style_matches_references": true,
       "no_text_or_watermark": true, "no_unintended_elements": true, "period_and_theme_fit": true
     }
   }
   ```

   The icon group has the six questions above. `hoi4.image_families` lists the questions of every group. An approval needs every answer true and a preview of this exact file. An agent may approve, and its name is recorded. The reply carries an `approval_id`. If the art is not good enough, use `"verdict": "changes_requested"` with notes, then `hoi4.image_edit { "candidate_id": "iron_pact.steel_gate-v1", "instruction": "...", "idempotency_key": "..." }` and process the new candidate.

8. **Install.** `hoi4.image_install { "target": "iron_pact.steel_gate-v1-p1", "approval_id": "apr-..." }`. This writes `gfx/interface/goals/iron_pact/focus_steel_gate.dds` and puts the sprite and its `_shine` entry into `docs/assets/iron_pact/gfx_handoff.md`. Add `"mode": "register"` to insert them into the mod's `.gfx` file instead.

Install says "installed", not "verified in game". Start the game yourself to check how the icon looks.

## 4. Fix problems

### Sign-in and provider

| What you see | What it means and what to do |
|---|---|
| `not signed in: run hoi4-imagegen login` | There are no stored tokens. Run `hoi4-imagegen login`, or use another provider |
| `the backend rejected the token (401): sign in again`, or the access token expired with no refresh token | The sign-in is no longer valid. Run `hoi4-imagegen login` again |
| `rate limited by the ChatGPT plan (429)` | You hit your plan's limits. Wait and retry with the same `idempotency_key`, or use an API provider |
| `the provider's safety system refused this image` (a moderation block) | The brief or a reference was refused. Change the brief or the references and plan again. |
| The backend ran another model, another effort, or did not say what it ran | The image was refused and not used. The route accepts only `gpt-6-luna` at high effort and has no fallback. Retry later. If it keeps happening, the backend may have changed, so report it |
| A plan that names a different model for the ChatGPT login | Refused at plan time. Leave `model` out |
| The image looks low quality or has an odd size | The route cannot force image quality. The backend's image tool chooses quality and size. Regenerate, or use an API provider where you set `quality` |
| `retry class reconcile_first` after an error | A call may have gone through. Run `hoi4.image_cancel` with `reconcile: true` before retrying |
| A provider is missing from `doctor` | For an API provider, the key variable is not set and Setup's credential store has none. Set the variable in the environment that starts your AI client |

### Processing and validation

| What you see | What to do |
|---|---|
| `no_transparency_in_source` | The provider returned an opaque picture and the family needs transparency. Regenerate. A `chroma_key` pre-step works only when the background is one flat colour. The tool never invents transparency |
| `GFX-FAMILY-MISMATCH` | The image has the size of another family. Use the right group tool and family, or import art of the correct size |
| `GFX-FAMILY-REUSE` | These pixels are already installed for another family. Make a new image |
| Template missing for an advisor card, achievement or portrait | These chains need the mod's copy of the asset skill inside the project. The message lists the paths searched. Copy the skill's `assets/` and `tools/models/` folders into `.claude/skills/<skill>/`, or point `[templates] dir` and `face_model` in `.hoi4-imagegen/config.toml` at them |
| The canvas is not what you expected | Run `hoi4.image_families { "family": "..." }`. The size is the vanilla default unless the mod's `.tools/gfx/profiles.json` or `[profiles.<family>]` overrides it. Fix the override or the file. `hoi4-imagegen calibrate` compares the table with the game |
| `chain_steps_empty` or `chain_step_not_allowed` | Only four pre-steps are accepted, and an empty list is refused. Leave `pre_steps` out for none |
| `GFX-STYLE-DRIFT` | A warning only. The art is far from the family's style. Look at the preview next to the references |
| Install refuses | The gate names the reason: validation missing or stale, no approving review of this exact file, or a hash that changed. Validate and review again |
| A building has one frame | Frame 2 is the ruined state. Make it with `hoi4.image_edit` on the first candidate and pass it as `second_frame_candidate` to `hoi4.image_process` |

### Windows and keychain

* A path with `..`, a drive letter, a UNC share, a stream (`:`), a device name such as `CON`, or a symlink out of the project is refused. Use project-relative paths.
* The tokens are split over several Credential Manager entries named `hoi4-imagegen.chatgpt-login` because of Windows' size limit. `logout` removes them all.
* On Linux without a running keyring, the tool uses a private file instead. Install a Secret Service (GNOME Keyring or KWallet) if you want the keychain.
* Sign-in uses port 1455 on `127.0.0.1`. If another program holds that port, close it and try again.
* The browser does not open on a remote or headless machine. Open the printed address on a machine that can reach it, or use an API key.
* macOS and Windows may warn that the binary is unsigned. Verify it with `SHA256SUMS` and `gh attestation verify`, then allow it.

### Where things are

* Project state is in `.hoi4-imagegen/`: `config.toml`, `styles/`, plans, `ledger.jsonl` and records. The ledger is an append-only list of every reference, setting, hash, reviewer and output. `hoi4.image_ledger` reads it.
* Working files are in `docs/assets/<feature>/`: source, processed images, prompts, contact sheets and `gfx_handoff.md`.
* The server writes no log file. Its standard error carries messages for a person, and your AI client shows them in its MCP log.
* `HOI4_IMAGEGEN_RECORD_RESPONSES=<folder>` saves each raw ChatGPT-backend response. Use it only to make a fixture, and read the files before sharing them.
* Your sign-in and risk-note record are in your user data folder, never in the project.

## 5. Before you publish a mod

* Disclose that AI-generated art was used. The ledger has provider, model, date and prompt for every asset.
* Do not make likenesses of real people without thinking about personal rights. Prefer stylised or fictional portraits.
* Read [rules and terms](rules-and-terms.md).
