# Writes display copies cropped to their visible pixels plus a small margin, so a subject drawn small on a large
# transparent canvas fills its tile: <asset>-vN-fit.png beside each regenerated raw candidate in assets/regen/
# (every version of one asset shares one crop, so a refinement keeps its original's framing), and
# assets/gallery/fit/<asset>-v1.png for the gallery's first run. The raw and processed files stay as the tool wrote them.
import json, os, re
from PIL import Image

here = os.path.dirname(os.path.abspath(__file__))
regen = os.path.join(here, "..", "assets", "regen")
gallery = os.path.join(here, "..", "assets", "gallery")


def bbox(im):
    return im.getchannel("A").point(lambda a: 255 if a > 8 else 0).getbbox()


def crop(ims, dsts):
    boxes = [b for b in map(bbox, ims) if b]
    if not boxes:
        return
    box = (min(b[0] for b in boxes), min(b[1] for b in boxes), max(b[2] for b in boxes), max(b[3] for b in boxes))
    w, h = ims[0].size
    pad = round(max(box[2] - box[0], box[3] - box[1]) * 0.04)
    box = (max(0, box[0] - pad), max(0, box[1] - pad), min(w, box[2] + pad), min(h, box[3] + pad))
    for im, dst in zip(ims, dsts):
        im.crop(box).save(dst)


index = json.load(open(os.path.join(regen, "index.json"), encoding="utf-8")) if os.path.exists(os.path.join(regen, "index.json")) else {}


def first_frame(im, asset):
    # a building is a two-frame strip (intact, then ruined); its display copy shows the intact frame
    if index.get(asset, {}).get("family") == "building":
        return im.crop((0, 0, im.width // 2, im.height))
    return im


for asset in sorted(os.listdir(regen)):
    d = os.path.join(regen, asset)
    raws = sorted(f for f in os.listdir(d) if re.fullmatch(rf"{re.escape(asset)}-v\d+\.png", f)) if os.path.isdir(d) else []
    if raws:
        crop([first_frame(Image.open(os.path.join(d, f)).convert("RGBA"), asset) for f in raws], [os.path.join(d, f[:-4] + "-fit.png") for f in raws])
picked = os.path.join(here, "..", "assets", "picked")
if os.path.isdir(picked):
    for family in sorted(os.listdir(picked)):
        d = os.path.join(picked, family)
        if os.path.isdir(d):
            for f in sorted(os.listdir(d)):
                if f.endswith("-v1.png"):
                    crop([Image.open(os.path.join(d, f)).convert("RGBA")], [os.path.join(d, f[:-4] + "-fit.png")])
os.makedirs(os.path.join(gallery, "fit"), exist_ok=True)
for f in sorted(os.listdir(os.path.join(gallery, "source_png"))):
    if f.endswith("-v1.png"):
        crop([Image.open(os.path.join(gallery, "source_png", f)).convert("RGBA")], [os.path.join(gallery, "fit", f)])
print("fit copies written")
