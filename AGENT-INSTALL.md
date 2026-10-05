# Install GPTImage with your coding agent

Copy the prompt below and paste it into your coding agent (Claude Code, Codex, Cursor, or any agent that can run terminal commands). It installs GPTImage, connects it to that agent, and checks that it works. You only do one thing yourself: sign in to ChatGPT in your browser when asked.

---

```text
Install GPTImage for me (image-generation MCP server, https://github.com/Connected-Mate/gptimage). Follow these steps exactly, show me each command's result, and stop to ask me if anything fails.

1. Check that Node.js 20 or newer is installed (`node -v`). If it is missing or older, stop and tell me to install it from https://nodejs.org.
2. Clone the project into my home folder (skip the clone if ~/gptimage already exists, and run `git -C ~/gptimage pull` instead):
   git clone https://github.com/Connected-Mate/gptimage.git ~/gptimage
3. Install and register the MCP server for the agent you are (pick the one that matches you):
   - Claude Code:  cd ~/gptimage && ./install.sh --agent claude --no-login --yes
   - Codex:        cd ~/gptimage && ./install.sh --agent codex --no-login --yes
   - Cursor:       cd ~/gptimage && ./install.sh --agent cursor --no-login --yes
   - Any other agent: cd ~/gptimage && ./install.sh --agent none --no-login --yes, then add an MCP server named "gptimage" to your own configuration with command `node` and argument `~/gptimage/src/server.js` (stdio, use the absolute path).
4. Sign-in. Run `cd ~/gptimage && npm run status`.
   - If it says "Authenticated", I am already signed in (GPTImage also reuses a Codex CLI sign-in): continue.
   - Otherwise run `cd ~/gptimage && npm run login` and tell me: "Your browser is opening: please sign in with your ChatGPT account, then come back." Wait for the command to finish. NEVER ask me for my password and never type it yourself.
5. Verify: run `cd ~/gptimage && npm run gen -- -p "a small red paper boat on blue water, flat illustration" -o ~/gptimage-test.png --quality low --size 1024x1024`. It must print the path of the saved PNG. Give me that path so I can look at it.
6. Tell me to restart you (the agent) so the new "gptimage" tool loads. After the restart, call the `image_auth_status` tool once to confirm the connection.
7. Finally, tell me plainly: "GPTImage uses your ChatGPT sign-in, not an API key. It is an unofficial use of that sign-in: keep it personal, heavy use can hit your plan's limits."

Rules: do not use sudo, do not change any other MCP server or setting, do not commit or publish anything, and never print or share the contents of ~/.gptimage/auth.json or ~/.codex/auth.json.
```

---

## What the prompt does

| Step | What happens |
|------|--------------|
| 1–2 | Checks Node.js ≥ 20, downloads GPTImage to `~/gptimage` |
| 3 | `install.sh` installs dependencies and registers the MCP server: `claude mcp add` (Claude Code, + the `/gptimage` skill), `codex mcp add` (Codex), `~/.cursor/mcp.json` (Cursor; existing servers kept) |
| 4 | Reuses an existing sign-in, or opens the ChatGPT sign-in page in **your** browser. The agent never sees a password |
| 5–6 | One low-quality test image, then `image_auth_status` after a restart |
| 7 | Grey-area reminder |

Prefer doing it by hand? See the README: `git clone …`, `npm install`, `./install.sh`.
