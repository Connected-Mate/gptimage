// Writes src/scenes/drop.html: one family per half bar, raw candidate -> clamp -> final files.
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const G = "assets/gallery";
// width and height from the PNG header
const size = (f) => { const b = readFileSync(join(root, f)); return [b.readUInt32BE(16), b.readUInt32BE(20)]; };
const P = (n) => `${G}/processed_png/${n}.png`;
// a chosen 0.3.1 image from the gallery run (assets/picked/)
const RT = (fam, n) => `assets/picked/${fam}/${n}.png`;
const S = (n) => `${G}/source_png/${n}.png`;
// building and medal wait for their 0.3.2 regenerations

// family, label, raw source (null for imported art), final files, note
export const items = [
  ["report_event", "report card", S("winter_border-v1"), [P("winter_border-v1-p1")], "sepia · tilt · shadow"],
  ["news_event", "news photo", S("peace_conference-v1"), [P("peace_conference-v1-p1")], "cropped · grayscale"],
  ["flag", "flag ladder", S("IRP_fascism-v1"), [P("IRP_fascism-v1-p1"), P("IRP_fascism-v1-p1_medium"), P("IRP_fascism-v1-p1_small")], "three sizes · TGA"],
  ["faction_logo", "faction logo", S("iron_pact-v1"), [P("iron_pact-v1-p1"), P("iron_pact-v1-p1_miniature")], "selected frame · miniature"],
  ["agency_logo", "agency logo", S("state_security-v1"), [P("state_security-v1-p1")], "normal · selected"],
  ["counter_large", "unit counter", RT("counter_large", "infantry_rifleman-v1"), [RT("counter_large", "infantry_rifleman-v1-p1")], "green map counter"],
  ["division_emblem", "division emblem", RT("division_emblem", "bear_standing-v1"), [RT("division_emblem", "bear_standing-v1-p1")], "green glow"],
  ["achievement", "achievement", S("hold_the_line-v1"), [P("hold_the_line-v1-p1"), P("hold_the_line-v1-p1_grey"), P("hold_the_line-v1-p1_not_eligible")], "normal · grey · locked"],
];

const BOX_W = 780, BOX_H = 480;
function finals(files, glyph) {
  if (!files.length) return "";
  const dims = files.map(size);
  const totalW = dims.reduce((a, [w]) => a + w, 0);
  const maxH = Math.max(...dims.map(([, h]) => h));
  const gap = 24;
  let k = Math.floor(Math.min((BOX_W - gap * (files.length - 1)) / totalW, BOX_H / maxH));
  // painted art stops at 3x (5x when tiny) so the smooth scaling stays sharp; flat glyph art (counters, emblems) scales cleanly to 6x
  const big = Math.max(...dims.map(([w, h]) => Math.max(w, h)));
  k = Math.max(1, Math.min(k, glyph ? 6 : big < 64 ? 5 : 3));
  const w = totalW * k + gap * (files.length - 1);
  let x = (BOX_W - w) / 2;
  return files
    .map((f, i) => {
      const [fw, fh] = dims[i];
      const el = `<img class="px dr-f" src="${f}" style="left:${Math.round(x)}px;top:${Math.round((BOX_H - fh * k) / 2)}px;width:${fw * k}px;height:${fh * k}px" alt="" />`;
      x += fw * k + gap;
      return el;
    })
    .join("");
}

const groups = items
  .map(([fam, label, raw, files, note], i) => {
    const n = String(i + 1).padStart(2, "0");
    const rawEl = raw
      ? `<div class="dr-raw${raw.startsWith("assets/picked/") ? " card" : ""}"><img src="${raw}" alt="" /></div>`
      : `<div class="dr-raw dr-imp"><span class="hero">IMPORT</span><span class="mono">art made elsewhere · processed to the strip</span></div>`;
    return `      <div class="dr-g" id="dr-g${n}">
        ${rawEl}
        <div class="dr-box">${finals(files, fam === "counter_large" || fam === "division_emblem")}<div class="bracket tl"></div><div class="bracket tr"></div><div class="bracket bl"></div><div class="bracket br"></div></div>
        <div class="dr-name hero ice">${label}</div>
        <div class="dr-fam">${note}</div>
      </div>`;
  })
  .join("\n");

const html = `<template>
  <style>
    {{BASE}}
    #root .dr-g { position: absolute; inset: 0; opacity: 0; }
    #root .dr-raw { position: absolute; left: 110px; top: 170px; width: 780px; height: 600px; display: flex; align-items: center; justify-content: center; }
    #root .dr-raw img { max-width: 780px; max-height: 560px; filter: drop-shadow(0 30px 40px rgba(0,0,0,.55)); }
    #root .dr-raw span.mono { position: absolute; left: 0; right: 0; bottom: -34px; text-align: center; font-size: 22px; color: #c9c2a3; white-space: nowrap; }
    #root .dr-raw.card img { background: #efe6cc; padding: 40px; border-radius: 6px; max-width: 600px; max-height: 460px; }
    #root .dr-imp { flex-direction: column; border: 3px dashed rgba(201,194,163,.3); }
    #root .dr-imp .hero { font-size: 110px; color: rgba(201,194,163,.35); }
    #root .dr-imp span.mono { position: static; margin-top: 20px; }
    #root .dr-box { position: absolute; left: 990px; top: 200px; width: ${BOX_W}px; height: ${BOX_H}px; }
    #root .dr-box img { position: absolute; filter: drop-shadow(0 18px 24px rgba(0,0,0,.6)); }
    #root .dr-name { position: absolute; left: 990px; top: 730px; width: 780px; text-align: center; font-size: 72px; white-space: nowrap; }
    #root .dr-fam { position: absolute; left: 990px; top: 830px; width: 780px; text-align: center; font-size: 36px; letter-spacing: .03em; color: #efe6cc; }
    #root .dr-fam b { color: #e8b54f; font-weight: 500; }
    #root .dr-n { position: absolute; right: 110px; top: 70px; font-size: 64px; color: #e8b54f; }
    #root .dr-n span { color: rgba(201,194,163,.4); font-size: 36px; }
    #root .flash { position: absolute; inset: 0; background: #fff1c4; mix-blend-mode: overlay; opacity: 0; }
  </style>
  <div id="root" data-composition-id="drop" data-width="1920" data-height="1080">
    <div class="paper" id="dr-paper" data-layout-allow-overflow></div>
    {{GRAIN}}
${groups}
    <div class="flash" id="dr-flash"></div>
    <div class="vig"></div>
  </div>
  <script>
    {{LIB}}
    const N = ${items.length};
    const step = b(4);
    tl.fromTo("#dr-paper", { scale: 1.1 }, { scale: 1, duration: DUR, ease: "none" }, 0);
    for (let i = 0; i < N; i++) {
      const g = "#dr-g" + String(i + 1).padStart(2, "0");
      const t0 = b(i * 4);
      const t1 = b((i + 1) * 4);
      tl.set(g, { opacity: 1 }, t0);
      if (i < N - 1) tl.set(g, { opacity: 0 }, t1);
      tl.fromTo(g + " .dr-raw", { x: -260, rotation: -5, filter: "blur(10px)", opacity: 0 }, { x: 0, rotation: -2, filter: "blur(0px)", opacity: 1, duration: b(1), ease: "expo.out" }, t0);
      tl.set(g + " .dr-raw", { filter: "none" }, t0 + b(1));
      tl.fromTo(g + " .dr-box .bracket", { scale: 2.2, opacity: 0 }, { scale: 1, opacity: 1, duration: b(0.5), ease: "back.out(2.4)" }, t0 + b(2));
      tl.fromTo(g + " .dr-f", { scale: 2.4, opacity: 0, filter: "blur(12px)" }, { scale: 1, opacity: 1, filter: "blur(0px)", duration: b(0.75), ease: "expo.out", stagger: b(0.25) }, t0 + b(2));
      tl.fromTo(g + " .dr-name", { opacity: 0, x: 80 }, { opacity: 1, x: 0, duration: b(0.5), ease: "power4.out" }, t0 + b(2));
      tl.fromTo(g + " .dr-fam", { opacity: 0 }, { opacity: 1, duration: b(0.5) }, t0 + b(2));
      tl.fromTo("#dr-flash", { opacity: 0.55 }, { opacity: 0, duration: b(1), ease: "power2.out", immediateRender: false }, t0 + b(2));
    }
    window.__timelines["drop"] = tl;
  </script>
</template>
`;
writeFileSync(join(here, "scenes", "drop.html"), html);
console.log(`drop: ${items.length} families`);
