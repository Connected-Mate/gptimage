#!/usr/bin/env node
// Interactive "Sign in with ChatGPT" — OAuth + PKCE, same flow as the Codex CLI.
// Opens your browser, you log in with your own ChatGPT account, and the resulting
// subscription token is stored at ~/.cc-gpt-image/auth.json.
//
//   node src/login.js            log in
//   node src/login.js --status   show current auth
//   node src/login.js --logout   delete stored credentials

import http from "node:http";
import { randomBytes, createHash } from "node:crypto";
import { spawn } from "node:child_process";
import fs from "node:fs/promises";
import {
  CLIENT_ID,
  AUTHORIZE_URL,
  TOKEN_URL,
  REDIRECT_URI,
  SCOPE,
  OUR_STORE,
  CODEX_STORE,
  loadAuth,
  accountIdFromToken,
  planFromToken,
  writeOurStore,
} from "./auth.js";

const CALLBACK_PORT = 1455;
const CALLBACK_PATH = "/auth/callback";

function base64url(buf) {
  return buf.toString("base64url");
}

function makePkce() {
  const verifier = base64url(randomBytes(32));
  const challenge = base64url(createHash("sha256").update(verifier).digest());
  return { verifier, challenge };
}

function buildAuthorizeUrl(challenge, state) {
  const url = new URL(AUTHORIZE_URL);
  url.searchParams.set("response_type", "code");
  url.searchParams.set("client_id", CLIENT_ID);
  url.searchParams.set("redirect_uri", REDIRECT_URI);
  url.searchParams.set("scope", SCOPE);
  url.searchParams.set("code_challenge", challenge);
  url.searchParams.set("code_challenge_method", "S256");
  url.searchParams.set("state", state);
  url.searchParams.set("id_token_add_organizations", "true");
  url.searchParams.set("codex_cli_simplified_flow", "true");
  url.searchParams.set("originator", "codex_cli_rs");
  return url.toString();
}

function openBrowser(url) {
  const cmd = process.platform === "darwin" ? "open" : process.platform === "win32" ? "start" : "xdg-open";
  try {
    spawn(cmd, [url], { stdio: "ignore", detached: true, shell: process.platform === "win32" }).unref();
  } catch {
    /* user opens it manually */
  }
}

async function exchangeCode(code, verifier) {
  const res = await fetch(TOKEN_URL, {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({
      grant_type: "authorization_code",
      client_id: CLIENT_ID,
      code,
      code_verifier: verifier,
      redirect_uri: REDIRECT_URI,
    }),
  });
  if (!res.ok) {
    throw new Error(`code->token exchange failed: ${res.status} ${(await res.text().catch(() => "")).slice(0, 300)}`);
  }
  const json = await res.json();
  if (!json?.access_token) throw new Error("token response missing access_token");
  return json;
}

const SUCCESS_HTML = `<!doctype html><html><head><meta charset="utf-8"><title>Signed in</title>
<style>body{font-family:-apple-system,system-ui,sans-serif;background:#0a0a0a;color:#ededed;display:grid;place-items:center;height:100vh;margin:0}
.card{text-align:center}.check{width:56px;height:56px;border-radius:50%;background:#16a34a;display:grid;place-items:center;margin:0 auto 20px}
.check svg{width:30px;height:30px;stroke:#fff;stroke-width:3;fill:none}h1{font-size:20px;margin:0 0 8px}p{color:#a1a1aa;margin:0}</style></head>
<body><div class="card"><div class="check"><svg viewBox="0 0 24 24"><path d="M5 13l4 4L19 7"/></svg></div>
<h1>Signed in to cc-gpt-image</h1><p>You can close this tab and return to your terminal.</p></div></body></html>`;

async function login() {
  const { verifier, challenge } = makePkce();
  const state = randomBytes(16).toString("hex");
  const authorizeUrl = buildAuthorizeUrl(challenge, state);

  const result = await new Promise((resolve, reject) => {
    const server = http.createServer((req, res) => {
      const url = new URL(req.url, `http://localhost:${CALLBACK_PORT}`);
      if (url.pathname !== CALLBACK_PATH) {
        res.writeHead(404).end("not found");
        return;
      }
      const code = url.searchParams.get("code");
      const returnedState = url.searchParams.get("state");
      const error = url.searchParams.get("error");
      res.writeHead(error ? 400 : 200, { "Content-Type": "text/html" });
      res.end(error ? `<h1>Login error: ${error}</h1>` : SUCCESS_HTML);
      server.close();
      if (error) return reject(new Error(`authorization error: ${error}`));
      if (!code) return reject(new Error("no authorization code in callback"));
      if (returnedState !== state) return reject(new Error("state mismatch (possible CSRF) — aborting"));
      resolve(code);
    });
    server.on("error", reject);
    server.listen(CALLBACK_PORT, "127.0.0.1", () => {
      console.log("\n  Sign in with your ChatGPT account in the browser window that just opened.");
      console.log("  If it didn't open, paste this URL manually:\n");
      console.log("  " + authorizeUrl + "\n");
      openBrowser(authorizeUrl);
    });
  });

  const tokens = await exchangeCode(result, verifier);
  const access = tokens.access_token;
  const record = {
    access,
    refresh: tokens.refresh_token ?? null,
    accountId: accountIdFromToken(access) ?? null,
    expires: tokens.expires_in ? Date.now() + tokens.expires_in * 1000 : null,
  };
  await writeOurStore(record);
  const plan = planFromToken(access);
  console.log(`  ✓ Signed in${plan ? ` (plan: ${plan})` : ""}. Credentials saved to ${OUR_STORE}`);
}

async function status() {
  const record = await loadAuth();
  if (!record) {
    console.log("Not authenticated. Run `npm run login`.");
    return;
  }
  const plan = planFromToken(record.access);
  const exp = record.expires ? new Date(record.expires).toISOString() : "unknown";
  const sourceLabel = record.store === CODEX_STORE ? "Codex CLI (~/.codex/auth.json)" : record.store;
  console.log(`Authenticated via: ${sourceLabel}`);
  console.log(`  plan:       ${plan ?? "unknown"}`);
  console.log(`  account id: ${record.accountId ?? "unknown"}`);
  console.log(`  expires:    ${exp}`);
}

async function logout() {
  try {
    await fs.unlink(OUR_STORE);
    console.log(`Removed ${OUR_STORE}.`);
  } catch {
    console.log("No cc-gpt-image credentials to remove.");
  }
  console.log("(Codex CLI credentials at ~/.codex/auth.json were left untouched.)");
}

const arg = process.argv[2];
try {
  if (arg === "--status") await status();
  else if (arg === "--logout") await logout();
  else await login();
} catch (err) {
  console.error("\n  ✗ " + (err?.message || err));
  process.exit(1);
}
