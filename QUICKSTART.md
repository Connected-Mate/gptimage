# GPTImage — Quickstart

Make Claude Code generate images with your ChatGPT subscription. Powered by GPT Image 2. No API key.

## 1. Install + sign in (one flow, run once)

```bash
git clone https://github.com/Connected-Mate/gptimage.git
cd gptimage
npm install
./install.sh
```

`./install.sh`:
1. registers the image tool with Claude Code **globally**,
2. opens your browser to **"Sign in with ChatGPT"** — log in with your account.

When you see the **"✅ Signed in to GPTImage — Setup complete"** screen, you're done.

## 2. Use it — in ANY project

Because the tool is registered globally, **every** Claude Code project can use it.
You set it up once.

**Restart Claude Code** (so it loads the new tool), open it in any project, and ask:

```
Generate a watercolor red fox in snow and save it to fox.png using the gptimage tool.
```

That's it.

## Handy commands

```bash
npm run status              # is it signed in? which plan?
npm run login               # sign in again
npm run logout              # remove credentials
npm run gen -- -p "a neon city at night" -o city.png --size 1536x1024   # generate from the terminal
```

## Troubleshooting

| Problem | Fix |
|---------|-----|
| `Not authenticated` | Run `npm run login` |
| Claude Code doesn't see the tool | Restart Claude Code; check `claude mcp list` shows `gptimage ✔ Connected` |
| Browser didn't open at login | The URL is copied to your clipboard — paste it into a browser |
| `429 rate limited` | Your ChatGPT plan hit its limit — wait and retry |
| Port 1455 in use during login | A previous login is still running — `pkill -f login.js`, then retry |
