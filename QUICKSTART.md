# cc-gpt-image — Quickstart (for end users)

Make Claude Code generate images with your ChatGPT subscription. No API key.

## 1. Install (run once, in your terminal)

```bash
cd cc-gpt-image
npm install
./install.sh
```

`./install.sh` registers the MCP server (`cc-gpt-image`) and the `/gpt-image`
skill with Claude Code.

## 2. Sign in with ChatGPT (run once)

```bash
npm run login
```

Your browser opens OpenAI's **"Sign in with ChatGPT"** page. Log in with your
account. When you see **"Signed in to cc-gpt-image"**, you're done.

Check it worked:

```bash
npm run status
```

## 3. Use it in Claude Code

**Restart Claude Code** (open a new session) so it picks up the new MCP server,
then paste a prompt like:

```
Generate an image of a red fox in snow, watercolor style, and save it to fox.png.
Use the cc-gpt-image MCP tool (generate_image).
```

That's it — Claude Code calls `generate_image` and saves the PNG.

## Quick check from the terminal (optional)

You can also generate without Claude Code, just to confirm everything works:

```bash
npm run gen -- --prompt "a red fox in snow, watercolor" --out fox.png --size 1024x1024
```

## Troubleshooting

| Problem | Fix |
|---------|-----|
| `Not authenticated` | Run `npm run login` again |
| Claude Code doesn't see the tool | Restart Claude Code; check `claude mcp list` shows `cc-gpt-image ✔ Connected` |
| `429 rate limited` | Your ChatGPT plan hit its limit — wait and retry |
| Port 1455 in use during login | Close the other login attempt, then `npm run login` |
