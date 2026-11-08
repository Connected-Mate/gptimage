#!/usr/bin/env python3
"""Turn the gallery run (scripts/make_gallery.py) into the committed gallery and its documentation.

  scripts/make_gallery_docs.py export   write docs/gallery/<family>.webp, contact-sheet.webp and manifest.json
  scripts/make_gallery_docs.py docs     write docs/families.md (needs the export first)

Reads .work/gallery-results/*.json and .work/gallery-mod (the scratch mod), plus the built-in family data from
the binary and the vanilla evidence in crates/imagegen-core/src/lookup.rs. Nothing here draws or edits art:
the images are the tool's processed output, shown on a dark tile and enlarged by whole numbers (nearest
neighbour) when they are small.
"""
import datetime
import hashlib
import json
import math
import os
import re
import subprocess
import sys

from PIL import Image, ImageDraw, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
WORK = os.path.join(ROOT, ".work")
MOD = os.path.join(WORK, "gallery-mod")
RES = os.path.join(WORK, "gallery-results")
OUT = os.path.join(ROOT, "docs", "gallery")
BIN = os.environ.get("HOI4_IMAGEGEN_BIN") or (
    os.path.join(WORK, "hoi4-imagegen.exe") if os.path.exists(os.path.join(WORK, "hoi4-imagegen.exe")) else "hoi4-imagegen"
)
TILE_BG = (38, 39, 46)
GROUPS = [
    ("icon", "Icons", "hoi4.image_icon"),
    ("picture", "Pictures", "hoi4.image_picture"),
    ("portrait", "Portraits", "hoi4.image_portrait"),
    ("flag", "Flags", "hoi4.image_flag"),
    ("unit", "Units, logos and buildings", "hoi4.image_unit"),
    ("interface", "Interface", "hoi4.image_interface"),
]

CHECKS = {
    "size": "canvas size is the family's size or an accepted alternative (`GFX-CANVAS-SIZE`)",
    "alpha": "real transparency from pixels, and a subject in it (`GFX-NO-ALPHA`, `GFX-EMPTY-SUBJECT`)",
    "opaque_canvas": "the subject is not an opaque rectangle (`GFX-OPAQUE-CANVAS`)",
    "fringe": "no magenta or green key-colour fringe on the edge (`GFX-CHROMA-FRINGE`)",
    "margin": "the subject keeps the family's margin (`GFX-MARGIN`)",
    "fill": "the subject is not undersized in its frame (`GFX-UNDERSIZED-SUBJECT`)",
    "checker": "no drawn checkerboard (`GFX-FAKE-CHECKERBOARD`)",
    "ghost": "the subject is not semi-transparent (`GFX-GHOSTED`)",
    "opaque": "no translucent pixel in an opaque picture (`GFX-ALPHA-IN-OPAQUE`)",
    "near_opaque": "the picture is opaque to within the template's own alpha (`GFX-ALPHA-IN-OPAQUE`)",
    "card_corners": "transparent card corners (`GFX-CARD-CORNERS`)",
    "frames": "the width divides into the frames (`GFX-FRAME-WIDTH`)",
    "line_art": "a special project drawing has no filled area (`GFX-FILLED-INTERIOR`)",
    "unit_material": "finished unit art is vanilla's green or grey material with the NATO box (`GFX-UNIT-MATERIAL`)",
    "side_view": "a technology picture is as wide as vanilla's side views, a warning only (`GFX-TECH-VIEW`)",
    "style": "distance from the family's style references, a warning only (`GFX-STYLE-DRIFT`, `GFX-STYLE-MATCH`)",
    "dds": "DDS header and layout (`GFX-DDS-HEADER`, `GFX-TRUNCATED`, `GFX-LFS-POINTER`)",
    "tga": "TGA format and bottom-left origin (`GFX-TGA-FORMAT`, `GFX-FLAG-ORIGIN`)",
}
RECIPES = {
    "icon": "trim, fit to the canvas with the profile margin, grayscale where the family needs it",
    "picture": "flatten on black, cover-resize, grayscale for news",
    "report_card": "flatten, then the report card: sepia, grain, tilt and a soft shadow fitted to vanilla, with transparent corners",
    "achievement": "fit the subject, then the grey and not-eligible states on the mod's template backgrounds (three DDS)",
    "advisor_card": "flatten, then the portrait composited into the mod's advisor template, with an alpha-coverage audit that fails closed",
    "portrait": "flatten, then a head-and-shoulders crop found with YuNet face detection",
    "flag": "flatten, then the 82x52, 41x26 and 10x7 ladder (three bottom-left TGA)",
    "selected_strip": "fit with a margin, then frame 2 as the selected state",
    "landmark_strip": "fit, then frame 2 as the ruined landmark from a second candidate",
    "imported_art": "import only: the exact strip, or one file per frame",
    "raid": "trim, fit, then vanilla's gold gradient and black outline",
    "doctrine": "trim, fit, then black units with vanilla's pale outline and glow",
    "line_art": "the image redrawn as vanilla's thin pale lines, then grayscale",
    "unit_counter": "the silhouette in vanilla's material, then the NATO box as frame 2",
    "division_symbol": "the silhouette in vanilla's green material",
    "phase_card": "flatten, then the report card on vanilla's phase picture mask",
}
KIND = {
    "full_name": "the script names the sprite in full, so the name is free",
    "short_key": "the game builds the sprite name from a short key in script",
    "engine_key": "the game builds the sprite name from the key of the object it belongs to",
    "numbered": "the game picks the sprite by number",
    "file_name": "the game loads the file by name, no sprite",
}


def sha256(path):
    return hashlib.sha256(open(path, "rb").read()).hexdigest()


def cli(*args):
    out = subprocess.run([BIN, "--project-root", MOD, "--json", *args], capture_output=True, text=True, encoding="utf-8")
    return json.loads(out.stdout)


def results():
    rows = {}
    for name in sorted(os.listdir(RES)):
        if name.endswith(".json"):
            rec = json.load(open(os.path.join(RES, name), encoding="utf-8"))
            if rec.get("install") and not rec["install"].get("error"):
                rows[rec["family"]] = rec
    return rows


def family_order():
    fams = cli("families")["families"]
    order = []
    for key, _, _ in GROUPS:
        order += [f["family"] for f in fams if f["group"] == key]
    return order


def display(family, rec):
    src = os.path.join(MOD, rec["process"]["path"])
    img = Image.open(src).convert("RGBA")
    w, h = img.size
    scale = max(1, min(8, math.ceil(220 / max(w, h)))) if max(w, h) < 220 else 1
    if scale > 1:
        img = img.resize((w * scale, h * scale), Image.NEAREST)
    tile = Image.new("RGB", img.size, TILE_BG)
    tile.paste(img, mask=img.getchannel("A"))
    return tile, (w, h), scale


def parse_note(note):
    def grab(key):
        m = re.search(key + r": ([^;]+?)(?:;|$)", note or "")
        return m.group(1).strip() if m else None

    return {"image_tool_model": grab("image tool model \\(chosen by the backend\\)"), "image_size": grab("image size"), "image_quality": grab("image quality")}


def source_info(rec):
    """Where the art came from: the tool's own generation, or an import of art made on the same route."""
    if rec.get("import_source"):
        src = rec["import_source"]
        files = src["files"]
        return {
            "imported": True,
            "brief": " | ".join(src["prompts"]),
            "prompt": src["prompts"] if len(src["prompts"]) > 1 else src["prompts"][0],
            "prompt_sha256": [hashlib.sha256(t.encode("utf-8")).hexdigest() for t in src["prompts"]],
            "provider": "chatgpt_login (gptimage MCP, the same ChatGPT login)",
            "model": "ChatGPT subscription image model, quality high, as the gptimage MCP reports no more",
            "image_tool_model": None,
            "image_quality": "high (requested)",
            "source_image_size": ", ".join(f"{Image.open(os.path.join(MOD, f['file'])).width}x{Image.open(os.path.join(MOD, f['file'])).height}" for f in files),
            "date": files[0]["made"],
            "candidate_id": None,
            "source_pixels_sha256": [f["sha256"] for f in files],
        }
    cand = rec["generate"]["generation"]["candidates"][0]
    note = parse_note(cand.get("note"))
    return {
        "imported": False,
        "brief": rec["brief"],
        "prompt": rec["generate"]["plan"]["prompt"]["text"],
        "prompt_sha256": cand.get("prompt_sha256"),
        "provider": cand.get("provider"),
        "model": cand.get("model"),
        "image_tool_model": note["image_tool_model"],
        "image_quality": note["image_quality"],
        "source_image_size": note["image_size"],
        "date": rec["generated_at"],
        "candidate_id": cand["candidate_id"],
        "source_pixels_sha256": cand.get("pixels_sha256"),
    }


def export():
    rows = results()
    os.makedirs(OUT, exist_ok=True)
    order = [f for f in family_order() if f in rows]
    fams_group = {f["family"]: f["group"] for f in cli("families")["families"]}
    manifest = []
    tiles = []
    for fam in order:
        rec = rows[fam]
        tile, native, scale = display(fam, rec)
        path = os.path.join(OUT, fam + ".webp")
        if scale > 1:
            tile.save(path, "WEBP", lossless=True, method=6)
            enc = "lossless WebP"
        else:
            tile.save(path, "WEBP", quality=90, method=6)
            enc = "WebP quality 90"
        si = source_info(rec)
        install = rec["install"]
        manifest.append(
            {
                "family": fam,
                "group": fams_group[fam],
                "tool": rec["tool"],
                "image": f"docs/gallery/{fam}.webp",
                "sha256": sha256(path),
                "encoding": enc,
                "native_size": list(native),
                "enlarged": scale,
                "asset": rec["asset"],
                "made_by": "imported art, made on the same ChatGPT login" if si["imported"] else "hoi4-imagegen",
                "brief": si["brief"],
                "prompt": si["prompt"],
                "prompt_sha256": si["prompt_sha256"],
                "provider": si["provider"],
                "model": si["model"],
                "reasoning_effort": None if si["imported"] else "high",
                "image_tool_model": si["image_tool_model"],
                "image_quality": si["image_quality"],
                "source_image_size": si["source_image_size"],
                "date": si["date"],
                "candidate_id": si["candidate_id"],
                "source_pixels_sha256": si["source_pixels_sha256"],
                "processed_pixels_sha256": rec["process"].get("pixels_sha256"),
                "recipe": rec["process"].get("recipe"),
                "installed_files": [{"path": f["path"], "sha256": f["sha256"]} for f in install.get("final_files", [])],
            }
        )
        tiles.append((fam, tile))
    contact_sheet(tiles)
    meta = {
        "about": "Real output of hoi4-imagegen on the ChatGPT-login route (plan quota, no money). Images are the tool's processed files at native size on a dark tile, enlarged by whole numbers with nearest neighbour when small.",
        "game": "Hearts of Iron IV 1.19.4.1",
        "generated": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d"),
        "images": manifest,
    }
    with open(os.path.join(OUT, "manifest.json"), "w", encoding="utf-8", newline="\n") as f:
        json.dump(meta, f, indent=1, ensure_ascii=False)
        f.write("\n")
    print("exported", len(manifest), "families")


def font(size):
    for name in ("arial.ttf", "DejaVuSans.ttf", "Helvetica.ttc"):
        try:
            return ImageFont.truetype(name, size)
        except OSError:
            pass
    return ImageFont.load_default()


def contact_sheet(tiles, cols=8, box=(176, 150), label_h=22, pad=10):
    rows = math.ceil(len(tiles) / cols)
    cw, ch = box[0] + pad, box[1] + label_h + pad
    sheet = Image.new("RGB", (cols * cw + pad, rows * ch + pad), (24, 25, 30))
    draw = ImageDraw.Draw(sheet)
    f = font(13)
    for i, (fam, tile) in enumerate(tiles):
        t = tile.copy()
        t.thumbnail(box, Image.LANCZOS)
        x = pad + (i % cols) * cw
        y = pad + (i // cols) * ch
        sheet.paste(t, (x + (box[0] - t.width) // 2, y + (box[1] - t.height) // 2))
        draw.text((x + 2, y + box[1] + 3), fam, fill=(190, 200, 195), font=f)
    sheet.save(os.path.join(OUT, "contact-sheet.webp"), "WEBP", quality=86, method=6)


# ---------------------------------------------------------------- the documentation


def snake(camel):
    return re.sub(r"(?<!^)(?=[A-Z])", "_", camel).lower()


def unescape(s):
    return s.replace('\\"', '"').replace("\\\\", "\\")


def lookups():
    text = open(os.path.join(ROOT, "crates/imagegen-core/src/lookup.rs"), encoding="utf-8").read()
    pat = re.compile(r'^\s*([A-Za-z ]+(?:\|\s*[A-Za-z ]+)*) => l\((\w+), "((?:[^"\\]|\\.)*)", "((?:[^"\\]|\\.)*)", (None|Some\("(?:[^"\\]|\\.)*"\))\),', re.M)
    out = {}
    for m in pat.finditer(text):
        kind = snake(m.group(2))
        for name in m.group(1).split("|"):
            out[snake(name.strip())] = {"kind": kind, "script": unescape(m.group(3)), "evidence": unescape(m.group(4))}
    return out


def steps_text(steps):
    parts = []
    for s in steps or []:
        op = s["op"]
        args = {k: v for k, v in op.items() if k != "op"}
        a = ", ".join(f"{k} {v}" for k, v in args.items() if k not in ("min_alpha",))
        parts.append(f"`{op['op']}`" + (f" ({a})" if a else ""))
    return " → ".join(parts)


def canvas_text(fam):
    c = fam["canvas"]
    if not c or c[0] == 0:
        return "set by the mod (no vanilla texture to measure)"
    return f"{c[0]}x{c[1]}"


IMPORT_CALLS = {
    "counter_large": {"frames": ["incoming/cl_f1.png", "incoming/cl_f2.png"]},
    "counter_small": {"frames": ["incoming/cs_f1.png", "incoming/cs_f2.png"]},
    "division_emblem": {"path": "incoming/de.png"},
}


def docs():
    rows = results()
    order = [f for f in family_order() if f in rows]
    lk = lookups()
    manifest = {m["family"]: m for m in json.load(open(os.path.join(OUT, "manifest.json"), encoding="utf-8"))["images"]}
    fams = {f: cli("families", f)["family"] for f in order}
    out = []
    w = out.append
    w("# Family catalogue: the 40 still families and 4 animated sprites\n")
    w(
        "Every family below was made by the tool itself, on the [ChatGPT login](chatgpt-login.md) (your own ChatGPT plan, no API key, no money). "
        "Each image is a real output: planned, generated, processed, validated, reviewed and installed by `hoi4-imagegen`, then shown here at its native size on a dark tile. "
        "Small art is enlarged by a whole number so the pixels stay crisp. The images are not retouched. "
        "They are installed files, not in-game captures: the tool says \"installed\", never \"verified in game\". "
        "[`gallery/manifest.json`](gallery/manifest.json) lists the family, prompt, model, date and SHA-256 of every image.\n"
    )
    w('<p align="center"><a href="gallery/contact-sheet.webp"><img src="gallery/contact-sheet.webp" alt="One current example of each still family" width="100%"></a></p>\n')
    w("## How to read an entry\n")
    w(
        "* **Canvas and frames** are measured from the installed game, HOI4 1.19.4.1.\n"
        "* **Names the game looks up** follow the vanilla evidence in `crates/imagegen-core/src/lookup.rs`, as fixed in v0.2.2. Where a script names the sprite in full the name is free. Where the game builds the name, the tool builds the same one and the install result's `script` field says what to write.\n"
        "* **Processing** is the family's fixed chain. The tool appends it and you cannot skip it. The steps listed are the ones that ran on the example. The examples were made with version 0.2.0. Since 0.3.2 more families have their own chain: raid icons get vanilla's gold gradient, doctrine icons black units with a pale outline and glow, special project icons are redrawn as line art, counters and division emblems always end in the unit recipe, and a landmark's small strip is a flat orange pictogram. The [reference](reference.md#processing-chains) lists every chain.\n"
        "* **Checks** are the deterministic set that `hoi4.image_validate` always runs for the family. Composition and readability at game size go to the visual review.\n"
        "* **Example call** is the one-call form (`approve: true` plans and generates together) as it was run for the image. `hoi4-imagegen call <tool> '<json>'` runs the same operation on the command line. The rest of the loop (process, validate, preview, review, install) is in the [how-to](how-to.md).\n"
        "* The example mod's feature slug is `gallery`. Replace it and the asset name with your own.\n"
    )
    w("## Contents\n")
    for key, title, tool in GROUPS:
        names = [f for f in order if fams[f]["group"] == key]
        w(f"* [{title}](#{title.lower().replace(',', '').replace(' ', '-')}) (`{tool}`): " + ", ".join(f"[`{n}`](#{n})" for n in names))
    w("")
    for key, title, tool in GROUPS:
        names = [f for f in order if fams[f]["group"] == key]
        w(f"## {title}\n")
        w(f"Planned with `{tool}`.\n")
        for name in names:
            rec, fam, man = rows[name], fams[name], manifest[name]
            install = rec["install"]
            lookup = lk[name]
            w(f"### {name}\n")
            w(f'<img src="gallery/{name}.webp" alt="Example {name} asset made by hoi4-imagegen" width="{min(480, man["native_size"][0] * man["enlarged"])}">\n')
            w("| | |")
            w("|---|---|")
            w(f"| Group and tool | {fam['group']}, `{fam['tool']}` |")
            frames = fam["frames"]
            strip = fam["strip"]
            size = canvas_text(fam)
            if frames > 1:
                size += f" per frame, {frames} frames side by side in a {strip[0]}x{strip[1]} strip"
            else:
                size += ", 1 frame"
            w(f"| Canvas and frames | {size}; {fam['container'].upper()} |")
            if len(fam["accepted"]) > 1:
                w("| Also accepted | " + ", ".join(f"{a[0]}x{a[1]}" for a in fam["accepted"][1:]) + " |")
            names_used = (install.get("sprite") or {}).get("names") or []
            files = [f["path"] for f in install.get("final_files", [])]
            naming = KIND[lookup["kind"]] + "."
            if names_used:
                naming += " The example installs " + ", ".join(f"`{n}`" for n in names_used) + "."
            w(f"| Names the game looks up | {naming} |")
            w(f"| Script | {install.get('script') or lookup['script']} |")
            w("| Installed files (example) | " + "<br>".join(f"`{p}`" for p in files) + " |")
            recipe = rec["process"]["recipe"]
            chain = steps_text(rec["process"].get("steps"))
            fin = [f["name"] for f in rec["process"].get("finishers", []) if isinstance(f, dict) and f.get("name")]
            w(f"| Processing | recipe `{recipe['id']}` v{recipe['version']}: {RECIPES.get(recipe['id'], '')}. Ran: {chain or 'import conform'}" + (f", then finishing `{'`, `'.join(fin)}`" if fin else "") + " |")
            w("| Checks | " + ", ".join(f"`{c}`" for c in fam["checks"]) + " |")
            imported = man["made_by"] != "hoi4-imagegen"
            if imported:
                w(f"| Made | {man['date'][:10]}. The tool does not generate this family, so the source art was made on the same ChatGPT login with the `gptimage` MCP ({man['source_image_size']}), cut to the exact frames and imported with `hoi4.image_process` |")
            else:
                w(f"| Made | {man['date'][:10]} on `{man['model']}` (high reasoning effort), ChatGPT's current image model, quality {man['image_quality']}, source {man['source_image_size']} |")
            w(f"| Image SHA-256 | `{man['sha256']}` |")
            w("")
            if imported:
                spec = IMPORT_CALLS[name]
                req = {"import": {**spec, "family": name, "feature_slug": "gallery", "asset": rec["asset"]}}
                w("```sh")
                w(f"hoi4-imagegen call hoi4.image_process '{json.dumps(req, ensure_ascii=False)}'")
                w("```\n")
                w("Source prompts:\n")
                for t in man["prompt"] if isinstance(man["prompt"], list) else [man["prompt"]]:
                    w(f"> {t}\n")
            else:
                req = {"family": name, "feature_slug": "gallery", "asset": rec["asset"], "brief": rec["brief"], "approve": True, "idempotency_key": f"gallery-{name}-1", **rec.get("extra", {})}
                w("```sh")
                w(f"hoi4-imagegen call {fam['tool']} '{json.dumps(req, ensure_ascii=False)}'")
                w("```\n")
            w(f"<details><summary>Vanilla evidence for the name</summary>\n\n{lookup['evidence']}\n\n</details>\n")
    gen = [m for m in manifest.values() if m["made_by"] == "hoi4-imagegen"]
    low = sorted(m["family"] for m in gen if m["image_quality"] == "low")
    w("## Usage\n")
    w(
        "The gallery used ChatGPT plan quota and no money (decision W5-1). It was made on 2026-11-02 with a ChatGPT Plus plan.\n\n"
        f"* **{len(gen)} images generated by the tool**, one per family, plus **1 edit** (the ruined second frame of `building`): {len(gen) + 1} responses on `gpt-6-luna` at high reasoning effort, all checked by the tool.\n"
        "* **6 images made with the `gptimage` MCP** on the same login for the three import-only families (two frames for each counter, one for the division emblem, and one regenerated rifle that did not match its other frame).\n"
        "* **1 discarded test generation** (a focus icon made while checking the setup, replaced by the one shown).\n"
        "* In total 45 image responses. The backend does not report tokens or a price per image, so the plan's own usage meter is the only record.\n"
        f"* The backend chose the quality, as the [ChatGPT login notes](chatgpt-login.md#what-the-backend-controls) explain: {len(gen) - len(low)} of {len(gen)} came back at medium and {len(low)} at low ({', '.join('`' + f + '`' for f in low)}). Every one passed validation and the visual review at native size.\n"
    )
    w("## The checks\n")
    w("| Check | What it looks at |")
    w("|---|---|")
    for k, v in CHECKS.items():
        w(f"| `{k}` | {v} |")
    w("")
    path = os.path.join(ROOT, "docs", "families.md")
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(out) + "\n")
    print("wrote", path, len(order), "families")


if __name__ == "__main__":
    {"export": export, "docs": docs}[sys.argv[1] if len(sys.argv) > 1 else "docs"]()
