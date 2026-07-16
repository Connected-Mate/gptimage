import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import os from "node:os";
import path from "node:path";
import test from "node:test";

const authModule = new URL("../src/auth.js", import.meta.url).href;

function resolveCodexStore(codexHome) {
  const env = { ...process.env };
  if (codexHome === undefined) delete env.CODEX_HOME;
  else env.CODEX_HOME = codexHome;

  return execFileSync(
    process.execPath,
    ["--input-type=module", "--eval", `import { CODEX_STORE } from ${JSON.stringify(authModule)}; process.stdout.write(CODEX_STORE);`],
    { encoding: "utf8", env },
  );
}

test("uses CODEX_HOME for the Codex credential store", () => {
  const codexHome = path.join(os.tmpdir(), "custom-codex-home");
  assert.equal(resolveCodexStore(codexHome), path.join(codexHome, "auth.json"));
});

test("defaults the Codex credential store to ~/.codex", () => {
  assert.equal(resolveCodexStore(), path.join(os.homedir(), ".codex", "auth.json"));
});
