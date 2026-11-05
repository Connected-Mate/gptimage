// Writes the pipeline stations (src/scenes/<id>.html) from one calm template:
// a big two-part title, one large visual on its own colour wash, and at most one large line.
// Content lands on beats 0 to 2 and then holds still, so the viewer reads it on the music.
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const G = "assets/gallery";
const PX = "";
// width and height from a PNG header
const size = (p) => { const b = readFileSync(join(here, "..", p)); return [b.readUInt32BE(16), b.readUInt32BE(20)]; };
// an image at its in-game size, enlarged k times, so finished art keeps the detail the game shows
const game = (p, k) => { const [w, h] = size(p); return `<img src="${p}" style="width:${Math.round(w * k)}px;height:${Math.round(h * k)}px" />`; };
const logo = (n, fill) => readFileSync(join(here, "..", "assets", "logos", `${n}.svg`), "utf8").replace("<svg ", `<svg fill="${fill}" width="120" height="120" `).replace(/<title>[^<]*<\/title>/, "");
// the style step: The New Order's own icons (shown with the team's consent) and the icon made from them, from
// assets/tno/ as written by src/pull-regen.mjs. There is no other mod to fall back on: the build stops until TNO is in,
// unless TRAILER_PENDING=1 is set for a review build, which leaves the step empty.
const tnoSrc = join(here, "..", "assets", "tno", "source.json");
const tnoAsset = existsSync(tnoSrc) ? JSON.parse(readFileSync(tnoSrc, "utf8")).asset : null;
if (!tnoAsset && process.env.TRAILER_PENDING !== "1") throw new Error("the TNO style run is not pulled into assets/tno yet");
const style = { mod: "The New Order", out: tnoAsset ? `assets/regen/${tnoAsset}/${tnoAsset}-v1-p1.png` : "" };
const refDir = join(here, "..", "assets", "tno", "refs");
// four of the run's references whose art fills its canvas (refs 1 and 4 sit small in theirs)
const refFiles = existsSync(refDir) ? readdirSync(refDir).filter((f) => f.endsWith(".png")).sort() : [];
const shown = ["ref_2.png", "ref_3.png", "ref_6.png", "ref_7.png"].filter((f) => refFiles.includes(f));
style.refs = (shown.length === 4 ? shown : refFiles.slice(0, 4)).map((f) => `assets/tno/refs/${f}`);

// the process step's burst: one finished file of each kind, at its in-game size enlarged to fit a tile. [file, family
// label, frames in the strip]
const BURST = [
  ["assets/regen/embassy_flags_handshake_v033/embassy_flags_handshake_v033-v1-p1.png", "focus"],
  ["assets/picked/idea_spirit/women_welders-v1-p1.png", "idea"],
  ["assets/gallery/processed_png/IRP_fascism-v1-p1.png", "flag"],
  ["assets/gallery/processed_png/winter_border-v1-p1.png", "report event"],
  ["assets/gallery/processed_png/peace_conference-v1-p1.png", "news event"],
  ["assets/gallery/processed_png/heavy_tank-v1-p1.png", "technology"],
  ["assets/cmp/mulcahy_mcp.png", "leader portrait"],
  ["assets/regen/laurel_wreath_v033/laurel_wreath_v033-v1-p1.png", "medal"],
  ["assets/regen/military_reforms/military_reforms-v1-p1.png", "decision category"],
  ["assets/picked/raid/paratrooper_canopy-v1-p1.png", "raid"],
  ["assets/picked/doctrine/howitzer_battery_night-v1-p1.png", "doctrine"],
  ["assets/regen/platform_suitcase_swap/platform_suitcase_swap-v1-p1.png", "operation"],
  ["assets/regen/japanese_five_tier_pagoda/japanese_five_tier_pagoda-v1-p1.png", "building", 4],
  ["assets/gallery/processed_png/hold_the_line-v1-p1.png", "achievement"],
];
const TILE_W = 190, TILE_H = 170;
const burst = BURST.map(([p, label, n = 1]) => {
  const [sw, h] = size(p), w = sw / n, k = Math.min(5, TILE_W / w, TILE_H / h);
  const art = n > 1
    ? `<div style="width:${Math.round(w * k)}px;height:${Math.round(h * k)}px;background:url(${p}) 0 0/${n * 100}% 100% no-repeat"></div>`
    : `<img src="${p}" style="width:${Math.round(w * k)}px;height:${Math.round(h * k)}px" />`;
  return `<div class="bt"><div class="ba">${art}</div><div class="bl">${label}</div></div>`;
}).join("");

// Every value shown comes from a real hoi4-imagegen run (see SOURCES.md).
// wash: the colour behind the visual, taken from the step's asset or meaning
const S = [
  { id: "measured", t1: "Measured from", t2: "the game", wash: "#2f5d73",
    vis: `<div class="row3">
      <div class="m"><div class="pic"><img src="${G}/processed_png/peace_conference-v1-p1.png" style="width:596px;height:230px" /></div><div class="sz hero ice">397×153</div></div>
      <div class="m"><div class="pic"><img src="${G}/processed_png/winter_border-v1-p1.png" style="width:315px;height:264px" /></div><div class="sz hero ice">210×176</div></div>
      <div class="m"><div class="pic"><img src="${G}/processed_png/IRP_fascism-v1-p1.png" style="${PX}width:328px;height:208px" /></div><div class="sz hero ice">82×52</div></div></div>`,
    line: "Every canvas read from the game itself" },
  { id: "plan", t1: "Plan first.", t2: "See the cost.", wash: "#3f6b2c",
    vis: `<div class="price"><div class="big hero brass">$0.00</div></div>`,
    line: "on your ChatGPT plan" },
  { id: "generate", t1: "ChatGPT login.", t2: "Or an API key.", wash: "#1f6f63",
    vis: `<div class="row3 dev">
      <div class="m"><img class="g-dev" src="${G}/source_png/heavy_tank-v1.png" style="width:470px" /></div>
      <div class="m"><img class="g-dev" src="${G}/source_png/state_security-v1.png" style="height:330px" /></div>
      <div class="m"><img class="g-dev" src="${G}/source_png/peace_conference-v1.png" style="width:470px" /></div></div>
      <div class="chips"><span style="background:#10a37f;color:#fff">ChatGPT login</span><span style="background:#e8e8e8;color:#111">OpenAI</span><span style="background:#4f7cf0;color:#fff">Gemini</span><span style="background:#7b3fe4;color:#fff">fal</span><span style="background:#f2c94c;color:#111">ComfyUI</span></div>`,
    line: "" },
  { id: "style", t1: "It matches", t2: "your mod's style", wash: "#7a2c2c",
    vis: style.refs.length
      ? `<div class="refs">
      ${style.refs.map((r) => `<div class="r">${game(r, 2.2)}</div>`).join("")}
      <div class="arrow hero brass">›</div>
      <div class="out plate">${game(style.out, 2.2)}</div></div>`
      : `<div class="solo">${style.out ? game(style.out, 4) : ""}</div>`,
    line: style.refs.length ? `${style.mod} icons in · a new one out` : `Made to match ${style.mod}` },
  { id: "process", t1: "The vanilla finish.", t2: "Automatic.", wash: "#8c6a1e",
    vis: `<div class="burst">${burst}</div>`,
    line: "Every family gets its own finish, at the game's size" },
  { id: "validate", t1: "Checked before", t2: "anyone looks", wash: "#2d6a3a",
    vis: `<div class="sheet plate"><img src="${G}/preview_heavy_tank.png" /></div><div class="stamp ok">✓ Pass</div>`,
    line: "" },
  { id: "review", t1: "No approval,", t2: "no install", wash: "#6e2330",
    vis: `<div class="rev"><div class="portrait plate"><img src="${G}/processed_png/iron_pact-v1-p1.png" style="width:400px;height:200px" /></div><div class="stamp big">Approved</div></div>`,
    line: "" },
  { id: "install", t1: "Installed.", t2: "Registered.", wash: "#24476e",
    vis: `<div class="inst"><div class="tank plate"><img src="${G}/processed_png/heavy_tank-v1-p1.png" style="${PX}" /></div><div class="arrow hero brass">›</div>
      <div class="files"><div class="file"><b>DDS</b><span>texture</span></div><div class="file gfx"><b>.gfx</b><span>sprite</span></div></div></div>`,
    line: "" },
  { id: "audit", t1: "Audit", t2: "the whole mod", wash: "#4a2f6b",
    vis: `<div class="nums"><div><b class="hero ice">29,566</b><span>sprites read</span></div><div><b class="hero ice">22,239</b><span>textures</span></div></div>
      <div class="finding">1 missing sprite found</div>`,
    line: "" },
  { id: "clients", t1: "Claude Code", t2: "& Codex", wash: "#7a4a22",
    vis: `<div class="cl"><div class="plate c1" style="box-shadow: inset 0 0 0 3px #d97757, 0 30px 60px rgba(0,0,0,.65)">${logo("claude", "#d97757")}<div class="hero" style="color:#f1b39a">Claude Code</div></div><div class="plate c1" style="box-shadow: inset 0 0 0 3px #e8e8e8, 0 30px 60px rgba(0,0,0,.65)">${logo("openai", "#f2f2f2")}<div class="hero" style="color:#f2f2f2">Codex</div></div><div class="plate c1 any" style="box-shadow: inset 0 0 0 3px #e8b54f, 0 30px 60px rgba(0,0,0,.65)"><div class="plus hero brass">+</div><div class="hero brass">Any provider</div><div class="sub">any MCP client</div></div></div>`,
    line: "Same tools. Same mod." },
];

const css = `
    {{BASE}}
    /* the station template, inside the 90% action-safe box (x 96-1824, y 54-1026) */
    #root .wash { position: absolute; inset: 0; pointer-events: none; }
    #root .tt { position: absolute; left: 96px; right: 96px; top: 96px; text-align: center; font-size: 80px; white-space: nowrap; }
    #root .vis { position: absolute; left: 96px; right: 96px; top: 250px; height: 600px; display: flex; flex-direction: column; align-items: center; justify-content: center; }
    #root .line { position: absolute; left: 96px; right: 96px; top: 880px; text-align: center; font-size: 46px; letter-spacing: .04em; color: #efe6cc; }
    #root .row3 { display: flex; gap: 60px; align-items: flex-end; }
    #root .row3 .m { display: flex; flex-direction: column; align-items: center; gap: 18px; }
    #root .row3 .pic { height: 270px; display: flex; align-items: flex-end; }
    #root .row3 .pic, #root .dev .m, #root .pair .pa, #root .refs .r { filter: drop-shadow(0 22px 26px rgba(0,0,0,.6)); }
    #root .row3 .sz { font-size: 72px; }
    #root .price .big { font-size: 300px; line-height: .9; }
    #root .dev { align-items: center; }
    #root .chips { margin-top: 48px; display: flex; gap: 16px; }
    #root .chips span { padding: 12px 28px; font-size: 34px; letter-spacing: .04em; border-radius: 6px; box-shadow: 0 12px 24px rgba(0,0,0,.45); }
    #root .refs { display: flex; align-items: center; gap: 24px; }
    #root .refs .r img { display: block; }
    #root .arrow { font-size: 120px; margin: 0 20px; }
    #root .refs .out { padding: 40px 46px; display: flex; align-items: center; justify-content: center; }
    #root .refs .out img { display: block; }
    #root .solo { filter: drop-shadow(0 26px 34px rgba(0,0,0,.65)); }
    #root .solo img { display: block; }
    #root .pair { display: flex; align-items: center; gap: 30px; }
    #root .pair .pa img { width: 520px; height: 520px; display: block; object-fit: contain; }
    #root .clamp { position: relative; width: 400px; height: 440px; }
    #root .clamp .slot { position: absolute; left: 30px; top: 26px; width: 340px; height: 388px; display: flex; align-items: center; justify-content: center; background: linear-gradient(160deg, #262d31, #14181b); box-shadow: inset 0 0 0 3px #3b444a; }
    #root .clamp .slot img { width: 300px; height: 340px; }
    #root .sheet { padding: 16px; }
    #root .sheet img { width: 1600px; display: block; }
    #root .stamp { margin-top: 50px; font-size: 80px; }
    #root .stamp.ok { color: #7fd38a; border-color: #7fd38a; box-shadow: 0 0 30px rgba(127,211,138,.35), inset 0 0 20px rgba(127,211,138,.2); }
    #root .rev { display: flex; align-items: center; gap: 80px; }
    #root .rev .stamp { margin: 0; }
    #root .stamp.big { font-size: 96px; transform: rotate(-5deg); }
    #root .portrait { padding: 16px; }
    #root .portrait img { display: block; }
    #root .inst { display: flex; align-items: center; }
    #root .inst .tank { padding: 30px; }
    #root .inst .tank img { width: 730px; height: 270px; display: block; }
    #root .files { display: flex; gap: 30px; }
    #root .file { position: relative; width: 200px; height: 250px; border-radius: 8px; background: linear-gradient(170deg, #efe6cc, #c9bd98); box-shadow: 0 24px 40px rgba(0,0,0,.55); display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 6px; clip-path: polygon(0 0, 72% 0, 100% 22%, 100% 100%, 0 100%); }
    #root .file b { font-family: "Archivo Black", sans-serif; font-size: 58px; color: #24476e; font-weight: 400; }
    #root .file.gfx b { color: #8c4a12; }
    #root .file span { font-size: 30px; color: #3a3428; letter-spacing: .06em; text-transform: uppercase; }
    #root .nums { display: flex; gap: 140px; }
    #root .nums div { display: flex; flex-direction: column; align-items: center; }
    #root .nums b { font-size: 150px; font-weight: 400; }
    #root .nums span { font-size: 36px; letter-spacing: .1em; text-transform: uppercase; color: #c9c2a3; }
    #root .finding { margin-top: 60px; font-size: 48px; letter-spacing: .04em; color: #fff; padding: 16px 40px; border-radius: 6px; background: #b8342c; box-shadow: 0 0 40px rgba(184,52,44,.5); }
    #root .burst { display: grid; grid-template-columns: repeat(7, 220px); gap: 26px 20px; }
    #root .burst .bt { display: flex; flex-direction: column; align-items: center; gap: 10px; }
    #root .burst .ba { width: 220px; height: 190px; display: flex; align-items: center; justify-content: center; filter: drop-shadow(0 14px 18px rgba(0,0,0,.55)); }
    #root .burst .bl { font-family: "IBM Plex Mono", monospace; font-size: 19px; letter-spacing: .04em; color: #c9c2a3; white-space: nowrap; }
    #root .cl { display: flex; gap: 50px; }
    #root .cl .c1 { width: 520px; height: 360px; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 30px; }
    #root .cl .c1 svg { display: block; filter: drop-shadow(0 10px 18px rgba(0,0,0,.5)); }
    #root .cl .c1 .hero { font-size: 58px; text-align: center; white-space: nowrap; }
    #root .cl .any .plus { font-size: 120px; line-height: 120px; height: 120px; }
    #root .cl .any .sub { margin-top: -18px; font-size: 32px; letter-spacing: .06em; color: #c9c2a3; }
`;

S.forEach((s) => {
  const html = `<template>
  <style>${css}
  </style>
  <div id="root" data-composition-id="${s.id}" data-width="1920" data-height="1080">
    <div class="table"></div>
    <div class="wash" data-layout-allow-overflow id="${s.id}-wash" style="background: radial-gradient(ellipse 60% 55% at 50% 52%, ${s.wash}cc 0%, ${s.wash}55 45%, transparent 75%)"></div>
    {{GRAIN}}
    <div class="tt hero"><span class="ice ${s.id}-w">${s.t1}</span> <span class="brass ${s.id}-w">${s.t2}</span></div>
    <div class="vis" id="${s.id}-vis">${s.vis}</div>
    ${s.line ? `<div class="line" id="${s.id}-line">${s.line}</div>` : ""}
    <div class="vig"></div>
  </div>
  <script>
    {{LIB}}
    // beat 0: the title rises in word by word; from beat 0.5 the visual's parts follow one after another on sixteenth
    // notes, each easing smoothly into place; beat 2: the line. Then everything holds still.
    tl.fromTo("#${s.id}-wash", { opacity: 0 }, { opacity: 1, duration: b(1), ease: "sine.out" }, 0);
    tl.fromTo(".${s.id}-w", { opacity: 0, y: 34, filter: "blur(10px)" }, { opacity: 1, y: 0, filter: "blur(0px)", duration: b(1), ease: "expo.out", stagger: b(0.5) }, 0);
${entrance(s)}${s.line ? `    tl.fromTo("#${s.id}-line", { opacity: 0, y: 24 }, { opacity: 1, y: 0, duration: b(1), ease: "expo.out" }, b(2));\n` : ""}${extra(s.id)}
    window.__timelines["${s.id}"] = tl;
  </script>
</template>
`;
  writeFileSync(join(here, "scenes", `${s.id}.html`), html);
});

// the parts of each station's visual that enter one after another, like dominoes (the rest enter as one block)
function entrance(s) {
  const items = { measured: ".row3 .m", generate: ".row3 .m", style: ".refs > *", validate: ".sheet", review: ".rev > .portrait", install: ".inst > .tank, .inst > .arrow", audit: ".nums > div", clients: ".cl .c1:not(.any)" }[s.id];
  if (!items) return `    tl.fromTo("#${s.id}-vis", { opacity: 0, y: 50, scale: 0.94 }, { opacity: 1, y: 0, scale: 1, duration: b(1.25), ease: "expo.out" }, b(0.5));\n`;
  const sel = items.split(",").map((x) => `#${s.id}-vis ${x.trim()}`).join(", ");
  return `    tl.fromTo("#${s.id}-vis", { opacity: 0 }, { opacity: 1, duration: b(0.25) }, b(0.5));
    tl.fromTo("${sel}", { opacity: 0, y: 60, scale: 0.92, rotation: -2 }, { opacity: 1, y: 0, scale: 1, rotation: 0, duration: b(1.25), ease: "expo.out", stagger: b(0.25) }, b(0.5));\n`;
}

// one accent per station, on a beat
function extra(id) {
  switch (id) {
    case "generate":
      return `    tl.fromTo(".g-dev", { filter: "brightness(2.6) contrast(.4) blur(12px)" }, { filter: "brightness(1) contrast(1) blur(0px)", duration: b(1.5), ease: "power2.out", stagger: b(0.5) }, b(0.5));
    tl.fromTo("#generate-vis .chips span", { opacity: 0, y: 16 }, { opacity: 1, y: 0, duration: b(0.5), ease: "power3.out", stagger: b(0.25) }, b(2));`;
    case "clients":
      // Claude Code and Codex land with the visual, the third box on the second bar
      // the first two start centred and make room as the third arrives (half a box and gap: (520 + 50) / 2)
      return `    tl.fromTo("#clients-vis .cl", { x: 285 }, { x: 0, duration: b(0.75), ease: "power3.inOut" }, b(4));
    tl.fromTo("#clients-vis .any", { opacity: 0, y: 40, scale: 0.94 }, { opacity: 1, y: 0, scale: 1, duration: b(1), ease: "power3.out" }, b(4.5));`;
    case "process":
      // one kind of art lands on every sixteenth note, then the whole set holds through the second bar
      return `    tl.fromTo("#process-vis .bt", { opacity: 0, y: 50, scale: 0.85, rotation: -3 }, { opacity: 1, y: 0, scale: 1, rotation: 0, duration: b(1.25), ease: "expo.out", stagger: b(0.25) }, b(0.5));`;
    case "validate":
    case "review":
      return `    tl.fromTo("#${id}-vis .stamp", { opacity: 0, scale: 1.6 }, { opacity: 1, scale: 1, duration: b(0.5), ease: "back.out(1.8)" }, b(2));`;
    case "install":
      return `    tl.fromTo("#install-vis .file", { opacity: 0, y: -60 }, { opacity: 1, y: 0, duration: b(0.75), ease: "back.out(1.4)", stagger: b(0.5) }, b(2));`;
    case "audit":
      return `    tl.fromTo("#audit-vis .finding", { opacity: 0, scale: 1.15 }, { opacity: 1, scale: 1, duration: b(0.5), ease: "back.out(1.8)" }, b(2));`;
    default:
      return "";
  }
}
console.log(`stations: ${S.length}`);
