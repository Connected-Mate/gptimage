#!/usr/bin/env python3
"""Build the public gallery under <repo>/docs/gallery/big/ from the gallery runs.

  build_gallery_site.py <repo root>

An image is published when it was processed on hoi4-imagegen 0.3.4 or later (an image generated earlier counts once
its raw candidate was processed again on a current build, which .work/restamp.py records as `process_version`), passes
validation, and passes the tool's own vanilla-match check (hoi4.image_preview's vanilla_check; families the check cannot measure are marked so). Families
that wait for a later build are left out. Each family page shows a few vanilla references from the installed game
beside the generated examples, at native size and enlarged by a whole number when small. The TNO section shows only
generated images: TNO's own art is not reproduced. Environment as for make_gallery_big.py; TNO_MOD and TNO_RES point
at the TNO run.
"""
import json
import os
import shutil
import subprocess
import sys

from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import make_gallery_big as m  # noqa: E402

HELD = set()  # families waiting for a later build (the four held for 0.3.4 ran on it on 2026-11-06)
CACHE = os.path.join(m.WORK, "vanilla-check-cache.json")
COMPARE = os.environ.get("GALLERY_COMPARE", "")
MIN_BUILD = "0.3.4"  # older processing is hidden unless the image was processed again on a current build
STYLES = json.load(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "gallery_styles.json"), encoding="utf-8"))["styles"]


def cli_at(mod, *args):
    out = subprocess.run([m.BIN, "--project-root", mod, "--json", *args], capture_output=True, text=True, timeout=600, encoding="utf-8")
    try:
        return json.loads(out.stdout.strip())
    except json.JSONDecodeError:
        return {"error": {"message": (out.stdout + out.stderr).strip()[:300]}}


def vanilla_check(mod, artifact, cache):
    key = f"{mod}|{artifact}"
    if key not in cache:
        r = cli_at(mod, "call", "hoi4.image_preview", json.dumps({"target": artifact}))
        vc = r.get("vanilla_check") or {}
        cache[key] = {"status": vc.get("status") or ("unavailable" if "error" not in r else "error"), "outside": vc.get("outside", [])}
        json.dump(cache, open(CACHE, "w", encoding="utf-8"), indent=0)
    return cache[key]


def webp(src, dst, scale=1):
    img = Image.open(src).convert("RGBA")
    if scale > 1:
        img = img.resize((img.width * scale, img.height * scale), Image.NEAREST)
    img.save(dst, "WEBP", lossless=max(img.size) <= 256, quality=85)
    return list(img.size)


def scale_for(w, h):
    big = max(w, h)
    return 4 if big <= 40 else 3 if big <= 70 else 2 if big <= 128 else 1


def entries(mod, res, fams):
    for fam in fams:
        d = os.path.join(res, fam)
        if not os.path.isdir(d):
            continue
        for f in sorted(os.listdir(d)):
            if f.endswith(".json"):
                yield fam, f[:-5], json.load(open(os.path.join(d, f), encoding="utf-8"))


def build(repo):
    out = os.path.join(repo, "docs", "gallery", "big")
    if os.path.isdir(out):
        shutil.rmtree(out)
    os.makedirs(out)
    cache = json.load(open(CACHE, encoding="utf-8")) if os.path.exists(CACHE) else {}
    order = list(m.briefs())
    sections = [("vanilla", m.MOD, m.RES, [f for f in order if f not in HELD and not f.startswith("anim_")])]
    for style in STYLES:  # further mod styles (TNO now, others later) from scripts/gallery_styles.json
        mod, res = os.environ.get(style["mod_env"], ""), os.environ.get(style["res_env"], "")
        if res and os.path.isdir(res):
            sections.append((style["key"], mod, res, [f for f in order if os.path.isdir(os.path.join(res, f))]))
    index, manifest = {}, []
    for section, mod, res, fams in sections:
        for fam, asset, rec in entries(mod, res, fams):
            made, processed = rec.get("tool_version") or "0", rec.get("process_version") or rec.get("tool_version") or "0"
            if made < "0.3" or processed < MIN_BUILD or rec.get("validation") != "pass" or not rec.get("processed"):
                continue
            vc = vanilla_check(mod, rec["processed"], cache)
            if vc["status"] not in ("pass", "unavailable"):
                continue
            src = os.path.join(mod, rec["process"]["path"])
            if not os.path.exists(src):
                continue
            folder = os.path.join(out, section, fam)
            os.makedirs(folder, exist_ok=True)
            w, h = Image.open(src).size
            native = webp(src, os.path.join(folder, asset + ".webp"))
            k = scale_for(w, h)
            enlarged = webp(src, os.path.join(folder, asset + f"@{k}x.webp"), k) if k > 1 else None
            e = {"section": section, "family": fam, "asset": asset, "brief": rec.get("brief", ""), "build": made, "processed_on": processed,
                 "size": [w, h], "vanilla_check": vc["status"], "subfamily": ((rec.get("anchors") or [{}])[0] or {}).get("subfamily"),
                 "native": f"{section}/{fam}/{asset}.webp", "enlarged": f"{section}/{fam}/{asset}@{k}x.webp" if enlarged else None, "scale": k}
            index.setdefault((section, fam), []).append(e)
            manifest.append(e)
    # vanilla references beside each vanilla family (a few files from the installed game, credited on every page)
    refs = {}
    if COMPARE:
        for (section, fam) in [x for x in index if x[0] == "vanilla"]:
            picks = sorted(f for f in os.listdir(COMPARE) if f.startswith(fam + "_vanilla_") and f.endswith(".png") and "_4x" not in f and "_2x" not in f)[:4]
            for n, f in enumerate(picks, 1):
                folder = os.path.join(out, "vanilla", fam)
                webp(os.path.join(COMPARE, f), os.path.join(folder, f"vanilla_{n}.webp"))
                refs.setdefault(fam, []).append(f"vanilla_{n}.webp")
    write_pages(out, index, refs)
    json.dump({"about": "Published gallery images: generated on hoi4-imagegen 0.3 or later and processed on 0.3.4 or later, validation passed, the tool's vanilla-match check passed (or not measurable for the family). build is the generation build, processed_on the processing build.",
               "images": manifest}, open(os.path.join(out, "manifest.json"), "w", encoding="utf-8", newline="\n"), indent=1, ensure_ascii=False)
    print(len(manifest), "images,", len(index), "family pages")


NOTE = ("Every image here was made by `hoi4-imagegen` on the ChatGPT login, then processed, validated and passed by the tool's own check"
        " against the game's art. Nothing is retouched. Only images processed on version 0.3.4 or later that pass both checks are shown:"
        " an image generated on an older build was processed again from its raw candidate. The gallery grows as more runs finish."
        " Hover over an image for its prompt, or open the prompts under each family. Click an image to open it enlarged.")


def caption(e):
    """A short caption: the asset name in words."""
    return e["asset"].replace("_", " ")


def stamp(e):
    return f"build {e['build']}" + (f", processed on {e['processed_on']}" if e["processed_on"] != e["build"] else "")


def anchor(text):
    return text.lower().replace(" ", "-").replace("_", "-")


def write_pages(out, index, refs):
    by_section = {}
    for (section, fam), items in index.items():
        by_section.setdefault(section, []).append((fam, items))
    titles = {"vanilla": ("Vanilla style", "Each family in the game's own style, beside a few vanilla references from *Hearts of Iron IV* by Paradox Interactive.")}
    titles.update({st["key"]: (st["title"], st["intro"]) for st in STYLES})
    order = ["vanilla"] + [st["key"] for st in STYLES]
    lines = ["# Gallery", "", NOTE, "", "## Contents", ""]
    for sec in order:
        if sec not in by_section:
            continue
        title = titles[sec][0]
        lines.append(f"* [{title}](#{anchor(title)}): " + ", ".join(f"[{fam}](#{anchor(sec + ' ' + fam)})" for fam, _ in by_section[sec]))
    for sec in order:
        if sec not in by_section:
            continue
        title, intro = titles[sec]
        lines += ["", f"## {title}", "", intro]
        for fam, items in by_section[sec]:
            lines += ["", f'<a id="{anchor(sec + " " + fam)}"></a>', "", f"### {fam}" + ("" if sec == "vanilla" else f" ({title})"), ""]
            if sec == "vanilla" and refs.get(fam):
                lines += ["Vanilla: " + " ".join(f'<img src="vanilla/{fam}/{r}" alt="vanilla {fam}" title="Hearts of Iron IV (Paradox Interactive)">' for r in refs[fam]), ""]
            cells, prompts = [], []
            for e in items:
                brief = e["brief"].replace("\n", " ").strip()
                tip = brief.replace('"', "'") + f" ({stamp(e)}, {e['size'][0]}x{e['size'][1]})"
                img = f'<img src="{e["native"]}" alt="{caption(e)}" title="{tip}">'
                cells.append(f'<a href="{e["enlarged"] or e["native"]}">{img}</a>')
                prompts.append(f"| {caption(e)} | {stamp(e)} | {brief.replace('|', '/')} |")
            # short captions on the images, the full prompt on hover and in a folded table under the family
            lines += ["Generated: " + " ".join(cells), "",
                      f"<details><summary>Prompts ({len(items)})</summary>", "", "| Image | Build | Prompt |", "|---|---|---|", *prompts, "", "</details>", ""]
    lines += ["", "Families still waiting for a later build are not shown yet. [`manifest.json`](manifest.json) lists every image with its brief, build and check result."]
    with open(os.path.join(out, "README.md"), "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(lines) + "\n")


if __name__ == "__main__":
    build(sys.argv[1])
