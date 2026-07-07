---
name: gptimage
description: Generate and edit raster images (PNG) from text prompts using the user's ChatGPT subscription — no API key, billed to their ChatGPT plan. Use whenever the user asks to create, generate, draw, make, design, or illustrate an image, picture, logo, icon, illustration, texture, sprite, mockup, background, photo, or visual asset; or to edit/restyle/combine images using reference images. Reference-first: ask for reference images and keep them in a references/ folder before generating. Powered by the gptimage MCP server.
---

# gptimage — image generation via ChatGPT subscription

This skill lets you create images using the **gptimage** MCP server, which calls
the ChatGPT subscription image model through the Codex OAuth backend. No API key is
involved — generation is billed to the user's ChatGPT plan.

## Tools (MCP server `gptimage`)

- **`generate_image`** — create one PNG from a prompt.
  - `prompt` (required): detailed description. Be specific: subject, style, colors, composition, lighting, mood.
  - `out` (required): output path (relative to project dir unless absolute). Existing files are auto-versioned (`-v2`, `-v3`), never overwritten.
  - `quality`: `low` | `medium` | `high` | `auto` (default `high`).
  - `size`: `auto` or `WIDTHxHEIGHT` (e.g. `1024x1024`, `1536x1024`, `1024x1536`). Multiples of 16, max edge 3840, ratio ≤ 3:1.
  - `reference_images`: array of image paths to guide style/subject/composition.
- **`image_auth_status`** — check whether the user is signed in. Call this first if a generation fails with an auth error.

## Reference-first workflow (do this before generating)

**GPT Image 2 is far better at transforming existing images than creating from scratch.** A generation guided by 1–3 reference images beats a text-only prompt almost every time. So before the first `generate_image` call in a project:

1. **Ask the user for reference images.** Style samples, brand assets, competitor screenshots, sketches, mood-board pieces — anything visual that shows what "good" looks like for them.
2. **Store references in a `references/` folder** at the project root. Copy any images the user provides (or that you download) into `references/`, with descriptive names (e.g. `references/brand-palette.png`, `references/hero-style.jpg`). This folder is the project's growing visual memory — reuse it across sessions.
3. **If the user has nothing**, offer to source references yourself: browse Pinterest, Dribbble, Behance, product sites, or whatever fits the project's domain (using the web/browser tools available to you), save the best matches into `references/`, and show them to the user for approval before generating.
4. **Only generate from scratch as a last resort**, when no reference exists and the user declines sourcing. Even then, generate one exploratory image first and treat *it* as the reference to transform in follow-up calls.

## Feedback loop (after generating)

- Tell the user the saved path. If you can view images, read the file to confirm the result matches the request; if it's off, refine the prompt and regenerate.
- **Ask for feedback with screenshots.** Invite the user to drop screenshots of the result in context (in their app, site, deck…) or annotated captures of what's wrong. Save those into `references/` too — each iteration should feed the folder so the next generation has more to transform from.
- When iterating, pass the previous output *and* the feedback screenshots as `reference_images` and describe in the prompt what to keep vs. change.

## How to use it well

1. **Write a strong prompt.** Expand terse requests into a vivid, specific description before calling `generate_image`. Name the art style, palette, lighting, and composition. Don't pass the user's three words verbatim — enrich them.
2. **One image per call.** For several distinct assets (e.g. a set of icons), call `generate_image` once per asset with its own `out` path.
3. **Reference images.** Pass paths (typically from `references/`) in `reference_images` and explicitly state each one's role in the `prompt`, e.g. *"Image 1 is the character to keep; Image 2 is the background style to apply."* This is how you do edits, restyles, and compositing today.
4. **Choose sensible sizes**: square `1024x1024` for icons/logos/avatars, landscape `1536x1024` for banners/headers, portrait `1024x1536` for posters/mobile.

## If not authenticated

If `image_auth_status` reports "not authenticated" or a call returns a 401, tell the user to run, in the `gptimage` project directory:

```
npm run login
```

This opens a browser to sign in with their ChatGPT account. **Never ask for or handle their credentials yourself.** They can also just be logged into the Codex CLI (`codex login`) — the server falls back to those credentials automatically.

## Notes / limits

- This rides the ChatGPT subscription via Codex OAuth (a grey-area but widely-used path). Heavy use can hit plan rate limits (429) or, in the worst case, account restrictions. Keep usage personal and reasonable.
- The model produces raster PNGs. For vector/SVG, icon-system extensions, or anything better built directly in HTML/CSS/canvas, prefer those over `generate_image`.
