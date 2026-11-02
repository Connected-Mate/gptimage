#!/usr/bin/env python3
"""Measure every vanilla frameAnimatedSpriteType from the installed game (read-only).

Usage: scripts/py.sh scripts/measure_animated.py [--json crates/imagegen-core/data/vanilla-measurement.json]

Reads the game's .gfx files and the strips they name (base, dlc/*, integrated_dlc/*), reusing the sprite index
of measure_families.py. Writes, into the measurement file in place:

* `animated_sprites`: every frameAnimatedSpriteType grouped by frame count (fps, effect file, looping,
  play_on_show, pause_on_loop and frame sizes), and
* one `families` entry per animated family this tool makes (`anim_*`), in the same shape as the other entries
  (strip sizes in `canvas_*`, registration counts, pixel class) plus an `animation` object with the frame size,
  fps and the per-frame motion statistics the derivations were fitted to.

The game is never modified and never started.
"""
import argparse
import collections
import json
import os
import statistics
import sys

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import measure_families as M  # noqa: E402

BLEND = 'gfx/fx/buttonstate_blendframes.lua'


def fnum(v):
    try:
        return float(v)
    except (TypeError, ValueError):
        return None


def rnd(x, n=4):
    return None if x is None else round(float(x), n)


def med(xs):
    xs = [x for x in xs if x is not None]
    return rnd(statistics.median(xs)) if xs else None


def load_frames(path, n):
    im = Image.open(path).convert('RGBA')
    a = np.asarray(im).astype(np.float64)
    h, w = a.shape[:2]
    fw = w // n
    return [a[:, i * fw:(i + 1) * fw] for i in range(n)], (w, h)


def centroid(f):
    al = f[..., 3]
    s = al.sum()
    if s <= 0:
        return None
    ys, xs = np.mgrid[0:al.shape[0], 0:al.shape[1]]
    return float((xs * al).sum() / s), float((ys * al).sum() / s)


def iou(a, b, t=16):
    ma, mb = a[..., 3] >= t, b[..., 3] >= t
    u = (ma | mb).sum()
    return float((ma & mb).sum() / u) if u else 1.0


def mean_diff(a, b):
    """Mean absolute difference of premultiplied RGBA, 0..255."""
    pa = np.concatenate([a[..., :3] * a[..., 3:4] / 255.0, a[..., 3:4]], -1)
    pb = np.concatenate([b[..., :3] * b[..., 3:4] / 255.0, b[..., 3:4]], -1)
    return float(np.abs(pa - pb).mean())


def frame_pixel_stats(frames):
    """Statistics of one strip's frames: what changes between them."""
    a0 = [f[..., 3].sum() for f in frames]
    rgb_mean = []
    for f in frames:
        w = f[..., 3]
        s = w.sum()
        rgb_mean.append([float((f[..., c] * w).sum() / s) if s else 0.0 for c in range(3)])
    cents = [centroid(f) for f in frames]
    c0 = cents[0]
    drift = [((c[0] - c0[0]) ** 2 + (c[1] - c0[1]) ** 2) ** 0.5 for c in cents if c and c0]
    steps = [mean_diff(frames[i], frames[(i + 1) % len(frames)]) for i in range(len(frames))]
    ious = [iou(frames[0], f) for f in frames[1:]]
    alpha_ratio = [x / a0[0] for x in a0] if a0[0] else None
    al = np.stack([f[..., 3] for f in frames])
    m0 = rgb_mean[0]
    pal = [sum((m[c] - m0[c]) ** 2 for c in range(3)) ** 0.5 for m in rgb_mean[1:]]
    lo, hi = min(a0), max(a0)
    return dict(
        palette_drift_max=rnd(max(pal) if pal else 0.0, 2),
        dim_to_bright_alpha_ratio=rnd(lo / hi) if hi else None,
        alpha_sum_ratio_to_frame1=[rnd(x) for x in alpha_ratio] if alpha_ratio else None,
        mean_rgb=[[rnd(c, 1) for c in m] for m in rgb_mean],
        centroid_drift_px_max=rnd(max(drift) if drift else 0.0, 2),
        silhouette_iou_min=rnd(min(ious) if ious else 1.0),
        step_diff=[rnd(s, 3) for s in steps],
        loop_seam_ratio=rnd(steps[-1] / statistics.median(steps[:-1])) if len(steps) > 2 and statistics.median(steps[:-1]) > 0 else None,
        transparent_fraction=rnd(float((al == 0).mean())),
        opaque_fraction=rnd(float((al == 255).mean())),
    )


def classify(s, attrs):
    n = int(attrs.get('noofframes', 1))
    eff = M.norm(attrs.get('effectfile', ''))
    if eff == BLEND and n == 2:
        return 'blend_glow'
    if eff == BLEND:
        return 'blend_keyframes'
    return 'frame_strip'


def scroll_shift(frames):
    """Horizontal shift in px per frame that best maps frame i onto frame i+1 (wrapping), median over steps."""
    out = []
    for i in range(len(frames) - 1):
        a, b = frames[i], frames[i + 1]
        w = a.shape[1]
        best = None
        for d in range(-w // 2, w // 2 + 1):
            e = float(np.abs(np.roll(a, d, axis=1) - b).mean())
            if best is None or e < best[0]:
                best = (e, d)
        out.append(best[1])
    return out


# The animated families this tool makes, each tied to the vanilla sprites it is measured from.
FAMILIES = [
    dict(id='anim_blend_glow', label='Two-frame blend glow (buttonstate_blendframes.lua): the engine cross-fades a dim and a bright frame',
         select=lambda s, a: classify(s, a) == 'blend_glow'),
    dict(id='anim_blend_keyframes', label='Three-frame blend animation (buttonstate_blendframes.lua): the engine cross-fades three key frames',
         select=lambda s, a: classify(s, a) == 'blend_keyframes' and int(a['noofframes']) == 3),
    dict(id='anim_scroll', label='Nine-frame looping strip at 15 fps: the currently-researching highlight whose pattern scrolls one period per loop',
         select=lambda s, a: int(a['noofframes']) == 9 and not a.get('effectfile')),
    dict(id='anim_spin', label='Twenty-frame looping icon at 7 fps: the turning cog icons of the leader groups and navy views',
         select=lambda s, a: int(a['noofframes']) == 20 and fnum(a.get('animation_rate_fps')) == 7.0),
]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--json', default=os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'crates', 'imagegen-core', 'data', 'vanilla-measurement.json'))
    args = ap.parse_args()
    sprites = [s for s in M.index_sprites() if s['type'].lower() == 'frameanimatedspritetype']
    rows = []
    for s in sprites:
        a = s['attrs']
        n = int(a.get('noofframes', 1))
        p = M.resolve_tex(s['tex'], s['root'])
        h = M.header(p) if p else None
        rows.append(dict(s=s, a=a, n=n, path=p, h=h,
                         strip=(h['w'], h['h']) if h else None,
                         frame=(h['w'] // n, h['h']) if h else None))
    # whole set, by frame count
    by_n = collections.defaultdict(list)
    for r in rows:
        by_n[r['n']].append(r)
    summary = {}
    for n in sorted(by_n):
        rs = by_n[n]
        c = lambda key: dict(collections.Counter(str(r['a'].get(key)) for r in rs).most_common())
        summary[str(n)] = dict(
            sprites=len(rs),
            fps=c('animation_rate_fps'),
            effect_file=dict(collections.Counter(M.norm(r['a'].get('effectfile', '')) or '(none)' for r in rs).most_common()),
            looping=c('looping'),
            play_on_show=c('play_on_show'),
            pause_on_loop=c('pause_on_loop'),
            frame_sizes_top=dict(('%dx%d' % f, k) for f, k in collections.Counter(r['frame'] for r in rs if r['frame']).most_common(8)),
            texture_missing=sum(1 for r in rs if not r['h']),
            strips_not_dividing=sum(1 for r in rs if r['h'] and r['h']['w'] % n),
        )
    doc = json.load(open(args.json, encoding='utf-8'))
    doc['animated_sprites'] = dict(
        type='frameAnimatedSpriteType',
        sprite_count=len(rows),
        attribute_counts=dict(collections.Counter(k for r in rows for k in r['a']).most_common()),
        by_frame_count=summary,
        notes='Counted from every .gfx under base, dlc/* and integrated_dlc/* (each root\'s own sprites). The 21-frame decision category strips are spriteType entries with noOfFrames = 21, not frameAnimatedSpriteType, so they are not here.',
    )
    fams = [f for f in doc['families'] if not f['id'].startswith('anim_')]
    for fam in FAMILIES:
        rs = [r for r in rows if fam['select'](r['s'], r['a'])]
        textures = {}
        for r in rs:
            if r['path']:
                textures.setdefault(os.path.normcase(r['path']), r)
        strips = collections.Counter('%dx%d' % r['strip'] for r in textures.values())
        frames_c = collections.Counter('%dx%d' % r['frame'] for r in textures.values())
        # ties between texture counts go to the size more sprites use (a researching strip is shared by 16 sprites)
        by_sprites = collections.Counter('%dx%d' % r['frame'] for r in rs if r['frame'])
        strip_sprites = collections.Counter('%dx%d' % r['strip'] for r in rs if r['strip'])
        dom = max(strips, key=lambda k: (strips[k], strip_sprites[k], k))
        dom_n = strips[dom]
        fdom = max(frames_c, key=lambda k: (frames_c[k], by_sprites[k], k))
        fdom_n = frames_c[fdom]
        stats = []
        fails = []
        for r in sorted(textures.values(), key=lambda r: r['path']):
            try:
                frames, _ = load_frames(r['path'], r['n'])
                st = frame_pixel_stats(frames)
                if fam['id'] == 'anim_scroll':
                    st['scroll_px_per_frame'] = scroll_shift(frames)
                st['texture'] = M.norm(os.path.relpath(r['path'], M.GAME))
                stats.append(st)
            except Exception as ex:  # a texture PIL cannot decode is recorded, not hidden
                fails.append([r['path'], str(ex)[:80]])
        cnt = lambda key: dict(collections.Counter(str(r['a'].get(key)) for r in rs).most_common())
        anim = dict(
            frames=sorted({r['n'] for r in rs}),
            frame_dominant=fdom,
            frame_dominant_count=fdom_n,
            frame_distribution_top=dict(frames_c.most_common(10)),
            frame_distribution_by_sprites=dict(by_sprites.most_common(10)),
            textures_measured=len(stats),
            median_alpha_sum_ratio_to_frame1=[med([s['alpha_sum_ratio_to_frame1'][i] for s in stats if s['alpha_sum_ratio_to_frame1']]) for i in range(max(r['n'] for r in rs))],
            median_centroid_drift_px_max=med([s['centroid_drift_px_max'] for s in stats]),
            p90_centroid_drift_px_max=rnd(float(np.percentile([s['centroid_drift_px_max'] for s in stats], 90)), 2) if stats else None,
            median_silhouette_iou_min=med([s['silhouette_iou_min'] for s in stats]),
            p10_silhouette_iou_min=rnd(float(np.percentile([s['silhouette_iou_min'] for s in stats], 10))) if stats else None,
            median_palette_drift_max=med([s['palette_drift_max'] for s in stats]),
            p90_palette_drift_max=rnd(float(np.percentile([s['palette_drift_max'] for s in stats], 90)), 2) if stats else None,
            median_dim_to_bright_alpha_ratio=med([s['dim_to_bright_alpha_ratio'] for s in stats if s['dim_to_bright_alpha_ratio'] is not None and 0 < s['dim_to_bright_alpha_ratio'] < 0.95]),
            dim_frames_fully_clear=sum(1 for s in stats if s['dim_to_bright_alpha_ratio'] == 0),
            median_loop_seam_ratio=med([s['loop_seam_ratio'] for s in stats]),
            max_loop_seam_ratio=rnd(max([s['loop_seam_ratio'] for s in stats if s['loop_seam_ratio'] is not None], default=0) or None),
            per_texture=stats,
            decode_failures=fails,
        )
        all_alpha = [s['transparent_fraction'] for s in stats]
        all_opq = [s['opaque_fraction'] for s in stats]
        fams.append(dict(
            id=fam['id'],
            label=fam['label'],
            kind='gfx_animated',
            texture_folders=sorted({os.path.dirname(M.norm(os.path.relpath(r['path'], M.GAME))) for r in textures.values()}),
            measured_texture_count=len(textures),
            canvas_dominant=dom,
            canvas_dominant_count=dom_n,
            canvas_distribution_top=dict(strips.most_common(10)),
            canvas_distinct=len(strips),
            registration=dict(
                sprite_count=len(rs),
                gfx_files=[[k, v] for k, v in collections.Counter('%s:%s' % (r['s']['root'], r['s']['file']) for r in rs).most_common()],
                sprite_block_types={'frameAnimatedSpriteType': len(rs)},
                noOfFrames_values=dict(collections.Counter(str(r['n']) for r in rs).most_common()),
                animation_rate_fps_values=cnt('animation_rate_fps'),
                effect_file_values=dict(collections.Counter(M.norm(r['a'].get('effectfile', '')) or '(none)' for r in rs).most_common()),
                looping_values=cnt('looping'),
                play_on_show_values=cnt('play_on_show'),
                pause_on_loop_values=cnt('pause_on_loop'),
                alwaystransparent_values=cnt('alwaystransparent'),
                example_sprites=sorted(r['s']['name'] for r in rs)[:8],
            ),
            pixels=dict(
                sampled=len(stats),
                median_fully_transparent_fraction=med(all_alpha),
                median_opaque_fraction=med(all_opq),
                alpha_class='opaque canvas (no transparent px)' if (med(all_alpha) or 0) == 0 and (med(all_opq) or 0) > 0.9 else 'transparent background',
            ),
            animation=anim,
            notes=[],
        ))
        print('%-22s sprites=%-3d textures=%-3d strip=%s frame=%s' % (fam['id'], len(rs), len(textures), dom, fdom), file=sys.stderr)
    doc['families'] = fams
    with open(args.json, 'w', encoding='utf-8', newline='\n') as f:
        json.dump(doc, f, indent=1)
    print('wrote', args.json, file=sys.stderr)


if __name__ == '__main__':
    main()
