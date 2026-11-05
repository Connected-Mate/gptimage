// Copies queue hand-backs into assets/regen/<asset>/ and records their provenance in assets/regen/index.json.
// Usage: node src/pull-regen.mjs <handback-dir> [<handback-dir> ...]
// Each hand-back folder holds result.json, the raw candidate <asset>-vN.png and the processed <asset>-vN-p1*.png.
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const out = join(root, "assets", "regen");
mkdirSync(out, { recursive: true });
const indexFile = join(out, "index.json");
const index = existsSync(indexFile) ? JSON.parse(readFileSync(indexFile, "utf8")) : {};

// a hand-back holds <family>/result.json, or <family>/<asset>/result.json for several assets of one family
const runs = (base) =>
  readdirSync(base).flatMap((fam) => {
    const dir = join(base, fam);
    if (!statSync(dir).isDirectory()) return [];
    if (existsSync(join(dir, "result.json"))) return [[fam, dir]];
    return readdirSync(dir).map((a) => [fam, join(dir, a)]).filter(([, d]) => statSync(d).isDirectory() && existsSync(join(d, "result.json")));
  });
for (const base of process.argv.slice(2)) {
  for (const [fam, dir] of runs(base)) {
    const r = JSON.parse(readFileSync(join(dir, "result.json"), "utf8"));
    // the first TNO run (0.3.1) kept the vanilla plaque instead of TNO's look, so only a later rerun is used
    if (fam.startsWith("tno_style") && r.tool_version === "0.3.1") continue;
    const all = readdirSync(dir).filter((f) => f.endsWith(".png"));
    const refs = all.filter((f) => /^ref_\d+\.png$/.test(f)).sort();
    const pngs = all.filter((f) => !refs.includes(f));
    // a raw candidate is <asset>-vN.png or <asset>-vN-raw.png; it is stored as <asset>-vN.png
    const raw = pngs.find((f) => /-v1(-raw)?\.png$/.test(f));
    if (!raw) continue;
    const asset = raw.replace(/-v1(-raw)?\.png$/, "");
    const to = join(out, asset);
    mkdirSync(to, { recursive: true });
    for (const f of pngs) copyFileSync(join(dir, f), join(to, f.replace(/-raw\.png$/, ".png")));
    index[asset] = { family: fam, tool_version: r.tool_version, model: r.model, plan_id: r.plan_id ?? r.v1?.plan_id, generated_at: r.generated_at, files: pngs.sort() };
    // the TNO style run: its references (shown with the maintainer's consent) and the name of the icon made from them
    if (fam.startsWith("tno_style")) {
      const tno = join(root, "assets", "tno");
      mkdirSync(join(tno, "refs"), { recursive: true });
      for (const f of refs) copyFileSync(join(dir, f), join(tno, "refs", f));
      writeFileSync(join(tno, "source.json"), JSON.stringify({ asset, refs }, null, 1) + "\n");
    }
    console.log(`${asset}: ${fam}, ${r.tool_version}, ${pngs.length} files${refs.length ? `, ${refs.length} refs` : ""}`);
  }
}
writeFileSync(indexFile, JSON.stringify(index, null, 1) + "\n");
