# The opt-in ChatGPT login (unofficial, personal use)

**Read this before turning it on.** This route exists because the upstream project is built on it and some people want to use a ChatGPT subscription they already pay for. It is not a supported OpenAI API.

## What it does

`provider-chatgpt-login` is a Rust port of upstream's `src/auth.js`, `src/login.js` and `src/codex.js` (commit `dfb4948`).

1. **Sign-in.** `hoi4-imagegen chatgpt-login login` runs an OAuth 2.0 authorization-code flow with PKCE against `auth.openai.com`, using the public client id of the official Codex CLI, a loopback redirect on `127.0.0.1:1455`, and the parameters the Codex CLI uses (`codex_cli_simplified_flow`, `originator=codex_cli_rs`). It prints the URL for you to open; it does not open a browser.
2. **Tokens.** Stored in this plugin's own file (default: your user data folder, `hoi4-imagegen/chatgpt-login/auth.json`; mode 0600 on Unix, the per-user folder's default access on Windows). Refreshed tokens are written back to that file only.
3. **Generation.** A `POST` to `chatgpt.com/backend-api/codex/responses` with your access token and account id, presenting itself as `codex_cli_rs`, forcing the hosted `image_generation` tool, and reading the image out of the event stream. The model slug (`gpt-5.5` by default) carries the tool; it is not a stable image model.

## Why it is isolated

* It impersonates the Codex CLI against an **undocumented** endpoint. OpenAI can change or block it at any time, without notice.
* Upstream's own README warns that heavy use can trigger plan rate limits or, in the worst case, account restrictions. The consequences are yours alone.
* OpenAI's Codex authentication page says to treat the credential file like a password and to use Platform API keys for general API calls.
* Whether this use fits OpenAI's Terms of Use was **not verified**: the primary terms pages could not be fetched on 2026-10-31. A secondary source says programmatic extraction of output outside the API is prohibited. Check the current terms yourself.
* Spend is unknown: there is no per-image price and no usage report. It never charges any operator allowance, and records `usage_unknown`.

## What changed from upstream

| Upstream | Here |
|---|---|
| Read `~/.codex/auth.json` as a fallback and wrote rotated tokens back into it | Never touches the Codex CLI's file, so it cannot rotate that tool's refresh token behind its back |
| `GPTIMAGE_ACCESS_TOKEN` environment variable | Removed |
| Identifiers scattered through the code | One block of constants at the top of `src/lib.rs`, each with a warning |
| Auto-opened a browser | Prints the URL |
| The default route | Never a default; documented APIs are the defaults |

## The four gates

All must hold, or the plugin does not register and `hoi4.image_status` says which are missing:

1. Built with `--features unofficial-chatgpt-login` (a source build; the official binaries do not contain it).
2. `[providers.chatgpt_login] enabled = true` in `.hoi4-imagegen/config.toml`.
3. `HOI4_IMAGEGEN_ACCEPT_UNOFFICIAL=1` in **your own environment**. A project file cannot set this, so a cloned mod cannot switch the plugin on for you.
4. Local stdio or command-line mode. It refuses any hosted or networked mode.

## Proof it stays out of default builds

`crates/imagegen-mcp/tests/plugin_isolation.rs` checks that the plugin is a workspace member but not a default member; that `cargo tree` for every default crate lacks it (and shows it with the feature on, as a positive control); that the compiled default binary contains none of its identifying strings (`chatgpt.com`, `auth.openai.com`, `codex_cli_rs`, `backend-api/codex`, in UTF-8 and UTF-16); and that no default crate's source or manifest names its endpoints. HOI4 Factory must repeat the `cargo tree` and string checks on its own server binary.

## Distribution

Source build only. A separately named prebuilt artifact is a later maintainer decision.

## Not exercised

No test or tool in this repository has contacted the backend. The request is reconstructed from upstream's source and checked against synthetic fixtures. Treat it as unverified against the live service.
