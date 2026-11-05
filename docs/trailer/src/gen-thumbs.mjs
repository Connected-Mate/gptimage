// Writes thumbs/index.html: the YouTube thumbnail options, one per second, in the trailer's own type and art.
// Usage: node src/gen-thumbs.mjs, then npx hyperframes snapshot thumbs --at 0.5,1.5,... and src/thumbs.py to size them.
// The art is copied into thumbs/assets/ (ignored), because a HyperFrames project serves only its own folder.
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const out = join(root, "thumbs");
const base = readFileSync(join(here, "base.css"), "utf8");
const used = new Set();
const A = (p) => {
  if (!existsSync(join(root, p))) throw new Error(`missing ${p}`);
  used.add(p);
  return p;
};

const tno = JSON.parse(readFileSync(join(root, "assets/tno/source.json"), "utf8")).asset;
const logo = A("assets/logo_imagegen.png");
const plate = (src, w, h, extra = "") => `<div class="pl plate" style="width:${w}px;height:${h}px;${extra}"><img src="${A(src)}" alt="" /></div>`;
const brand = `<div class="brand"><img src="${logo}" alt="" /><span class="hero ice">HOI4 Imagegen</span></div>`;

const options = [
  // 1: the promise, with a fan of finished art
  `<div class="table"></div>
    <div class="t1 hero"><div class="ice">AI art</div><div class="brass">ready for</div><div class="brass">HOI4</div></div>
    <div class="fan">
      ${plate("assets/regen/embassy_flags_handshake_v033/embassy_flags_handshake_v033-v1-p1.png", 330, 330, "left:0;top:150px;transform:rotate(-8deg)")}
      ${plate("assets/picked/idea_spirit/women_welders-v1-p1.png", 330, 330, "left:250px;top:40px;transform:rotate(3deg);z-index:2")}
      ${plate("assets/regen/military_reforms/military_reforms-v1-p1.png", 330, 330, "left:520px;top:190px;transform:rotate(9deg);z-index:3")}
      ${plate(`assets/regen/${tno}/${tno}-v1-p1.png`, 300, 300, "left:120px;top:470px;transform:rotate(-4deg);z-index:4")}
      ${plate("assets/picked/raid/paratrooper_canopy-v1-p1.png", 300, 300, "left:430px;top:520px;transform:rotate(5deg);z-index:5")}
    </div>
    ${brand}`,
  // 2: raw model output to the game's file
  `<div class="table"></div>
    <div class="t2 hero"><span class="ice">Raw AI image</span> <span class="arrow brass">›</span> <span class="brass">game-ready</span></div>
    <div class="raw"><img src="${A("assets/gallery/source_png/heavy_tank-v1.png")}" alt="" /></div>
    <div class="clampbox"><img src="${A("assets/gallery/processed_png/heavy_tank-v1-p1.png")}" alt="" /><div class="bracket tl"></div><div class="bracket tr"></div><div class="bracket bl"></div><div class="bracket br"></div></div>
    <div class="t2b"><span class="chip on">DDS</span><span class="chip on">.gfx</span><span class="chip on">vanilla size</span></div>
    ${brand}`,
  // 3: the "many more" wall
  `<div class="paper"></div>
    <div class="wall">${[
      "assets/picked/idea_spirit/women_welders-v1-p1.png", "assets/regen/signed_treaty_quill_v033/signed_treaty_quill_v033-v1-p1.png", "assets/picked/raid/paratrooper_canopy-v1-p1.png",
      "assets/regen/military_reforms/military_reforms-v1-p1.png", "assets/gallery/processed_png/armistice-v1-p1.png", "assets/picked/operation_icon/telephone_with_bug-v1-p1.png",
      "assets/gallery/processed_png/iron_will-v1-p1.png", "assets/picked/doctrine/howitzer_battery_night-v1-p1.png", "assets/gallery/processed_png/heavy_tank-v1-p1.png",
      "assets/picked/idea_concern/shell_filling_hall-v1-p1.png", "assets/gallery/processed_png/hold_the_line-v1-p1.png", "assets/gallery/processed_png/defenders_of_order-v1-p1.png",
    ].map((s) => `<div class="w plate"><img src="${A(s)}" alt="" /></div>`).join("")}</div>
    <div class="shade"></div>
    <div class="t3 hero"><div class="brass">40</div><div class="ice">asset families</div></div>
    ${brand}`,
  // 4: matches your mod's style
  `<div class="table"></div>
    <div class="t4 hero"><span class="ice">Your mod's</span> <span class="brass">style</span></div>
    <div class="refs">${["ref_2", "ref_3", "ref_6"].map((r) => `<img src="${A(`assets/tno/refs/${r}.png`)}" alt="" />`).join("")}</div>
    <div class="arrow4 brass hero">›</div>
    <div class="out plate"><img src="${A(`assets/regen/${tno}/${tno}-v1-p1.png`)}" alt="" /></div>
    ${brand}`,
  // 5: in the game
  `<div class="steel"></div>
    <div class="shot"><img src="${A("assets/ingame/news_event_in_frame.png")}" alt="" /></div>
    <div class="t5 hero"><div class="ice">Made by AI.</div><div class="brass">Loaded by</div><div class="brass">the game.</div></div>
    ${brand}`,
  // 6: the brand, big
  `<div class="steel"></div>
    <div class="glow"></div>
    <img class="biglogo" src="${logo}" alt="" />
    <div class="t6 hero"><div class="ice">HOI4</div><div class="brass">Imagegen</div></div>
    <div class="t6b">Free · open source MCP server for HOI4 modding</div>`,
];

const css = `
    #root .opt { position: absolute; inset: 0; overflow: hidden; }
    #root .brand { position: absolute; left: 70px; bottom: 56px; display: flex; align-items: center; gap: 22px; z-index: 20; }
    #root .brand img { width: 96px; height: 96px; object-fit: contain; filter: drop-shadow(0 8px 14px rgba(0,0,0,.7)); }
    #root .brand span { font-size: 52px; }
    #root .pl { position: absolute; display: flex; align-items: center; justify-content: center; }
    #root .pl img { width: 86%; height: 86%; object-fit: contain; filter: drop-shadow(0 16px 20px rgba(0,0,0,.6)); }
    #root .t1 { position: absolute; left: 80px; top: 250px; font-size: 150px; z-index: 6; }
    #root .fan { position: absolute; left: 1080px; top: 50px; width: 820px; height: 900px; transform: scale(.92); transform-origin: 100% 0; }
    #root .t2 { position: absolute; left: 0; right: 0; top: 70px; text-align: center; font-size: 104px; white-space: nowrap; }
    #root .t2 .arrow { font-size: 120px; }
    #root .raw { position: absolute; left: 60px; top: 300px; width: 1040px; height: 520px; display: flex; align-items: center; justify-content: center; }
    #root .raw img { max-width: 1040px; max-height: 480px; transform: rotate(-3deg); filter: drop-shadow(0 30px 40px rgba(0,0,0,.6)); }
    #root .clampbox { position: absolute; left: 1220px; top: 400px; width: 620px; height: 330px; display: flex; align-items: center; justify-content: center; }
    #root .clampbox img { width: 560px; height: 260px; object-fit: contain; filter: drop-shadow(0 20px 26px rgba(0,0,0,.6)); }
    #root .t2b { position: absolute; left: 1220px; top: 770px; width: 620px; text-align: center; }
    #root .t2b .chip { font-size: 34px; }
    #root .wall { position: absolute; left: -40px; top: -30px; width: 2000px; display: grid; grid-template-columns: repeat(4, 470px); gap: 30px; transform: rotate(-6deg) scale(1.08); transform-origin: 50% 50%; }
    #root .wall .w { width: 470px; height: 360px; display: flex; align-items: center; justify-content: center; }
    #root .wall .w img { width: 82%; height: 82%; object-fit: contain; }
    #root .shade { position: absolute; inset: 0; background: radial-gradient(ellipse 60% 55% at 50% 52%, rgba(8,10,12,.9) 0%, rgba(8,10,12,.55) 60%, rgba(8,10,12,.2) 100%); }
    #root .t3 { position: absolute; left: 0; right: 0; top: 230px; text-align: center; }
    #root .t3 .brass { font-size: 400px; }
    #root .t3 .ice { font-size: 130px; margin-top: 10px; }
    #root .t4 { position: absolute; left: 0; right: 0; top: 80px; text-align: center; font-size: 150px; white-space: nowrap; }
    #root .refs { position: absolute; left: 90px; top: 380px; width: 900px; height: 420px; display: flex; align-items: center; justify-content: space-between; }
    #root .refs img { width: 280px; height: 300px; object-fit: contain; filter: drop-shadow(0 20px 24px rgba(0,0,0,.65)); }
    #root .arrow4 { position: absolute; left: 1010px; top: 470px; font-size: 220px; }
    #root .out { position: absolute; left: 1200px; top: 300px; width: 620px; height: 580px; display: flex; align-items: center; justify-content: center; }
    #root .out img { width: 520px; height: 500px; object-fit: contain; }
    #root .shot { position: absolute; right: -60px; top: 120px; width: 1060px; height: 860px; display: flex; align-items: center; justify-content: center; }
    #root .shot img { max-width: 1060px; max-height: 860px; object-fit: contain; transform: rotate(3deg); box-shadow: 0 40px 80px rgba(0,0,0,.75), 0 0 0 6px #59646b; }
    #root .t5 { position: absolute; left: 70px; top: 200px; font-size: 118px; z-index: 5; }
    #root .glow { position: absolute; inset: 0; background: radial-gradient(ellipse 45% 55% at 30% 50%, rgba(232,181,79,.35), transparent 70%); }
    #root .biglogo { position: absolute; left: 110px; top: 190px; width: 700px; height: 700px; object-fit: contain; filter: drop-shadow(0 40px 60px rgba(0,0,0,.7)); }
    #root .t6 { position: absolute; left: 840px; top: 300px; }
    #root .t6 .ice { font-size: 230px; }
    #root .t6 .brass { font-size: 150px; }
    #root .t6b { position: absolute; left: 870px; top: 800px; font-size: 44px; letter-spacing: .06em; color: #efe6cc; }`;

const html = `<!doctype html>
<html lang="en" data-resolution="landscape">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=1920, height=1080" />
    <script src="https://cdn.jsdelivr.net/npm/gsap@3.15.0/dist/gsap.min.js"></script>
    <style>
      * { margin: 0; padding: 0; box-sizing: border-box; }
      html, body { margin: 0; width: 1920px; height: 1080px; overflow: hidden; background: #0d1318; }
      ${base}
      ${css}
    </style>
  </head>
  <body>
    <div id="root" data-composition-id="thumbs" data-start="0" data-duration="${options.length}" data-width="1920" data-height="1080">
${options.map((o, i) => `      <div id="opt-${i + 1}" class="clip opt" data-start="${i}" data-duration="1" data-track-index="1">
    ${o}
        <div class="vig"></div>
      </div>`).join("\n")}
    </div>
    <script>
      window.__timelines = window.__timelines || {};
      window.__timelines["thumbs"] = gsap.timeline({ paused: true });
    </script>
  </body>
</html>
`;
mkdirSync(out, { recursive: true });
for (const p of used) {
  mkdirSync(dirname(join(out, p)), { recursive: true });
  copyFileSync(join(root, p), join(out, p));
}
writeFileSync(join(out, "index.html"), html);
console.log(`thumbs: ${options.length} options, ${used.size} images`);
