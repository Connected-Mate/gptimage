// Dev tool: render a PNG into ANSI half-block "pixel art" for the terminal banner.
// Requires pngjs at build time only:  npm i pngjs --no-save  &&  node scripts/build-banner.mjs
// Output (src/banner.txt) is a committed artifact; the shipped tool needs no deps.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { PNG } from "pngjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SRC = path.join(ROOT, "assets", "terminal-art.png");
const OUT = path.join(ROOT, "src", "banner.txt");

const TARGET_W = 46; // characters wide (1 px per char column)
const TRANSPARENCY_THRESHOLD = 52; // distance from the corner bg color → treated as transparent

const png = PNG.sync.read(fs.readFileSync(SRC));
const { width: W, height: H, data } = png;

const at = (x, y) => {
  const i = (y * W + x) * 4;
  return [data[i], data[i + 1], data[i + 2], data[i + 3]];
};

// Nearest-neighbor downscale to TARGET_W (keeps pixel-art edges crisp).
const scale = W / TARGET_W;
const outW = TARGET_W;
const outH = Math.round(H / scale);
const grid = [];
for (let y = 0; y < outH; y++) {
  const row = [];
  for (let x = 0; x < outW; x++) {
    row.push(at(Math.min(W - 1, Math.floor(x * scale)), Math.min(H - 1, Math.floor(y * scale))));
  }
  grid.push(row);
}

const bg = grid[0][0];
const dist = (a, b) => Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
const transparent = (p) => p[3] < 32 || dist(p, bg) < TRANSPARENCY_THRESHOLD;

const fg = (p) => `\x1b[38;2;${p[0]};${p[1]};${p[2]}m`;
const bgc = (p) => `\x1b[48;2;${p[0]};${p[1]};${p[2]}m`;
const RESET = "\x1b[0m";
const DEFBG = "\x1b[49m";
const PAD = "  ";

let out = "\n";
for (let y = 0; y < outH; y += 2) {
  let line = PAD;
  for (let x = 0; x < outW; x++) {
    const top = grid[y][x];
    const bot = y + 1 < outH ? grid[y + 1][x] : bg;
    const tT = transparent(top);
    const bT = transparent(bot);
    if (tT && bT) line += " ";
    else if (!tT && !bT) line += fg(top) + bgc(bot) + "▀";
    else if (!tT && bT) line += fg(top) + DEFBG + "▀";
    else line += fg(bot) + DEFBG + "▄";
  }
  out += line + RESET + "\n";
}

// Wordmark + tagline under the art.
const C = "\x1b[1m\x1b[38;2;120;200;255m";
const D = "\x1b[2m";
out += "\n" + PAD + C + "G P T I M A G E" + RESET + "\n";
out += PAD + D + "image generation for Claude Code · powered by GPT Image 2" + RESET + "\n\n";

fs.writeFileSync(OUT, out);
console.error(`banner.txt written: ${outW}x${outH}px -> ${Math.ceil(outH / 2)} rows, ${out.length} bytes`);
