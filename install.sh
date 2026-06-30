#!/usr/bin/env bash
# Register the gptimage MCP server and skill with Claude Code.
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SKILL_SRC="$DIR/skill/gptimage"
SKILL_DST="$HOME/.claude/skills/gptimage"

echo "==> Installing npm dependencies"
( cd "$DIR" && npm install --silent )

echo "==> Registering MCP server 'gptimage' (user scope)"
# Remove any prior registration so re-running is idempotent.
claude mcp remove gptimage -s user >/dev/null 2>&1 || true
claude mcp add gptimage -s user -- node "$DIR/src/server.js"

echo "==> Installing the /gptimage skill"
mkdir -p "$HOME/.claude/skills"
if [ -d "$SKILL_SRC" ]; then
  cp -R "$SKILL_SRC" "$SKILL_DST"
fi

echo
echo "==> Tool registered (MCP 'gptimage' + /gptimage skill)."
echo

# Continue straight into the ChatGPT sign-in so setup is one smooth flow.
# Skip with: ./install.sh --no-login
if [ "${1:-}" = "--no-login" ]; then
  echo "Skipping login. When ready:  npm run login"
  exit 0
fi

echo "==> Last step: sign in with your ChatGPT account"
node "$DIR/src/login.js"
