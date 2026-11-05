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

// the varied set for the main thumbnail: one of each kind of HOI4 art, as the processed file the game loads, and one
// portrait from the ComfyUI HOI4 portrait workflow (the trailer's comparison)
const V = {
  portrait: "assets/cmp/mulcahy_comfyui.png",
  focus: "assets/regen/embassy_flags_handshake_v033/embassy_flags_handshake_v033-v1-p1.png",
  report: "assets/gallery/processed_png/winter_border-v1-p1.png",
  flag: "assets/gallery/processed_png/IRP_fascism-v1-p1.png",
  tech: "assets/gallery/processed_png/heavy_tank-v1-p1.png",
  medal: "assets/regen/laurel_wreath_v033/laurel_wreath_v033-v1-p1.png",
  emblem: "assets/picked/division_emblem/bear_standing-v1-p1.png",
};
// a plate that frames only the visible part of a file with wide transparent margins: box is [x0, y0, x1, y1] in its pixels
const size = (p) => { const b = readFileSync(join(root, p)); return [b.readUInt32BE(16), b.readUInt32BE(20)]; };
const framed = (src, box, w, h, extra = "") => {
  const [iw, ih] = size(src), bw = box[2] - box[0], bh = box[3] - box[1], k = Math.min((w * 0.82) / bw, (h * 0.82) / bh);
  const left = Math.round(w / 2 - (box[0] + bw / 2) * k), top = Math.round(h / 2 - (box[1] + bh / 2) * k);
  return `<div class="pl plate" style="width:${w}px;height:${h}px;overflow:hidden;${extra}"><img src="${A(src)}" style="position:absolute;left:${left}px;top:${top}px;width:${Math.round(iw * k)}px;height:${Math.round(ih * k)}px" alt="" /></div>`;
};
const MEDAL_BOX = [11, 14, 44, 70], EMBLEM_BOX = [26, 4, 50, 39];
const card = (src, w, h, extra = "") => `<div class="pl plate card" style="width:${w}px;height:${h}px;${extra}"><img src="${A(src)}" alt="" /></div>`;
const title1 = `<div class="t1 hero"><div class="ice">AI art</div><div class="brass">ready for</div><div class="brass">HOI4</div></div>`;

const options = [
  // 1: the promise, with a fan of varied finished art and a ComfyUI portrait
  `<div class="table"></div>
    ${title1}
    <div class="fan">
      ${plate(V.focus, 300, 300, "left:0;top:20px;transform:rotate(-7deg);z-index:3")}
      ${framed(V.medal, MEDAL_BOX, 210, 290, "left:590px;top:0;transform:rotate(6deg);z-index:4")}
      ${plate(V.report, 330, 290, "left:-20px;top:380px;transform:rotate(-5deg);z-index:5")}
      ${plate(V.flag, 260, 190, "left:580px;top:330px;transform:rotate(7deg);z-index:6")}
      ${plate(V.tech, 420, 190, "left:60px;top:690px;transform:rotate(-3deg);z-index:7")}
      ${framed(V.emblem, EMBLEM_BOX, 280, 200, "left:500px;top:620px;transform:rotate(5deg);z-index:8")}
      ${card(V.portrait, 300, 400, "left:255px;top:200px;transform:rotate(2deg);z-index:10")}
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
  // 7: option 1's title with the varied set in a tidy grid, the portrait as its own column
  `<div class="table"></div>
    ${title1}
    <div class="grid7">
      ${card(V.portrait, 300, 420, "left:560px;top:0")}
      ${plate(V.focus, 260, 200, "left:0;top:0")}
      ${framed(V.medal, MEDAL_BOX, 260, 200, "left:280px;top:0")}
      ${plate(V.report, 260, 200, "left:0;top:220px")}
      ${plate(V.flag, 260, 200, "left:280px;top:220px")}
      ${plate(V.tech, 540, 200, "left:0;top:440px")}
      ${framed(V.emblem, EMBLEM_BOX, 300, 200, "left:560px;top:440px")}
    </div>
    ${brand}`,
  // 8: option 1's title with the portrait as the hero and the other kinds around it
  `<div class="table"></div>
    ${title1}
    <div class="orbit">
      ${card(V.portrait, 400, 530, "left:220px;top:150px;transform:rotate(-2deg);z-index:10")}
      ${plate(V.focus, 250, 250, "left:0;top:40px;transform:rotate(-8deg);z-index:4")}
      ${framed(V.medal, MEDAL_BOX, 190, 260, "left:640px;top:60px;transform:rotate(7deg);z-index:4")}
      ${plate(V.flag, 230, 170, "left:20px;top:370px;transform:rotate(-6deg);z-index:5")}
      ${plate(V.report, 290, 250, "left:610px;top:390px;transform:rotate(5deg);z-index:5")}
      ${plate(V.tech, 380, 170, "left:40px;top:720px;transform:rotate(-3deg);z-index:6")}
      ${framed(V.emblem, EMBLEM_BOX, 250, 180, "left:560px;top:720px;transform:rotate(4deg);z-index:6")}
    </div>
    ${brand}`,
];

const css = `
    #root .opt { position: absolute; inset: 0; overflow: hidden; }
    #root .brand { position: absolute; left: 70px; bottom: 56px; display: flex; align-items: center; gap: 22px; z-index: 20; }
    #root .brand img { width: 96px; height: 96px; object-fit: contain; filter: drop-shadow(0 8px 14px rgba(0,0,0,.7)); }
    #root .brand span { font-size: 52px; }
    #root .pl { position: absolute; display: flex; align-items: center; justify-content: center; }
    #root .pl img { width: 86%; height: 86%; object-fit: contain; filter: drop-shadow(0 16px 20px rgba(0,0,0,.6)); }
    #root .t1 { position: absolute; left: 80px; top: 250px; font-size: 150px; z-index: 6; }
    #root .card img { width: 92%; height: 92%; object-fit: cover; object-position: 50% 20%; filter: none; }
    #root .grid7 { position: absolute; left: 1060px; top: 150px; width: 860px; height: 640px; transform: scale(.9); transform-origin: 0 0; }
    #root .orbit { position: absolute; left: 1010px; top: 40px; width: 860px; height: 940px; transform: scale(.94); transform-origin: 100% 0; }
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
