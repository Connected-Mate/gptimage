#!/usr/bin/env python3
"""Replace the generated examples of some families in the published gallery, and leave every other image alone.

  gallery_patch_families.py <repo root> <family> [<family> ...] [--exclude family/asset,...] [--drop section/family ...]

build_gallery_site.py rebuilds the whole site from every run folder. This script is for a run of a few families: it takes
their results from .work/big-results (the vanilla section), runs the same checks (validation passed, processed on a current
build, the tool's vanilla-match check passed), replaces their images and manifest entries in docs/gallery/big/, and rewrites
README.md with `build_gallery_site.py --pages-only`. `--exclude` leaves out examples a reviewer rejected (a bad picture, or one of the wrong kind). `--drop section/family` removes a section's family page (its generated
examples and references), for a mod style whose examples are not made yet.

A balance of power side is shown as its stage set: every emblem gives one row of four images (standard, improved, and the
cracked pair), the way vanilla's own sides come in pairs.

Environment as for make_gallery_big.py (HOI4_IMAGEGEN_BIN, GALLERY_MOD).
"""
import json
import os
import re
import shutil
import subprocess
import sys

from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import build_gallery_site as g  # noqa: E402
import make_gallery_big as m  # noqa: E402

GAME = os.environ.get("HOI4_GAME_DIR", "C:/Program Files (x86)/Steam/steamapps/common/Hearts of Iron IV")
STAGES = [("", "standard"), ("_improved", "improved"), ("_bad", "bad"), ("_bad_improved", "bad improved")]
# vanilla's own pairs, as the references beside the stage sets (four stages of one emblem per row)
BOP_VANILLA = [
    ("bop_FIN_mannerheim_good_side", "bop_FIN_mannerheim_good_improved_side", "bop_FIN_mannerheim_bad_side", "bop_FIN_mannerheim_bad_improved_side"),
    ("bop_CZE_army_readiness_good_side", "bop_CZE_army_readiness_good_improved_side", "bop_CZE_army_readiness_bad_side", "bop_CZE_army_readiness_bad_improved_side"),
]
# vanilla's operation cards (the 85x85 `icon = GFX_operations_<name>` pictures)
OPERATION_VANILLA = ["steal_blueprints", "fake_intel", "capture_ciphers", "coordinated_strike"]


def read(path):
    from PIL import Image as I  # DDS through Pillow

    return I.open(path).convert("RGBA")


def clear_family(out, section, fam, keep_refs=True):
    folder = os.path.join(out, section, fam)
    if not os.path.isdir(folder):
        return
    for f in os.listdir(folder):
        if keep_refs and re.match(r"(vanilla|reference)_\d+(@\dx)?\.webp$", f):
            continue
        os.remove(os.path.join(folder, f))


def reset_vanilla_refs(out, fam, images):
    folder = os.path.join(out, "vanilla", fam)
    os.makedirs(folder, exist_ok=True)
    for f in os.listdir(folder):
        if re.match(r"vanilla_\d+(@\dx)?\.webp$", f):
            os.remove(os.path.join(folder, f))
    for n, img in enumerate(images, 1):
        tmp = os.path.join(folder, f"_tmp_{n}.png")
        img.save(tmp)
        g.webp(tmp, os.path.join(folder, f"vanilla_{n}.webp"))
        os.remove(tmp)


def main(argv):
    exclude = set()
    if "--exclude" in argv:
        i = argv.index("--exclude")
        exclude = set(argv[i + 1].split(","))
        del argv[i : i + 2]
    drops = []
    if "--drop" in argv:
        i = argv.index("--drop")
        drops = argv[i + 1 :]
        argv = argv[:i]
    repo, fams = argv[0], argv[1:]
    out = os.path.join(repo, "docs", "gallery", "big")
    mpath = os.path.join(out, "manifest.json")
    manifest = json.load(open(mpath, encoding="utf-8"))
    cache = json.load(open(g.CACHE, encoding="utf-8")) if os.path.exists(g.CACHE) else {}
    drop_keys = {tuple(d.split("/")) for d in drops}
    keep = [e for e in manifest["images"] if not (e["section"] == "vanilla" and e["family"] in fams) and (e["section"], e["family"]) not in drop_keys]
    for sec, fam in drop_keys:
        shutil.rmtree(os.path.join(out, sec, fam), ignore_errors=True)
    new = []
    for fam in fams:
        clear_family(out, "vanilla", fam)
        folder = os.path.join(out, "vanilla", fam)
        os.makedirs(folder, exist_ok=True)
        for fam_, asset, rec in g.entries(m.MOD, m.RES, [fam]):
 
            if f"{fam}/{asset}" in exclude:
                print("excluded", fam, asset, flush=True)
                continue
            made, processed = rec.get("tool_version") or "0", rec.get("process_version") or rec.get("tool_version") or "0"
            if made < "0.3" or processed < g.MIN_BUILD or rec.get("validation") != "pass" or not rec.get("processed"):
                print("skip", fam, asset, "not passed", flush=True)
                continue
            vc = g.vanilla_check(m.MOD, rec["processed"], cache)
            if vc["status"] not in ("pass", "unavailable"):
                print("skip", fam, asset, "vanilla check", vc["status"], vc.get("outside"), flush=True)
                continue
            proc = rec["process"]
            parts = [("", os.path.join(m.MOD, proc["path"]))]
            if fam == "bop_side":
                comp = {c["key"]: os.path.join(m.MOD, c["path"]) for c in proc.get("companions", [])}
                if any(k not in comp for k, _ in STAGES[1:]):
                    print("skip", fam, asset, "stages missing", flush=True)
                    continue
                parts = [(k, comp[k] if k else parts[0][1]) for k, _ in STAGES]
            for key, src in parts:
                name = asset + key
                w, h = Image.open(src).size
                native = g.webp(src, os.path.join(folder, name + ".webp"))
                k = g.scale_for(w, h)
                enlarged = g.webp(src, os.path.join(folder, name + f"@{k}x.webp"), k) if k > 1 else None
                label = dict(STAGES)[key] if fam == "bop_side" else ""
                new.append({"section": "vanilla", "family": fam, "asset": name, "brief": rec.get("brief", "") + (f" ({label} stage)" if label else ""),
                            "build": made, "processed_on": processed, "size": [w, h], "vanilla_check": vc["status"], "subfamily": None,
                            "native": f"vanilla/{fam}/{name}.webp", "enlarged": f"vanilla/{fam}/{name}@{k}x.webp" if enlarged else None, "scale": k})
        print(fam, sum(1 for e in new if e["family"] == fam), "images", flush=True)
    # vanilla references of the right sprite type
    if "bop_side" in fams:
        imgs = [read(f"{GAME}/gfx/interface/bop/{n}.dds") for row in BOP_VANILLA for n in row]
        reset_vanilla_refs(out, "bop_side", imgs)
    if "operation_icon" in fams:
        reset_vanilla_refs(out, "operation_icon", [read(f"{GAME}/gfx/interface/operations/{n}.dds") for n in OPERATION_VANILLA])
    order = list(m.briefs())
    key = lambda e: (["vanilla", "tno", "tfr"].index(e["section"]) if e["section"] in ("vanilla", "tno", "tfr") else 9, order.index(e["family"]) if e["family"] in order else 99)  # noqa: E731
    manifest["images"] = sorted(keep + new, key=key)
    json.dump(manifest, open(mpath, "w", encoding="utf-8", newline="\n"), indent=1, ensure_ascii=False)
    subprocess.run([sys.executable, os.path.join(os.path.dirname(os.path.abspath(__file__)), "build_gallery_site.py"), repo, "--pages-only"], check=True)


if __name__ == "__main__":
    main(sys.argv[1:])
