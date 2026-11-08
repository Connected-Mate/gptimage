# ChatGPT login

The ChatGPT-login provider makes images on your own ChatGPT plan, with no API key. Decision D-12a (HOI4 Factory, 2026-11-01) makes it a first-class local route:
- It ships in every local build, with no feature flag, config entry or environment variable.
- You sign in once in the browser. The risk note is shown as information, and signing in accepts it. There is nothing to type.
- Once you are signed in, it is the default local provider. An explicit `provider` in a plan still wins.
- Documented API providers stay available: OpenAI API key, Gemini, fal and ComfyUI.
- It is never part of hosted HOI4 Factory (see "Hosted deployments" below).

## Sign in

```
hoi4-imagegen login
```

An agent or a setup tool can start the same sign-in with the MCP tool `hoi4.image_login`, with no terminal at all. It opens the browser and returns at once, and the sign-in finishes in the background.

1. **The risk note.** It is printed in the terminal (or returned by `hoi4.image_login`), and the page the browser lands on after the sign-in sums it up. Signing in is the acceptance, so there is no question to answer (a maintainer decision of 2026-11-05 that replaced the typed `yes`). The acceptance is recorded once per user, in your own data folder, never in a project. `--accept-risk` is still accepted and does nothing.
2. **The browser.** It opens OpenAI's sign-in page, and the address is also printed. This is an OAuth 2.0 authorization-code flow with PKCE against `auth.openai.com`, using the public client id of the official Codex CLI and a loopback redirect on `127.0.0.1:1455`. Your password is typed only into OpenAI's page.
3. **The tokens.** They go to your operating system's credential store:
   - Windows: Credential Manager, service `hoi4-imagegen.chatgpt-login`, split across numbered entries because of Windows' size limit
   - macOS: the login Keychain
   - Linux: the Secret Service

   Only Linux without a running keyring falls back to a private file (mode 0600) in `~/.local/share/hoi4-imagegen/chatgpt-login/`. A version 0.1 credential file is moved into the store on first use and deleted.

Other commands:
- `hoi4-imagegen login-status` shows the account and plan.
- `hoi4-imagegen logout` removes the tokens.
- The MCP tool `hoi4.image_status` reports `chatgpt_login.signed_in`.

## The model is fixed

Every image runs on **gpt-6-luna**. This is the maintainer's decision of 2026-11-01, and there is no other model and no model fallback.

The reasoning effort is **the highest one the backend accepts**. On 2026-11-05 the backend began refusing `high` for image requests ("Complexity 'high' is not supported for this image model"), and the maintainer chose this rule (decision W5-6):
- Each request asks for `reasoning.effort = high` first. If the backend refuses that effort, the same request is sent once at `medium`, then once at `low`, and finally once with no `reasoning` field at all, so the backend uses its own default effort. The first rung the backend accepts is used.
- On 2026-11-05 the backend refused high, medium and low (it passes the effort to the image tool as its `complexity`). The maintainer added `low`, and then the request without an effort. An image made that way records `reasoning_effort: "backend default"` and the effort the backend reported. Whether such images are good enough is checked image by image.
- A refused effort is remembered on the machine for a day (`refused_efforts` in `route-state.json`), so later images skip it and send one request. The last effort on the ladder is always tried, so a backend that changes again is noticed. After the day, `high` is tried again.
- The attempts of one image run one after another in that image's slot and keep the slot's pacing (see "Requests at once" below).
- If the backend refuses all four, the call ends with one clear error that lists each refusal, and the route is down until the backend changes. Nothing is retried after that.
- Any other backend error ends the call at once. It is not retried at a lower effort.
- The backend's response events must report the same model (or a dated snapshot of it, `gpt-6-luna-*`) and the effort that attempt asked for. Anything else, or a response that does not say, is refused with a clear error, and the image is never used.
- The effort each image ran at is data on its candidate record in the ledger: `usage.reasoning_effort` (a named effort or `backend default`), `usage.reported_effort` (what the backend said it ran at), `usage.refused_efforts` and the backend's own `usage.backend_usage`. The candidate notes say the same in words ("ran on gpt-6-luna at medium reasoning effort").
- A plan that names another model for this provider is refused at plan time.
- The model and effort the backend reported are recorded with every candidate in the ledger.

## The image model

The sign-in route asks for an image the way ChatGPT does: the request's `image_generation` tool carries no `model`, and ChatGPT draws the picture with its current image model. The route cannot choose one per family. On 2026-11-08 one request named `gpt-image-2.6-sunburst` on the tool and one named `gpt-image-2.6-flare`; the backend accepted both and ignored the field, and the response listed the same image model both times (recorded in `crates/provider-chatgpt-login/tests/fixtures/recorded-real/tool-model-*-ignored-2026-11-08.sse`). So the route keeps the backend's choice and treats it as ChatGPT's latest image model. The model each image ran on is recorded in its candidate note.

## Requests at once

On 2026-11-02 the server revoked every Codex sign-in on the maintainer's account while several sessions generated at once. From then until 0.3.2 the route sent one request at a time. Since 0.3.3 it may send up to three at once on the machine (maintainer decision W5-8 of 2026-11-05, a trial).

- **Slots.** Every request the route makes takes one slot: a generation, a token refresh and the sign-in code exchange. A slot is an operating system lock on a file in the per-user data folder, next to `route-state.json`. Slot 0 is `route.lock`, the same file older versions lock, and slots 1 to 5 are `route.lock.1` to `route.lock.5`. A caller takes the first free slot. When all are busy it waits, and after 10 minutes it gives up with an error that says every slot is in use. Every process on the machine shares the same slots, and a slot is freed when its process ends, even after a crash.
- **How many.** Three by default. Set `HOI4_IMAGEGEN_CHATGPT_CONCURRENCY` to a number from 1 to 6 to change it. A value outside that range is clamped, and 1 gives the old one-at-a-time behaviour. A project can also set it in `.hoi4-imagegen/config.toml`, and the environment variable wins:

  ```toml
  [providers.chatgpt_login]
  concurrency = 2
  ```

  Each process uses its own setting, so keep it the same for every tool on the machine.
- **Pacing.** Each slot waits at least 20 seconds after its own last request. While the route runs one at a time, the gap counts from the last request of any slot, as before.
- **Backoff.** A 429 starts a backoff of 30 seconds that doubles with every further 429, up to 5 minutes. It holds every slot.
- **Fall back to one at a time.** A sign-out by the server (a 401, or an answer that says `token_revoked`), a 429, or the backend's usage-limit message drops the route to one request at a time on the whole machine. The reason and the time are recorded in `route-state.json` (`fallback`), so every process sees it. A caller that already holds a higher slot gives it up before its next request and waits for slot 0.
- **When it ends.** Six hours after the last sign-out or rate limit. Every new one restarts the six hours. A new sign-in does not end it early.
- **"Unable to verify model access".** Between 19:44 and 19:47 UTC on 2026-11-05 the backend answered "Unable to verify model access right now" three times across parallel slots, with successful requests in between. It comes as an error event in the stream or in an HTTP error body, and the match ignores case. It is a temporary condition, not a terminal bad request:
  - The first answer fails that request with a retryable error. Its slot waits 60 seconds before its next request (`slot_pause` in `route-state.json`). The other slots keep running.
  - A second answer within 15 minutes of the last one, from any slot or process, drops the route to one request at a time with the reason `model_access_unverified`, through the same mechanism and warning as above. It lasts one hour after that answer, not six, because the message looked like a passing hiccup and not a sign-out or a plan limit. Every further answer within 15 minutes of the last restarts the hour. It never cuts a longer fall back short.
  - The count and the time of the last answer are recorded in `route-state.json` (`model_access`) and shown by `doctor`, `login-status` and `hoi4.image_status` (`chatgpt_login.route.model_access_unverified`, with the slots still paused).
- **The warning.** While it holds, every generation from this route says so, in the image's notes or in the error. `hoi4-imagegen doctor`, `hoi4-imagegen login-status` and `hoi4.image_status` (`chatgpt_login.route.fallback` and `chatgpt_login.route.concurrency`) show it too. It reads like this:

  ```
  warning: the ChatGPT-login route dropped to one request at a time on this machine after a rate limit (http_429) at 2026-11-05T10:00:00Z; it goes back to several at a time after 2026-11-05T16:00:00Z if no new sign-out or rate limit arrives
  ```

  After two "Unable to verify model access" answers it reads:

  ```
  warning: the ChatGPT-login route dropped to one request at a time on this machine after the backend failing twice within 15 minutes to verify model access (model_access_unverified) at 2026-11-05T19:46:00Z; it goes back to several at a time after 2026-11-05T20:46:00Z if no new model-access error arrives
  ```
- **The state file.** Several slot holders write `route-state.json` at once, so every read and every change of it happens under a short lock on `route-state.lock`. The file keeps the last request of each slot (`slot_last_request`). A file written by 0.3.2 or older still reads: its single `last_request_epoch` paces every slot until the slots have their own times.

## What to know before using it

- It presents itself as the Codex CLI to a backend that OpenAI does not document for other tools. It can change or be blocked without notice.
- Heavy use counts against your plan's limits. In the worst case OpenAI could restrict the account. The consequences are yours.
- Whether this use fits OpenAI's terms has not been confirmed; see [rules and terms](rules-and-terms.md). OpenAI's guidance is to use Platform API keys for general API calls.
- Spend is recorded as unknown, since there is no per-image price or usage report. It never charges an operator allowance.

## What changed from upstream (Connected-Mate/gptimage `dfb4948`)

| Upstream | Here |
|---|---|
| Read `~/.codex/auth.json` as a fallback and wrote rotated tokens back into it | Never touches the Codex CLI's credentials, so it cannot rotate that tool's refresh token behind its back |
| Plain JSON token file | Operating system credential store |
| `GPTIMAGE_ACCESS_TOKEN` and `GPTIMAGE_MODEL` environment variables | Removed; the model is fixed |
| No check of what the backend ran | The model and effort must be echoed, or the image is refused |
| Identifiers scattered through the code | One block of constants at the top of `crates/provider-chatgpt-login/src/lib.rs` |

## Hosted deployments

`RunMode::Hosted` never registers the provider. `imagegen_mcp::plugin::refuses_hosted(RunMode::Hosted)` is `true`, and `crates/imagegen-mcp/tests/plugin_gates.rs` checks that a hosted engine cannot select it, even signed in and named in the project file. HOI4 Factory's S06 hosted tool profile depends on that test, and S06 adds its own profile test on the Factory side.

## Recorded responses

Set `HOI4_IMAGEGEN_RECORD_RESPONSES=<folder>` to save each raw backend response. The fixtures under `crates/provider-chatgpt-login/tests/fixtures/recorded-real/` were made this way. They are reviewed before commit and labelled "recorded-real replay", and faults are injected into them for CI.

## What the backend controls

The first signed-in probe was on Windows on 2026-11-01. Its fixture is `crates/provider-chatgpt-login/tests/fixtures/recorded-real/focus-luna-high-2026-11-01.sse`, and the evidence is `docs/evidence/2026-11-01-luna-probe.json`.
- The backend ran the request on `gpt-6-luna` with `reasoning.effort = high`, as both `response.created` and `response.completed` report.
- The pixels come from the backend's hosted image tool, which the response names `gpt-image-2-codex`.
- The backend replaces the tool settings we send with `quality = auto` and `size = auto`. The probe image came back at quality `medium`, 1337x1176, with a transparent background.

So the route forces and verifies the outer model and effort, but it cannot choose the image tool's model, quality or size. The ledger records what the backend reported for every image.
