# Sizes the thumbnail snapshots for YouTube: thumbs/out/option_NN.png at 1280x720 (under 2 MB) and a labelled
# contact sheet, thumbs/out/options.png. Usage: python src/thumbs.py <snapshot folder>  (frame-NN-at-*.png, in order)
import os, sys
from PIL import Image, ImageDraw, ImageFont

here = os.path.dirname(os.path.abspath(__file__))
out = os.path.join(here, "..", "thumbs", "out")
os.makedirs(out, exist_ok=True)
frames = sorted(f for f in os.listdir(sys.argv[1]) if f.startswith("frame-") and f.endswith(".png"))
opts = []
for i, f in enumerate(frames, 1):
    im = Image.open(os.path.join(sys.argv[1], f)).convert("RGB").resize((1280, 720), Image.LANCZOS)
    p = os.path.join(out, f"option_{i:02d}.png")
    im.save(p, optimize=True)
    if os.path.getsize(p) >= 2 * 1024 * 1024:
        raise SystemExit(f"{p} is over 2 MB")
    opts.append(im)
try:
    font = ImageFont.truetype("arial.ttf", 30)
except OSError:
    font = ImageFont.load_default()
cols, w, h, pad = 3, 640, 360, 24
rows = (len(opts) + cols - 1) // cols
sheet = Image.new("RGB", (cols * (w + pad) + pad, rows * (h + pad + 44) + pad), (20, 24, 28))
d = ImageDraw.Draw(sheet)
for i, im in enumerate(opts):
    x, y = pad + (i % cols) * (w + pad), pad + (i // cols) * (h + pad + 44)
    d.text((x, y), f"option_{i + 1:02d}", fill=(232, 181, 79), font=font)
    sheet.paste(im.resize((w, h), Image.LANCZOS), (x, y + 40))
sheet.save(os.path.join(out, "options.png"), optimize=True)
print(f"{len(opts)} options, largest {max(os.path.getsize(os.path.join(out, f'option_{i:02d}.png')) for i in range(1, len(opts) + 1)) // 1024} KB")
