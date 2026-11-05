// Writes src/scenes/more.html: every generated family not shown on its own elsewhere, on one screen.
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const G = "assets/gallery";
// family -> asset; the families in the drop, the 3 import-only ones and the MIO and agency upgrade families are left
// out, and so is every family on the config's hold list until its regeneration lands
const hold = new Set(JSON.parse(readFileSync(join(here, "config.json"), "utf8")).hold || []);
const all = [
  ["idea_spirit", "war_economy"], ["idea_concern", "ration_crisis"], ["officer_corps_spirit", "professional_officers"],
  ["decision", "mobilise_reserves"], ["decision_category_icon", "military_reforms"], ["decision_category_picture", "war_industry"],
  ["state_modifier", "fortified_region"], ["special_project_icon", "rocket_programme"], ["raid", "sabotage"],
  ["bop_side", "defenders_of_order"], ["focus", "steel_gate"],
  ["operation_icon", "infiltrate_ministry"], ["operation_picture", "night_infiltration"], ["commander_trait", "armour_expert"],
  ["personal_trait", "iron_will"], ["doctrine", "mobile_warfare"], ["technology", "heavy_tank"],
  ["super_event", "armistice"], ["gui_texture", "panel_leather"],
  ["modifier_stat_icon", "supply_bonus"], ["building", null], ["medal", null],
];
// families shown with an image chosen from a later gallery run (assets/picked/): a 0.3.1 image stands in for a held
// family, and a 0.3.2 image replaces the family's first run
const chosen = {
  idea_spirit: "women_welders", idea_concern: "shell_filling_hall", commander_trait: "single_tank_side",
  officer_corps_spirit: "staff_college_desk", doctrine: "howitzer_battery_night", raid: "paratrooper_canopy", special_project_icon: "super_heavy_tank_hall", operation_icon: "telephone_with_bug",
};
const picked = (fam) => (chosen[fam] && existsSync(join(here, "..", `assets/picked/${fam}/${chosen[fam]}-v1-p1.png`)) ? `assets/picked/${fam}/${chosen[fam]}-v1-p1.png` : null);
// Every tile shows the processed file the game loads (<asset>-v1-p1.png), so the art keeps its in-game detail.
// The family's newest vanilla-form regeneration (latest build, then latest run, assets/regen/<asset>/) replaces the
// gallery's first run. A family with neither (building, medal) appears only once its regeneration is pulled.
const index = existsSync(join(here, "..", "assets/regen/index.json")) ? JSON.parse(readFileSync(join(here, "..", "assets/regen/index.json"), "utf8")) : {};
// the regeneration shown when a family's hand-back holds several
const regenPick = { building: "japanese_five_tier_pagoda", operation_picture: "platform_suitcase_swap", medal: "laurel_wreath_v033", focus: "embassy_flags_handshake_v033" };
const regen = (fam) => {
  const a = (index[regenPick[fam]] && regenPick[fam]) || Object.entries(index).filter(([, e]) => e.family === fam).sort(([, x], [, y]) => (y.tool_version || "").localeCompare(x.tool_version || "", undefined, { numeric: true }) || (y.generated_at || "").localeCompare(x.generated_at || ""))[0]?.[0];
  return a && existsSync(join(here, "..", `assets/regen/${a}/${a}-v1-p1.png`)) ? `assets/regen/${a}/${a}-v1-p1.png` : null;
};
const source = (fam, asset) => picked(fam) || regen(fam) || (asset && existsSync(join(here, "..", `${G}/processed_png/${asset}-v1-p1.png`)) ? `${G}/processed_png/${asset}-v1-p1.png` : null);
const fams = all.filter(([fam]) => !hold.has(fam) || picked(fam)).map(([fam, asset]) => [fam, source(fam, asset)]).filter(([, src]) => src);
// up to 18 tiles in rows of 5 or 6; more shrink into rows of 8 so the grid stays under the title
const gap = 20;
const cols = fams.length > 18 ? 8 : fams.length % 6 === 0 || fams.length > 15 ? 6 : 5;
const TW = Math.min(260, Math.floor((1728 - (cols - 1) * gap) / cols)), TH = Math.round((TW * 240) / 260), AW = TW - 34, AH = TH - 34;
// a building file is a strip of frames half as wide as they are tall (intact, then damaged): the tile shows the first
const frames = (fam, w, h) => (fam === "building" ? Math.round(w / (h / 2)) : 1);
const size = (p) => { const b = readFileSync(join(here, "..", p)); return [b.readUInt32BE(16), b.readUInt32BE(20)]; };
// enlarged to fit the tile, at most 5x, and smoothly scaled as the game's UI scaling does
const tiles = fams.map(([fam, src]) => {
  const [sw, h] = size(src), n = frames(fam, sw, h), w = sw / n;
  const k = Math.min(5, AW / w, AH / h);
  const art = n > 1
    ? `<div style="width:${Math.round(w * k)}px;height:${Math.round(h * k)}px;background:url(${src}) 0 0/${n * 100}% 100% no-repeat"></div>`
    : `<img src="${src}" style="width:${Math.round(w * k)}px;height:${Math.round(h * k)}px" alt="" />`;
  return `      <div class="c plate mo-c">${art}</div>`;
});
const gridW = cols * TW + (cols - 1) * gap;
const gridH = Math.ceil(fams.length / cols) * (TH + gap) - gap;
// centred in the space under the title
const gridTop = Math.round(190 + (1000 - 190 - gridH) / 2);
const html = `<template>
  <style>
    {{BASE}}
    /* inside the 90% action-safe box: x 96-1824, y 54-1026 */
    #root .t { position: absolute; left: 96px; right: 96px; top: 60px; text-align: center; font-size: 76px; white-space: nowrap; }
    #root .grid { position: absolute; left: ${(1920 - gridW) / 2}px; top: ${gridTop}px; width: ${gridW}px; display: flex; flex-wrap: wrap; justify-content: center; gap: ${gap}px; }
    #root .c { position: relative; width: ${TW}px; height: ${TH}px; display: flex; align-items: center; justify-content: center; }
    #root .c > img, #root .c > div { filter: drop-shadow(0 10px 14px rgba(0,0,0,.55)); }
    #root .c .f { position: absolute; left: 6px; right: 6px; bottom: 8px; text-align: center; font-family: "IBM Plex Mono", monospace; font-size: 16px; line-height: 1.15; color: #c9c2a3; }
  </style>
  <div id="root" data-composition-id="more" data-width="1920" data-height="1080">
    <div class="paper"></div>
    {{GRAIN}}
    <div class="t hero"><span class="ice">And</span> <span class="brass" id="mo-n">many more</span></div>
    <div class="grid">
${tiles.join("\n")}
    </div>
    <div class="vig"></div>
  </div>
  <script>
    {{LIB}}
    tl.fromTo("#mo-n", { opacity: 0, scale: 2.2 }, { opacity: 1, scale: 1, duration: b(0.5), ease: "back.out(1.6)" }, 0);
    // the tiles fall into place like dominoes: a wave from the top left across the grid, over two beats
    tl.fromTo(".mo-c", { opacity: 0, y: 60, scale: 0.85, rotation: -3 }, { opacity: 1, y: 0, scale: 1, rotation: 0, duration: b(1.25), ease: "expo.out", stagger: { amount: b(2), grid: "auto", from: "start" } }, b(0.5));
    window.__timelines["more"] = tl;
  </script>
</template>
`;
writeFileSync(join(here, "scenes", "more.html"), html);
console.log(`more: ${fams.length} families, grid ${gridW}px`);
