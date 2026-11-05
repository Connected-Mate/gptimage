# ChatGPT login

The ChatGPT-login provider makes images on your own ChatGPT plan, with no API key. Decision D-12a (HOI4 Factory, 2026-11-01) makes it a first-class local route:
- It ships in every local build, with no feature flag, config entry or environment variable.
- You sign in once in the browser, and the risk note is shown once.
- Once you are signed in, it is the default local provider. An explicit `provider` in a plan still wins.
- Documented API providers stay available: OpenAI API key, Gemini, fal and ComfyUI.
- It is never part of hosted HOI4 Factory (see "Hosted deployments" below).

## Sign in

```
hoi4-imagegen login
```

1. **The risk note.** It is printed and you type `yes`. Acceptance is recorded once per user, in your own data folder, never in a project. A cloned mod therefore cannot accept it for you. `--accept-risk` skips the question in scripts.
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
- The attempts run one at a time under the route lock and keep the pacing between requests.
- If the backend refuses all four, the call ends with one clear error that lists each refusal, and the route is down until the backend changes. Nothing is retried after that.
- Any other backend error ends the call at once. It is not retried at a lower effort.
- The backend's response events must report the same model (or a dated snapshot of it, `gpt-6-luna-*`) and the effort that attempt asked for. Anything else, or a response that does not say, is refused with a clear error, and the image is never used.
- The effort each image ran at is data on its candidate record in the ledger: `usage.reasoning_effort` (a named effort or `backend default`), `usage.reported_effort` (what the backend said it ran at), `usage.refused_efforts` and the backend's own `usage.backend_usage`. The candidate notes say the same in words ("ran on gpt-6-luna at medium reasoning effort").
- A plan that names another model for this provider is refused at plan time.
- The model and effort the backend reported are recorded with every candidate in the ledger.

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
