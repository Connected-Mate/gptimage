// Writes src/scenes/pipeline.html: the workflow steps as stations on one war-room table,
// joined by a single continuous camera move. Each station keeps its own markup, styles and motion
// from src/scenes/<id>.html; the camera travels between them on the last beat of each slot.
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const cfg = JSON.parse(readFileSync(join(here, "config.json"), "utf8"));
const pipe = cfg.scenes.find((s) => s.id === "pipeline");
const stations = pipe.stations; // [{ id, bars }]
const SX = 2300, SY = 1000;
const pos = (i) => [i * SX, (i % 2) * SY - (i % 4 === 3 ? SY * 0.6 : 0)];

const backdrops = /<div class="(table|steel|paper|map|grid)"[^>]*><\/div>\s*|\{\{GRAIN\}\}\s*|<div class="vig"><\/div>\s*/g;
let css = "", html = "", js = "";
let beat0 = 0;
stations.forEach(({ id, bars }, i) => {
  const src = readFileSync(join(here, "scenes", `${id}.html`), "utf8");
  const style = src.match(/<style>([\s\S]*?)<\/style>/)[1].replace("{{BASE}}", "");
  css += style.replace(/#root /g, `#root .S-${id} `).replace(/#root\s*\{/g, `#root .S-${id} {`) + "\n";
  const body = src.match(/data-height="1080">([\s\S]*)<\/div>\s*<script>/)[1].replace(backdrops, "");
  const [x, y] = pos(i);
  html += `      <div class="station S-${id}" style="left:${x}px;top:${y}px">${body}</div>\n`;
  const script = src.match(/\{\{LIB\}\}([\s\S]*?)window\.__timelines/)[1];
  js += `    { const DUR = b(${bars * 4}); const tl = gsap.timeline();\n      tl.set({}, {}, Math.max(0, DUR - 1 / 30));\n${script}\n      master.add(tl, b(${beat0})); }\n`;
  beat0 += bars * 4;
});

const camera = stations
  .map(({ bars }, i) => ({ i, at: stations.slice(0, i).reduce((a, s) => a + s.bars * 4, 0) }))
  .map(({ i, at }) => {
    if (i === 0) return "";
    const [x, y] = pos(i);
    return `    master.to("#pp-world", { x: ${-x}, y: ${-y}, duration: b(2), ease: "power2.inOut" }, b(${at - 2}));
    master.fromTo("#pp-cam", { scale: 1 }, { scale: 0.9, duration: b(1), ease: "sine.inOut", yoyo: true, repeat: 1, immediateRender: false }, b(${at - 2}));
    master.to("#pp-far", { x: ${-x * 0.35}, y: ${-y * 0.35}, duration: b(2), ease: "power2.inOut" }, b(${at - 2}));
    master.fromTo("#pp-streak", { xPercent: -100, opacity: 0.9 }, { xPercent: 100, opacity: 0, duration: b(2), ease: "power2.inOut", immediateRender: false }, b(${at - 2}));`;
  })
  .join("\n");

const n = stations.length;
const out = `<template>
  <style>
    {{BASE}}
    #root .far { position: absolute; left: -400px; top: -400px; width: ${SX * n * 0.4 + 3000}px; height: 3200px; background:
      repeating-linear-gradient(0deg, rgba(201,194,163,.05) 0 2px, transparent 2px 360px),
      repeating-linear-gradient(90deg, rgba(201,194,163,.05) 0 2px, transparent 2px 360px),
      #0f171d; }
    #root .world { position: absolute; left: 0; top: 0; width: ${SX * n + 1920}px; height: ${SY * 2 + 1080}px; }
    #root .ground { position: absolute; inset: -600px; background:
      repeating-linear-gradient(0deg, rgba(201,194,163,.10) 0 1px, transparent 1px 120px),
      repeating-linear-gradient(90deg, rgba(201,194,163,.10) 0 1px, transparent 1px 120px),
      repeating-linear-gradient(0deg, rgba(201,194,163,.04) 0 1px, transparent 1px 24px),
      repeating-linear-gradient(90deg, rgba(201,194,163,.04) 0 1px, transparent 1px 24px),
      rgba(21,33,43,.82); }
    #root .cam { position: absolute; inset: 0; transform-origin: 50% 50%; }
    #root .station { position: absolute; width: 1920px; height: 1080px; }
    #root .pstreak { position: absolute; inset: 0; background: linear-gradient(100deg, transparent 35%, rgba(255,226,150,.18) 50%, transparent 65%); pointer-events: none; }
${css}
  </style>
  <div id="root" data-composition-id="pipeline" data-width="1920" data-height="1080">
    <div class="far" id="pp-far"></div>
    <div class="cam" id="pp-cam"><div class="world" id="pp-world">
      <div class="ground"></div>
${html}
    </div></div>
    {{GRAIN}}
    <div class="pstreak" id="pp-streak"></div>
    <div class="vig"></div>
  </div>
  <script>
    {{LIB}}
    const master = tl;
${camera}
${js}
    window.__timelines["pipeline"] = master;
  </script>
</template>
`;
writeFileSync(join(here, "scenes", "pipeline.html"), out);
console.log(`pipeline: ${n} stations, ${beat0} beats`);
