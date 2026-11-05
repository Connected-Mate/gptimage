#!/usr/bin/env python3
"""Swap the README examples and the per-family examples in docs/families.md for current gallery images.

  update_examples.py <repo root>

Each family's docs/gallery/<family>.webp becomes one image from docs/gallery/big/manifest.json (built by
build_gallery_site.py: 0.3 or later, validation and the vanilla-match check passed), drawn on a dark tile and enlarged
by a whole number. A family with no such image yet loses its old example and says when one comes. The contact sheet
is redrawn from the new examples. Environment as for make_gallery_big.py.
"""
import json
import os
import re
import sys

from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import make_gallery_big as m  # noqa: E402

TILE = (38, 39, 46)
# hand picks for the README's six examples (varied focus frames, 0.3.3 technology side view)
PICKS = {"focus": "embassy_flags_handshake_v033", "technology": "sloped_armour_medium_tank"}
HELD_NOTE = "An example follows when hoi4-imagegen 0.3.4 lands; the 0.2.x example was removed."


def scale_for(w, h):
    big = max(w, h)
    return 4 if big <= 40 else 3 if big <= 70 else 2 if big <= 160 else 1


def choose(fam, items):
    if fam in PICKS:
        hit = [e for e in items if e["asset"] == PICKS[fam]]
        if hit:
            return hit[0]
    def rank(e):
        rec = m.load(fam, e["asset"]) or {}
        good = (rec.get("my_verdict") or {}).get("verdict") == "good"
        return (not good, not rec.get("anchors"), e["build"] != max(x["build"] for x in items), e["asset"])
    return sorted(items, key=rank)[0]


def main(repo):
    big = os.path.join(repo, "docs", "gallery", "big")
    manifest = json.load(open(os.path.join(big, "manifest.json"), encoding="utf-8"))["images"]
    by_fam = {}
    for e in manifest:
        if e["section"] == "vanilla":
            by_fam.setdefault(e["family"], []).append(e)
    gallery = os.path.join(repo, "docs", "gallery")
    fams_md = os.path.join(repo, "docs", "families.md")
    text = open(fams_md, encoding="utf-8").read()
    examples = {}
    for f in sorted(os.listdir(gallery)):
        if not f.endswith(".webp") or f == "contact-sheet.webp":
            continue
        fam = f[:-5]
        items = by_fam.get(fam)
        tag = re.compile(r'<img src="gallery/' + re.escape(fam) + r'\.webp"[^>]*>')
        if not items:
            os.remove(os.path.join(gallery, f))
            text = tag.sub(f"*{HELD_NOTE}*", text)
            print(fam, "removed (no current example)")
            continue
        e = choose(fam, items)
        src = os.path.join(big, e["native"])
        img = Image.open(src).convert("RGBA")
        k = scale_for(*img.size)
        img = img.resize((img.width * k, img.height * k), Image.NEAREST)
        tile = Image.new("RGBA", img.size, TILE + (255,))
        tile.alpha_composite(img)
        tile.convert("RGB").save(os.path.join(gallery, f), "WEBP", quality=90)
        examples[fam] = (tile, e)
        text = tag.sub(f'<img src="gallery/{fam}.webp" alt="Example {fam} asset made by hoi4-imagegen" width="{tile.width}">', text)
        print(fam, "->", e["asset"], e["build"], f"{k}x")
    open(fams_md, "w", encoding="utf-8", newline="\n").write(text)
    # contact sheet of every current example
    tiles = [t for t, _ in examples.values()]
    cols, cell = 8, 220
    rows = (len(tiles) + cols - 1) // cols
    sheet = Image.new("RGB", (cols * cell, rows * cell), TILE)
    for n, t in enumerate(tiles):
        t = t.copy()
        t.thumbnail((cell - 12, cell - 12), Image.NEAREST)
        x, y = (n % cols) * cell + (cell - t.width) // 2, (n // cols) * cell + (cell - t.height) // 2
        sheet.paste(t, (x, y))
    sheet.save(os.path.join(gallery, "contact-sheet.webp"), "WEBP", quality=88)
    json.dump({fam: {"asset": e["asset"], "build": e["build"], "brief": e["brief"]} for fam, (_, e) in examples.items()},
              open(os.path.join(gallery, "examples.json"), "w", encoding="utf-8", newline="\n"), indent=1, ensure_ascii=False)
    print(len(examples), "examples,", len(tiles), "on the contact sheet")


if __name__ == "__main__":
    main(sys.argv[1])
