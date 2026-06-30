---
name: gpt-image
description: Generate and edit raster images (PNG) from text prompts using the user's ChatGPT subscription — no API key, billed to their ChatGPT plan. Use whenever the user asks to create, generate, draw, make, design, or illustrate an image, picture, logo, icon, illustration, texture, sprite, mockup, background, photo, or visual asset; or to edit/restyle/combine images using reference images. Powered by the cc-gpt-image MCP server.
---

# gpt-image — image generation via ChatGPT subscription

This skill lets you create images using the **cc-gpt-image** MCP server, which calls
the ChatGPT subscription image model through the Codex OAuth backend. No API key is
involved — generation is billed to the user's ChatGPT plan.

## Tools (MCP server `cc-gpt-image`)

- **`generate_image`** — create one PNG from a prompt.
  - `prompt` (required): detailed description. Be specific: subject, style, colors, composition, lighting, mood.
  - `out` (required): output path (relative to project dir unless absolute). Existing files are auto-versioned (`-v2`, `-v3`), never overwritten.
  - `quality`: `low` | `medium` | `high` | `auto` (default `high`).
  - `size`: `auto` or `WIDTHxHEIGHT` (e.g. `1024x1024`, `1536x1024`, `1024x1536`). Multiples of 16, max edge 3840, ratio ≤ 3:1.
  - `reference_images`: array of image paths to guide style/subject/composition.
- **`image_auth_status`** — check whether the user is signed in. Call this first if a generation fails with an auth error.

## How to use it well

1. **Write a strong prompt.** Expand terse requests into a vivid, specific description before calling `generate_image`. Name the art style, palette, lighting, and composition. Don't pass the user's three words verbatim — enrich them.
2. **One image per call.** For several distinct assets (e.g. a set of icons), call `generate_image` once per asset with its own `out` path.
3. **Reference images.** When the user provides reference images, pass their paths in `reference_images` and explicitly state each one's role in the `prompt`, e.g. *"Image 1 is the character to keep; Image 2 is the background style to apply."* This is how you do edits, restyles, and compositing today.
4. **After generating**, tell the user the saved path. If you can view images, read the file to confirm the result matches the request; if it's off, refine the prompt and regenerate.
5. **Choose sensible sizes**: square `1024x1024` for icons/logos/avatars, landscape `1536x1024` for banners/headers, portrait `1024x1536` for posters/mobile.

## If not authenticated

If `image_auth_status` reports "not authenticated" or a call returns a 401, tell the user to run, in the `cc-gpt-image` project directory:

```
npm run login
```

This opens a browser to sign in with their ChatGPT account. **Never ask for or handle their credentials yourself.** They can also just be logged into the Codex CLI (`codex login`) — the server falls back to those credentials automatically.

## Notes / limits

- This rides the ChatGPT subscription via Codex OAuth (a grey-area but widely-used path). Heavy use can hit plan rate limits (429) or, in the worst case, account restrictions. Keep usage personal and reasonable.
- The model produces raster PNGs. For vector/SVG, icon-system extensions, or anything better built directly in HTML/CSS/canvas, prefer those over `generate_image`.
