<div align="center">

<img src="assets/mascot.png" width="220" alt="GPTImage mascot" />

# GPTImage

**Image generation for Claude Code — powered by GPT Image 2, through your ChatGPT subscription.**

No API key. No per-image bill. You sign in with your ChatGPT account once, and Claude Code can generate & edit images in any project.

</div>

---

> ⚠️ **Grey area, by design.** "Sign in with ChatGPT" is officially meant for Codex. GPTImage reuses that token to reach **GPT Image 2** (ChatGPT Images 2.0). It works and is widely done, but it is *not* an officially sanctioned API. Keep usage personal/local. Heavy use can trigger plan rate-limits (429) or, worst case, account restrictions. You accept that risk by using this.

## What you get

- A **local MCP server** Claude Code talks to (`generate_image`, `image_auth_status`).
- A **Claude Code skill** (`/gptimage`) that teaches the agent when and how to use it.
- A **CLI** for login and one-shot generation.
- **No API key.** No password ever handled by the tool — you log in yourself in the browser.

## Requirements

- Node.js ≥ 20 (tested on v22)
- A ChatGPT account with an active plan (Plus / Pro / etc.)

## Install — one flow

```bash
git clone https://github.com/Connected-Mate/gptimage.git
cd gptimage
npm install
./install.sh
```

`./install.sh` registers the tool **globally** with Claude Code, then opens your
browser to **sign in with ChatGPT**. When you see the success screen, you're done.

> Because the tool is registered at the user level, it works in **every** Claude Code
> project on your machine — you only set it up once. (Restart Claude Code if it was
> already open, so it picks up the new tool.)

Check it any time:

```bash
npm run status     # show plan, account, token expiry
npm run logout     # remove stored credentials
npm run login      # sign in again
```

## Use it in Claude Code

Open Claude Code in any project and just ask:

```
Generate a watercolor red fox in snow and save it to fox.png using the gptimage tool.
```

Claude Code calls `generate_image` and saves the PNG.

### Or from the terminal

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
   │   reads your ChatGPT token from ~/.gptimage/auth.json  (or ~/.codex/auth.json)
   │   refreshes via auth.openai.com/oauth/token if expired
   ▼
POST https://chatgpt.com/backend-api/codex/responses
   model: gpt-5.5, tools:[{ type: image_generation }]  ← GPT Image 2
   headers: Authorization: Bearer <subscription token>, chatgpt-account-id
   ▼
SSE stream → "image_generation_call" → base64 PNG
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
| `GPTIMAGE_MODEL` | Override the subscription model slug (default `gpt-5.5`) if OpenAI rotates it |
| `GPTIMAGE_ORIGINATOR` | Client identifier sent to the backend (default `codex_cli_rs`) |
| `GPTIMAGE_PROJECT_DIR` | Base dir for relative paths (MCP server) |
| `GPTIMAGE_ACCESS_TOKEN` | Provide a token directly (CI / escape hatch) |

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
