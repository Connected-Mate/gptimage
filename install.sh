#!/usr/bin/env bash
# Register the cc-gpt-image MCP server and skill with Claude Code.
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SKILL_SRC="$DIR/skill/gpt-image"
SKILL_DST="$HOME/.claude/skills/gpt-image"

echo "==> Installing npm dependencies"
( cd "$DIR" && npm install --silent )

echo "==> Registering MCP server 'cc-gpt-image' (user scope)"
# Remove any prior registration so re-running is idempotent.
claude mcp remove cc-gpt-image -s user >/dev/null 2>&1 || true
claude mcp add cc-gpt-image -s user -- node "$DIR/src/server.js"

echo "==> Installing the /gpt-image skill"
mkdir -p "$HOME/.claude/skills"
if [ -d "$SKILL_SRC" ]; then
  cp -R "$SKILL_SRC" "$SKILL_DST"
fi

echo
echo "Done."
echo "  • MCP server registered: cc-gpt-image  (tools: generate_image, image_auth_status)"
echo "  • Skill installed:        /gpt-image"
echo
echo "Next: authenticate with your ChatGPT account ->  npm run login"
echo "(Or skip it if you're already logged into the Codex CLI.)"
