// Writes src/scenes/animated.html: the four animated-sprite families from PR #6, each playing
// its real frames at the fps its frameAnimatedSpriteType block registers, beside its frame strip.
import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const A = [
  { id: "glow_focus_ready", fam: "anim_blend_glow", frames: 2, fps: 1, w: 141, h: 115, k: 2, blend: true, what: "focus-ready glow" },
  { id: "anim_signal_beacon", fam: "anim_blend_keyframes", frames: 3, fps: 1, w: 141, h: 115, k: 2, blend: true, what: "signal lantern" },
  { id: "scroll_research_progress", fam: "anim_scroll", frames: 9, fps: 15, w: 72, h: 72, k: 3, blend: false, what: "progress chevrons" },
  { id: "spin_industry_cog", fam: "anim_spin", frames: 20, fps: 7, w: 64, h: 64, k: 4, blend: false, what: "industry cog" },
];
const cell = (a, i) => {
  const imgs = Array.from({ length: a.frames }, (_, f) => `<img class="an-f an-${i}" src="assets/anim/${a.id}_${String(f).padStart(2, "0")}.png" style="width:${a.w * a.k}px;height:${a.h * a.k}px" alt="" />`).join("");
  const strip = Array.from({ length: Math.min(a.frames, 6) }, (_, f) => `<img src="assets/anim/${a.id}_${String(f).padStart(2, "0")}.png" alt="" />`).join("");
  return `      <div class="cell plate an-c" style="left:${115 + i * 430}px">
        <div class="live">${imgs}</div>
        <div class="strip">${strip}</div>
        <div class="what">${a.what}</div>
      </div>`;
};
const js = A.map((a, i) => {
  // real engine playback: frame f shows over [f/fps, (f+1)/fps), looping for the scene
  const step = 1 / a.fps;
  return `    { const fr = gsap.utils.toArray(".an-${i}"); const n = fr.length; const step = ${step};
      for (let t = 0, f = 0; t < DUR; t += step, f = (f + 1) % n) {
        const at = Math.round((b(1) + t) * 30) / 30; if (at >= DUR) break;
        ${a.blend ? `tl.to(fr[f], { opacity: 1, duration: step * 0.6, ease: "sine.inOut" }, at); tl.to(fr[(f + n - 1) % n], { opacity: 0, duration: step * 0.6, ease: "sine.inOut" }, at);` : `tl.set(fr, { opacity: 0 }, at); tl.set(fr[f], { opacity: 1 }, at);`}
      } }`;
}).join("\n");
const html = `<template>
  <style>
    {{BASE}}
    #root .t { position: absolute; left: 0; right: 0; top: 80px; text-align: center; font-size: 72px; white-space: nowrap; }
    #root .sub { position: absolute; left: 0; right: 0; top: 196px; text-align: center; font-size: 34px; }
    #root .cell { position: absolute; top: 290px; width: 400px; height: 600px; }
    #root .live { position: absolute; left: 0; right: 0; top: 40px; height: 300px; display: flex; align-items: center; justify-content: center; }
    #root .live img { position: absolute; opacity: 0; }
    #root .strip { position: absolute; left: 20px; right: 20px; top: 370px; display: flex; gap: 6px; align-items: center; justify-content: center; height: 60px; overflow: hidden; }
    #root .strip img { height: 40px; background: rgba(0,0,0,.35); box-shadow: inset 0 0 0 1px rgba(232,181,79,.3); }
    #root .strip span { font-family: "IBM Plex Mono", monospace; font-size: 18px; color: #a8a18f; margin-left: 6px; }
    #root .fam { position: absolute; left: 24px; top: 460px; font-size: 24px; color: #e8b54f; }
    #root .fam b { font-weight: 500; }
    #root .spec { position: absolute; left: 24px; top: 500px; font-size: 20px; color: #c9c2a3; }
    #root .what { position: absolute; left: 0; right: 0; text-align: center; top: 500px; font-size: 38px; letter-spacing: .08em; text-transform: uppercase; color: #efe6cc; }
  </style>
  <div id="root" data-composition-id="animated" data-width="1920" data-height="1080">
    <div class="steel"></div>
    {{GRAIN}}
    <div class="t hero"><span class="ice an-w">Animated sprites.</span> <span class="brass an-w">Loops that hold.</span></div>
    <div class="sub label">one base frame · every frame checked for drift</div>
${A.map(cell).join("\n")}
    <div class="vig"></div>
  </div>
  <script>
    {{LIB}}
    tl.fromTo(".an-w", { opacity: 0, filter: "blur(12px)" }, { opacity: 1, filter: "blur(0px)", duration: 0.15, ease: "power4.out", stagger: b(1) }, 0);
    tl.fromTo(".sub", { opacity: 0 }, { opacity: 1, duration: 0.3 }, b(1));
    tl.fromTo(".an-c", { opacity: 0, y: 60 }, { opacity: 1, y: 0, duration: b(0.5), ease: "expo.out", stagger: b(0.25) }, b(0.5));
${js}
    window.__timelines["animated"] = tl;
  </script>
</template>
`;
writeFileSync(join(here, "scenes", "animated.html"), html);
console.log("animated: 4 families");
