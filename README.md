# cc-gpt-image

**Give Claude Code (and other agents) the ability to generate images using your ChatGPT subscription — no API key.**

It signs in with your ChatGPT account via the same OAuth flow the official Codex CLI uses, then calls the subscription image model (`gpt-image` via the Codex backend). Generation is **billed to your ChatGPT plan**, not to a pay-per-image API key.

> ⚠️ **Grey area, by design.** "Sign in with ChatGPT" is officially meant for Codex. Reusing that token to generate images works and is widely done, but it is *not* an officially sanctioned API. Keep usage personal/local. Heavy use can trigger plan rate-limits (429) or, worst case, account restrictions. You accept that risk by using this.

---

## What you get

- A **local MCP server** Claude Code talks to (`generate_image`, `image_auth_status`).
- A **Claude Code skill** (`/gpt-image`) that teaches the agent when and how to use it.
- A **CLI** for login and one-shot generation.
- No API key. No password ever handled by the tool — you log in yourself in the browser.

## Requirements

- Node.js ≥ 20 (tested on v22)
- A ChatGPT account with an active plan (Plus / Pro / etc.)

## Install

```bash
cd cc-gpt-image
npm install
./install.sh        # registers the MCP server + skill with Claude Code
```

## Authenticate

Either sign in directly:

```bash
npm run login       # opens your browser → sign in with ChatGPT
npm run status      # show current auth (plan, account, expiry)
npm run logout      # remove stored credentials
```

…or, if you already use the Codex CLI (`codex login`), do nothing — the server
automatically falls back to `~/.codex/auth.json`.

Credentials are stored at `~/.cc-gpt-image/auth.json` (mode 0600). Tokens are
refreshed automatically when they expire.

## Use it

### In Claude Code
Just ask: *"generate a logo of a fox, save it to assets/fox.png"*. The `/gpt-image`
skill triggers and the agent calls the `generate_image` MCP tool.

### From the CLI
```bash
npm run gen -- --prompt "a red fox in snow, watercolor" --out fox.png --size 1024x1024
npm run gen -- -p "apply Image 1's style to Image 2" -o out.png --ref style.png --ref subject.png
```

## How it works

```
your prompt
   │
   ▼
generate_image (MCP tool)
   │   reads token from ~/.cc-gpt-image/auth.json  (or ~/.codex/auth.json)
   │   refreshes via auth.openai.com/oauth/token if expired
   ▼
POST https://chatgpt.com/backend-api/codex/responses
   model: gpt-5.5, tools:[{ type: image_generation }], tool_choice forced
   headers: Authorization: Bearer <subscription token>, chatgpt-account-id
   ▼
SSE stream → item.type "image_generation_call" → base64 PNG
   ▼
saved to disk (auto-versioned, never overwrites)
```

| File | Role |
|------|------|
| `src/auth.js` | OAuth token storage, refresh, account-id extraction, Codex fallback |
| `src/login.js` | Interactive PKCE login + `--status` / `--logout` |
| `src/codex.js` | The `responses` request + SSE image parsing |
| `src/images.js` | Reference-image encoding + safe PNG saving |
| `src/server.js` | MCP server (stdio) |
| `src/gen.js` | One-shot CLI |

## Configuration (env vars)

| Var | Purpose |
|-----|---------|
| `CC_GPT_IMAGE_MODEL` | Override the subscription model slug (default `gpt-5.5`) if OpenAI rotates it |
| `CC_GPT_IMAGE_ORIGINATOR` | Client identifier sent to the backend (default `codex_cli_rs`) |
| `CC_GPT_IMAGE_PROJECT_DIR` | Base dir for relative paths (MCP server) |
| `CC_GPT_IMAGE_ACCESS_TOKEN` | Provide a token directly (CI / escape hatch) |

## Roadmap

- [ ] `edit_image` — first-class image editing endpoint
- [ ] `inpaint_with_mask` — pixel-precise zone editing via PNG alpha masks
- [ ] Multiple-output / variations in one call
- [ ] Optional OpenAI API-key backend as a fallback when the plan is rate-limited

## Credits / prior art

- [yuji-hatakeyama/opencode-gpt-imagegen](https://github.com/yuji-hatakeyama/opencode-gpt-imagegen) — the OpenCode plugin this mirrors
- [numman-ali/opencode-openai-codex-auth](https://github.com/numman-ali/opencode-openai-codex-auth) — OAuth/PKCE flow reference
- [EvanZhouDev/openai-oauth](https://github.com/EvanZhouDev/openai-oauth) — Codex OAuth proxy reference

Not affiliated with OpenAI. Use at your own risk, in accordance with OpenAI's terms.
