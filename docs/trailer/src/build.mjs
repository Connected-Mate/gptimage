// Builds index.html and compositions/*.html from src/ on the music's beat grid.
// Usage: node src/build.mjs   (reads src/config.json)
import { readFileSync, writeFileSync, mkdirSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const cfg = JSON.parse(readFileSync(join(here, "config.json"), "utf8"));
const beat = 60 / cfg.bpm;
const bar = beat * cfg.beatsPerBar;
const base = readFileSync(join(here, "base.css"), "utf8");
const lib = readFileSync(join(here, "lib.js"), "utf8");
const grain = (id) =>
  `<svg class="grain" width="1920" height="1080"><filter id="${id}-n"><feTurbulence type="fractalNoise" baseFrequency=".85" numOctaves="2" seed="7"/><feColorMatrix values="0 0 0 0 .5  0 0 0 0 .5  0 0 0 0 .5  0 0 0 .9 0"/></filter><rect width="100%" height="100%" filter="url(#${id}-n)"/></svg>`;
const r3 = (x) => Math.round(x * 1000) / 1000;

mkdirSync(join(root, "compositions"), { recursive: true });
let t = cfg.offset;
const hosts = [];
for (const s of cfg.scenes) {
  const dur = s.bars * bar;
  const src = readFileSync(join(here, "scenes", `${s.id}.html`), "utf8")
    .replaceAll("{{BASE}}", base)
    .replaceAll("{{LIB}}", lib.replaceAll("{{BEAT}}", String(beat)).replaceAll("{{DUR}}", String(dur)))
    .replaceAll("{{GRAIN}}", grain(s.id))
    .replaceAll("{{BEAT}}", String(beat))
    .replaceAll("{{DUR}}", String(dur));
  // a soft steel dip at every scene change: the scene rises out of it and settles back into it
  const dip = `<div class="dip" id="${s.id}-dip" style="position:absolute;inset:0;background:#0b1014;pointer-events:none;z-index:50"></div>`;
  const dipJs = `    tl.fromTo("#${s.id}-dip", { opacity: 1 }, { opacity: 0, duration: b(0.5), ease: "power2.out" }, 0);
    tl.fromTo("#${s.id}-dip", { opacity: 0 }, { opacity: 0.85, duration: b(0.5), ease: "power2.in", immediateRender: false }, DUR - b(0.5));
`;
  const vi = src.lastIndexOf('<div class="vig"></div>');
  const ti = src.lastIndexOf(`    window.__timelines["${s.id}"]`);
  if (vi < 0 || ti < 0) throw new Error(`no vignette or timeline registration in ${s.id}`);
  const out = src.slice(0, vi) + dip + "\n    " + src.slice(vi, ti) + dipJs + src.slice(ti);
  writeFileSync(join(root, "compositions", `${s.id}.html`), out);
  hosts.push(
    `      <div id="${s.id}" data-composition-id="${s.id}" data-composition-src="compositions/${s.id}.html" data-start="${r3(t)}" data-duration="${r3(dur)}" data-track-index="1" data-track-kind="graphics"${s.id === "pipeline" ? "" : ' data-motion-blur="off"'} data-width="1920" data-height="1080"></div>`,
  );
  t += dur;
}
const total = r3(t + (cfg.tail || 0));
const audio = existsSync(join(root, cfg.track || "-"))
  ? `      <audio id="music" class="clip" src="${cfg.track}" data-start="0" data-duration="${total}" data-media-start="${cfg.trackStart || 0}" data-volume="${cfg.musicVolume || 1}" data-fade-out="2" data-track-index="5"></audio>`
  : "";
// sound effects on the beat grid: each cue takes the first audio track that is free at its start, so overlapping
// cues never share a track; mstart skips into the file (the riser's last bars)
const trackEnds = [];
const sfx = (cfg.sfx || [])
  .filter((x) => existsSync(join(root, x.src)))
  .map((x, i) => {
    const start = r3(cfg.offset + x.bar * bar + (x.beat || 0) * beat);
    const dur = x.dur || 2;
    let k = trackEnds.findIndex((end) => end <= start);
    if (k < 0) k = trackEnds.push(0) - 1;
    trackEnds[k] = start + dur;
    return `      <audio id="sfx-${i}" class="clip" src="${x.src}" data-start="${start}" data-duration="${dur}"${x.mstart ? ` data-media-start="${x.mstart}"` : ""} data-volume="${x.vol || 0.6}" data-track-index="${6 + k}"></audio>`;
  })
  .join("\n");

const index = `<!doctype html>
<html lang="en" data-resolution="landscape">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=1920, height=1080" />
    <script src="https://cdn.jsdelivr.net/npm/gsap@3.15.0/dist/gsap.min.js"></script>
    <style>
      * { margin: 0; padding: 0; box-sizing: border-box; }
      html, body { margin: 0; width: 1920px; height: 1080px; overflow: hidden; background: #0d1318; }
    </style>
  </head>
  <body>
    <div id="root" data-composition-id="main" data-start="0" data-duration="${total}" data-width="1920" data-height="1080" data-motion-blur="on">
${hosts.join("\n")}
${audio}
${sfx}
    </div>
    <script>
      window.__timelines["main"] = gsap.timeline({ paused: true });
    </script>
  </body>
</html>
`;
writeFileSync(join(root, "index.html"), index);
console.log(`bpm ${cfg.bpm}, beat ${r3(beat)}s, bar ${r3(bar)}s, ${cfg.scenes.length} scenes, ${total}s`);
