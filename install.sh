#!/usr/bin/env bash
# Install GPTImage and register its MCP server with your coding agent(s).
#
#   ./install.sh                     detect installed agents, register with each, then sign in
#   ./install.sh --agent claude      only Claude Code   (also: codex, cursor, none; repeatable or comma-separated)
#   ./install.sh --no-login          skip the ChatGPT sign-in step (run `npm run login` later)
#   ./install.sh --yes               non-interactive: never prompt (for coding agents running this)
#
# Safe to re-run: each registration replaces the previous GPTImage entry only.
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SERVER="$DIR/src/server.js"
AGENTS=""
LOGIN=1
YES=0

while [ $# -gt 0 ]; do
  case "$1" in
    --agent) AGENTS="$AGENTS,${2:-}"; shift 2 ;;
    --agent=*) AGENTS="$AGENTS,${1#*=}"; shift ;;
    --no-login) LOGIN=0; shift ;;
    --yes|-y) YES=1; shift ;;
    -h|--help) sed -n '2,10p' "$0"; exit 0 ;;
    *) echo "✗ Unknown option: $1 (see ./install.sh --help)"; exit 2 ;;
  esac
done

# Pixel-art welcome — the first thing you see when you run it.
[ -f "$DIR/src/banner.txt" ] && cat "$DIR/src/banner.txt"

NODE_MAJOR="$(node -p 'process.versions.node.split(".")[0]' 2>/dev/null || echo 0)"
if [ "$NODE_MAJOR" -lt 20 ]; then
  echo "✗ GPTImage needs Node.js 20 or newer (found: $(node -v 2>/dev/null || echo none))."
  echo "  Install it from https://nodejs.org, then run ./install.sh again."
  exit 1
fi

# Auto-detect agents when none were named.
if [ -z "${AGENTS//,/}" ]; then
  command -v claude >/dev/null 2>&1 && AGENTS="$AGENTS,claude"
  command -v codex >/dev/null 2>&1 && AGENTS="$AGENTS,codex"
  [ -d "$HOME/.cursor" ] && AGENTS="$AGENTS,cursor"
fi

echo "==> Installing npm dependencies"
( cd "$DIR" && npm install --silent --no-audit --no-fund )

REGISTERED=""
for agent in $(echo "$AGENTS" | tr ',' ' '); do
  case "$agent" in
    claude)
      if ! command -v claude >/dev/null 2>&1; then echo "⚠ Claude Code not found, skipping."; continue; fi
      echo "==> Registering with Claude Code (user scope)"
      claude mcp remove gptimage -s user >/dev/null 2>&1 || true
      claude mcp add gptimage -s user -- node "$SERVER"
      mkdir -p "$HOME/.claude/skills/gptimage"
      cp -R "$DIR/skill/gptimage/." "$HOME/.claude/skills/gptimage/"
      REGISTERED="$REGISTERED Claude-Code"
      ;;
    codex)
      if ! command -v codex >/dev/null 2>&1; then echo "⚠ Codex CLI not found, skipping."; continue; fi
      echo "==> Registering with Codex"
      codex mcp remove gptimage >/dev/null 2>&1 || true
      codex mcp add gptimage -- node "$SERVER"
      REGISTERED="$REGISTERED Codex"
      ;;
    cursor)
      echo "==> Registering with Cursor (~/.cursor/mcp.json)"
      mkdir -p "$HOME/.cursor"
      GPTIMAGE_SERVER="$SERVER" node -e '
        const fs = require("fs"), p = require("os").homedir() + "/.cursor/mcp.json";
        let cfg = {};
        try { cfg = JSON.parse(fs.readFileSync(p, "utf8")); } catch (e) { if (e.code !== "ENOENT") { fs.copyFileSync(p, p + ".bak"); console.log("  (unreadable mcp.json backed up to mcp.json.bak)"); } }
        cfg.mcpServers = cfg.mcpServers || {};
        cfg.mcpServers.gptimage = { command: "node", args: [process.env.GPTIMAGE_SERVER] };
        fs.writeFileSync(p, JSON.stringify(cfg, null, 2) + "\n");'
      REGISTERED="$REGISTERED Cursor"
      ;;
    none|"") ;;
    *) echo "⚠ Unknown agent '$agent' (use claude, codex, cursor or none)." ;;
  esac
done

echo
if [ -n "$REGISTERED" ]; then
  echo "==> Registered with:$REGISTERED. Restart the agent so it loads the new tool."
else
  echo "==> No agent registered. Any MCP client can run:  node \"$SERVER\"   (stdio)"
fi
echo

if [ "$LOGIN" = 0 ]; then
  echo "Skipping sign-in. When ready:  npm run login"
  exit 0
fi

# Already signed in (here or through the Codex CLI)? Reuse it, no second login.
if node "$DIR/src/login.js" --status 2>/dev/null | grep -q "^Authenticated"; then
  echo "==> Already signed in — reusing your existing ChatGPT sign-in:"
  node "$DIR/src/login.js" --status
else
  if [ "$YES" = 1 ] && [ ! -t 0 ]; then
    echo "==> Sign-in needed. A human must run:  cd \"$DIR\" && npm run login"
    echo "    (it opens the browser; you sign in with your own ChatGPT account)"
    exit 0
  fi
  echo "==> Last step: sign in with your ChatGPT account (in your browser)"
  GPTIMAGE_NO_BANNER=1 node "$DIR/src/login.js"
fi

echo
echo "✅ Setup complete. Test it:  npm run gen -- -p \"a small red paper boat, flat illustration\" -o test-image.png --quality low --size 1024x1024"
