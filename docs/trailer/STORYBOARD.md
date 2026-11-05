---
message: "Describe the asset. Get a finished, installed HOI4 file."
destination: youtube
aspect: 1920x1080
length: about 112s (55 bars of the track)
flow: automation
storyboard: yes
mode: autonomous
revision: 4 (the cut as built, after the 5 Nov review)
---

# hoi4-imagegen-mcp trailer: storyboard, revision 4

**One thing to remember:** an agent describes an asset, and hoi4-imagegen returns a finished file the game loads, with the vanilla size, the vanilla finish and the `.gfx` entry.

**Real material rule.** Every image shown as output was made by hoi4-imagegen itself (plan, then generate, then process) and has a plan id or gallery record (see `SOURCES.md`). Other art appears only labelled as a style reference or a comparison. In-game shots come from the 2026-11-02 runs. Every image follows its vanilla family's form, and a family whose form is not right yet is left out until its regeneration lands (`hold` in `src/config.json`).

## Music and beat grid

"Fantasia Fantasia" by Kevin MacLeod, measured at 117.9937 BPM in 4/4, cut in at 1.012s of the track. `src/config.json` holds the grid, the scene order in bars and the sound-effect cues. `node src/build.mjs` turns it into `index.html` and `compositions/`, with a steel dip at each scene change. Every cut and hit lands on a beat.

| Bars | Scene | Picture |
|---|---|---|
| 1-2 | Hook | Finished art drifts past the hero faction logo. "Made by an agent. Ready for the game." |
| 3-4 | The old way | A 2062×763 raw output slams against the game's 146×54 slot. The checklist (size, DDS, `.gfx` sprite, sprite name) strikes through one line per beat while the frame drifts in. |
| 5-6 | Reveal | Two steel plates slam together and the name extrudes: `hoi4-imagegen-mcp`, "Rust MCP server + CLI · 19 tools · 40 asset families". |
| 7-28 | The pipeline | Ten stations on a camera rail, two bars each and four for the last (below). |
| 29-30 | Riser | "The vanilla finish. Built in." over passing art. |
| 31-38 | The drop | One family per bar: raw candidate, then the processing clamps in and the final files land (below). |
| 39-40 | And many more | One grid of the generated families not shown elsewhere. |
| 41-44 | Portraits | Portraits from a photo, then "ComfyUI does it better". |
| 45-46 | Animated sprites | Real frames at each sprite's registered fps. |
| 47-49 | In the game | The World News window, then the agency logo selected, six beats each. "Real files. Loaded by the game." |
| 50-51 | Workflow | The full Agentic HOI4 Modding workflow, installed by HOI4 Mod Setup. |
| 52-55 | End card | The mark, the repository and the Discord, YouTube @ChaosRedux and Ko-fi links. |

## The pipeline stations

1. **Measured from the game.** Canvases roll in (397×153, 210×176, 82×52). "Every canvas read from the game itself."
2. **Plan first. See the cost.** The real `plan` output, "$0.00 on your ChatGPT plan".
3. **Generate.** "ChatGPT login. Or an API key." The provider list: ChatGPT login, OpenAI, Gemini, fal and ComfyUI. Real candidates develop.
4. **Style.** "It matches your mod's style." The New Order's focus icons go in as references and a new icon in their style comes out (shown with the TNO team's permission).
5. **Process.** "The vanilla finish. Automatic." A raw candidate becomes the 52×40 file the game uses.
6. **Validate.** "Checked before anyone looks." The real contact sheet and a pass.
7. **Review.** "No approval, no install." The approval stamp lands.
8. **Install.** "Installed. Registered." The DDS texture and the `.gfx` sprite are written into the mod.
9. **Audit.** The whole mod: 29,566 sprites and 22,239 textures read, 1 missing sprite found.
10. **Claude Code and Codex.** Each with its logo. On the second bar a third box joins them: "Any provider", any MCP client. "Same tools. Same mod."

## The drop

Report card (sepia, tilt, shadow), news photo (cropped, grayscale), flag ladder (three sizes, TGA), faction logo (selected frame, miniature), agency logo (normal and selected), unit counter (green map counter), division emblem (green glow) and achievement (normal, grey, locked). Painted art scales at most 3× and flat glyphs at most 6×, so nothing shows as blown-up pixels.

## Visual system

- **Medium:** HOI4 war-room materials. Steel plates with bevelled frames, a blueprint grid, period paper for the drop, and brass light.
- **Palette:** steel ink `#efe6cc` on gunmetal `#15212b` and `#1b2226`. Brass `#e8b54f`, with a gradient from `#fff1c4` to `#a8701a`, for hero words, clamp brackets and the end card.
- **Type:** Archivo Black for heavy caps with a brass or ice-steel fill. Oswald for labels. IBM Plex Mono for terminal text.
- **Layout:** everything sits inside the 90% action-safe box and is checked with `npx hyperframes check --at-transitions --frame-check` for overlaps and clipping.
- **Finished art at game size:** wherever finished art appears (the hook, the style step, the drop, the grid), it is the processed file the game loads, enlarged smoothly, so its detail matches vanilla and mod art at the same size.
- **Sound:** the music carries the edit. Its beat drives every cut, and five quiet Pixabay accents sit on downbeats where the picture lands a hit: the name slam, the riser into the drop, the drop, the portraits and the end card (`assets/sfx/CREDITS.md`).
