#!/usr/bin/env python3
"""Measure every HOI4 icon / picture family from the installed game (read-only).

Usage: scripts/py.sh measure.py [--out DIR] [--sample N]
Writes families.json into DIR (default: folder of this script).
Reads: the game install (base, dlc/*, integrated_dlc/*) and the kit reference shelf.
"""
import time, datetime
import os, re, sys, json, struct, collections, statistics, argparse
import numpy as np
from PIL import Image

GAME = r"C:\Program Files (x86)\Steam\steamapps\common\Hearts of Iron IV"
SHELF = r"C:\Users\klimp\Documents\Projects\agentic-hoi4-modding\.claude\skills\hoi4-feature-assets\assets\vanilla_reference"
BS = chr(92)

NOV1_TS = datetime.datetime(2026, 11, 1, 0, 0).timestamp()
UPDATE_TS = datetime.datetime(2026, 11, 1, 19, 46).timestamp()

# ----------------------------------------------------------------------------
# roots and gfx index
# ----------------------------------------------------------------------------
def roots():
    r = [('base', GAME)]
    for sub, pre in (('dlc', ''), ('integrated_dlc', 'int_')):
        d = os.path.join(GAME, sub)
        for n in sorted(os.listdir(d)):
            if os.path.isdir(os.path.join(d, n)):
                r.append((pre + n[:6], os.path.join(d, n)))
    return r

ROOTS = roots()
RP = dict(ROOTS)

def strip_comments(t):
    out = []
    for line in t.split('\n'):
        res = []
        q = False
        for ch in line:
            if ch == '"':
                q = not q
            if ch == '#' and not q:
                break
            res.append(ch)
        out.append(''.join(res))
    return '\n'.join(out)

def blocks(text):
    t = strip_comments(text)
    pos = 0
    for m in re.finditer(r'(\w+)\s*=\s*\{', t):
        if m.start() < pos:
            continue
        typ = m.group(1)
        if typ.lower() == 'spritetypes':
            continue
        depth = 1
        i = m.end()
        while i < len(t) and depth:
            c = t[i]
            if c == '{':
                depth += 1
            elif c == '}':
                depth -= 1
            i += 1
        pos = i
        yield typ, t[m.end():i - 1], t[m.start():i]

def top_attrs(body):
    d = {}
    s = ''
    depth = 0
    for c in body:
        if c == '{':
            depth += 1
        elif c == '}':
            depth -= 1
        elif depth == 0:
            s += c
    for m in re.finditer(r'(\w+)\s*=\s*("[^"]*"|[^\s{}]+)', s):
        d.setdefault(m.group(1).lower(), m.group(2).strip('"'))
    return d

def index_sprites():
    sprites = []
    for rn, rp in ROOTS:
        for dp, dn, fn in os.walk(rp):
            if rn == 'base' and dp == rp:
                dn[:] = [x for x in dn if x not in ('dlc', 'integrated_dlc')]
            for f in fn:
                if f.endswith('.gfx'):
                    p = os.path.join(dp, f)
                    try:
                        txt = open(p, encoding='utf-8-sig', errors='replace').read()
                    except Exception:
                        continue
                    for typ, body, raw in blocks(txt):
                        a = top_attrs(body)
                        if 'name' in a:
                            sprites.append(dict(root=rn, file=os.path.relpath(p, rp).replace(BS, '/'), type=typ,
                                                name=a['name'], tex=a.get('texturefile', ''), frames=a.get('noofframes'),
                                                attrs=a, raw=raw))
    return sprites

def norm(t):
    t = t.replace(BS, '/').lower()
    t = re.sub(r'/+', '/', t).lstrip('/')
    return t

def resolve_tex(rel, prefer=None):
    rel = norm(rel)
    if not rel:
        return None
    order = ([prefer] if prefer else []) + [r for r, _ in ROOTS]
    for rn in order:
        p = os.path.join(RP[rn], rel.replace('/', os.sep))
        if os.path.exists(p):
            return p
    return None

# ----------------------------------------------------------------------------
# headers and pixels
# ----------------------------------------------------------------------------
DXGI = {28: 'R8G8B8A8_UNORM', 29: 'R8G8B8A8_UNORM_SRGB', 87: 'B8G8R8A8_UNORM', 91: 'B8G8R8A8_UNORM_SRGB',
        71: 'BC1_UNORM', 74: 'BC2_UNORM', 77: 'BC3_UNORM', 98: 'BC7_UNORM'}

def friendly_fmt(fmt):
    if fmt.startswith('RGB32A:ff0000/ff00/ff/ff000000'):
        return 'DDS A8R8G8B8 (uncompressed BGRA32)'
    if fmt.startswith('RGB24:'):
        return 'DDS R8G8B8 (uncompressed 24bpp, no alpha)'
    if fmt.startswith('DX10:'):
        n = int(fmt[5:])
        return 'DDS DX10 DXGI %d %s' % (n, DXGI.get(n, '?'))
    if fmt in ('DXT1', 'DXT3', 'DXT5'):
        return 'DDS ' + fmt
    if fmt.startswith('TGA') or fmt.startswith('PNG'):
        return fmt
    return 'DDS ' + fmt

def header(path):
    e = os.path.splitext(path)[1].lower()
    try:
        with open(path, 'rb') as f:
            h = f.read(148)
        if e == '.dds' and h[:4] == b'DDS ':
            (size, flags, height, width, pitch, depth, mips) = struct.unpack('<7I', h[4:32])
            pfflags, fourcc = struct.unpack('<I4s', h[80:88])
            rgbbits, rm, gm, bm, am = struct.unpack('<5I', h[88:108])
            if pfflags & 4:
                fc = fourcc.decode('latin1')
                fmt = 'DX10:%d' % struct.unpack('<I', h[128:132])[0] if fc == 'DX10' else fc
            else:
                fmt = 'RGB%d%s:%x/%x/%x/%x' % (rgbbits, 'A' if pfflags & 1 else '', rm, gm, bm, am)
            return dict(w=width, h=height, mips=max(mips, 1), fmt=friendly_fmt(fmt))
        if e == '.tga':
            idlen, cmtype, imtype = h[0], h[1], h[2]
            w, hh = struct.unpack('<HH', h[12:16])
            bpp, desc = h[16], h[17]
            fmt = 'TGA type%d %dbpp alphabits=%d origin=%s' % (imtype, bpp, desc & 15, 'top-left' if desc & 32 else 'bottom-left')
            return dict(w=w, h=hh, mips=1, fmt=fmt)
        if e == '.png' and h[:8] == b'\x89PNG\r\n\x1a\n':
            w, hh, bd, ct = struct.unpack('>IIBB', h[16:26])
            return dict(w=w, h=hh, mips=1, fmt='PNG colortype%d bitdepth%d' % (ct, bd))
    except Exception:
        return None
    return None

def pixel_stats(path):
    try:
        im = Image.open(path).convert('RGBA')
    except Exception as ex:
        return None, str(ex)[:80]
    a = np.asarray(im).astype(np.int32)
    al = a[..., 3]
    H, W = al.shape
    tot = H * W
    ys, xs = np.where(al > 16)
    if len(xs):
        bbox = (int(xs.min()), int(ys.min()), int(W - 1 - xs.max()), int(H - 1 - ys.max()))
    else:
        bbox = (W, H, W, H)
    opaque = al > 200
    rgb = a[..., :3]
    mx = rgb.max(-1)
    mn = rgb.min(-1)
    chroma = float((mx - mn)[opaque].mean()) if opaque.any() else 0.0
    rb = float((rgb[..., 0] - rgb[..., 2])[opaque].mean()) if opaque.any() else 0.0
    border = np.concatenate([al[0], al[-1], al[:, 0], al[:, -1]])
    corners = [al[0, 0], al[0, -1], al[-1, 0], al[-1, -1]]
    return dict(w=W, h=H, transparent=float((al == 0).sum() / tot), semi=float(((al > 0) & (al < 255)).sum() / tot),
                opaque=float((al == 255).sum() / tot), any_alpha=bool(al.min() < 255), margins=bbox,
                border_alpha=float(border.mean()), corner_alpha=float(np.mean(corners)), chroma=chroma, r_minus_b=rb,
                mean_luma=float((0.299 * rgb[..., 0] + 0.587 * rgb[..., 1] + 0.114 * rgb[..., 2]).mean())), None

def med(xs):
    xs = [x for x in xs if x is not None]
    return round(statistics.median(xs), 4) if xs else None

def sample_even(items, n):
    items = sorted(items)
    if len(items) <= n:
        return items
    step = len(items) / n
    return [items[int(i * step)] for i in range(n)]

# ----------------------------------------------------------------------------
# script scanning
# ----------------------------------------------------------------------------
def read_txt_files(reldir):
    out = []
    for rn, rp in ROOTS:
        base = os.path.join(rp, reldir.replace('/', os.sep))
        if os.path.isfile(base):
            out.append(base)
        elif os.path.isdir(base):
            for dp, dn, fn in os.walk(base):
                for f in fn:
                    if f.lower().endswith(('.txt', '.md', '.gfx', '.gui')):
                        out.append(os.path.join(dp, f))
    return out

def script_scan(spec, names_lc):
    pat = re.compile(spec['pat'], re.M)
    vals = collections.Counter()
    where = {}
    for p in read_txt_files(spec['dir']):
        try:
            t = strip_comments(open(p, encoding='utf-8-sig', errors='replace').read())
        except Exception:
            continue
        for m in pat.finditer(t):
            v = m.group(1)
            vals[v] += 1
            where.setdefault(v, os.path.relpath(p, GAME).replace(BS, '/') if p.startswith(GAME) else p)
    explicit = sum(c for v, c in vals.items() if v.startswith('GFX_'))
    total = sum(vals.values())
    prefixes = spec.get('prefixes', [''])
    res = collections.Counter()
    unresolved = []
    for v in vals:
        hit = None
        for pf in ([''] if v.startswith('GFX_') else prefixes):
            if (pf + v).lower() in names_lc:
                hit = pf + v
                break
        res[('explicit_' if v.startswith('GFX_') else 'bare_') + ('resolved' if hit else 'unresolved')] += 1
        if not hit and len(unresolved) < 4:
            unresolved.append(v)
    ex = []
    for v, c in vals.most_common(3):
        ex.append(dict(value=v, uses=c, file=where[v]))
    return dict(dir=spec['dir'], regex=spec['pat'], note=spec.get('note', ''), occurrences=total, distinct=len(vals),
                explicit_GFX_form_occurrences=explicit, bare_form_occurrences=total - explicit,
                prefix_candidates_tried=prefixes, distinct_resolution=dict(res), unresolved_examples=unresolved, top=ex)

# ----------------------------------------------------------------------------
# family definitions
# ----------------------------------------------------------------------------
def F(id, label, sprite=None, dirs=None, measure_files=None, shelf=None, script=None, notes=None, frames_note=None, kind='gfx'):
    return dict(id=id, label=label, sprite=sprite or {}, dirs=dirs or [], measure_files=measure_files, shelf=shelf or [],
                script=script or [], notes=notes or [], kind=kind)

ICON_PAT = r'^\s*icon\s*=\s*"?([A-Za-z0-9_\-\.]+)"?'
PIC_PAT = r'^\s*picture\s*=\s*"?([A-Za-z0-9_\-\.]+)"?'

FAMILIES = [
    F('focus_icon', 'National focus icon (focus tree goal)',
      sprite=dict(file=r'(^|/)(tfv_)?goals\.gfx$|nationalfocusview\.gfx', name=r'^GFX_(goal|focus)_', xname=r'_shine$', tex=r'interface/goals/'),
      dirs=['gfx/interface/goals::^(goal|focus|untitl)'], shelf=['icons/national_focus'],
      script=[dict(dir='common/national_focus', pat=ICON_PAT, prefixes=[''], note='focus `icon = GFX_x` (explicit GFX_ name, never implicitly prefixed); a focus may use `icon = { trigger = {...} icon = GFX_a }` blocks'),
              dict(dir='common/national_focus', pat=r'^\s*icon\s*=\s*(\{)', prefixes=[''], note='count of conditional icon blocks (value is the brace, not resolved)')]),
    F('focus_shine', 'National focus shine overlay (animation texture) and shine sprites',
      sprite=dict(file=r'goals_shine\.gfx$', name=r'_shine$', tex=r'interface/goals/'),
      measure_files=['gfx/interface/goals/shine_overlay.dds'], shelf=[],
      notes=['one GFX_<focus sprite>_shine per focus sprite; texturefile = the focus icon itself, animation.animationmaskfile = the same icon, animation.animationtexturefile = goals/shine_overlay.dds']),
    F('idea_spirit_and_advisor', 'Ideas / national spirits / advisors (all GFX_idea_* sprites: see idea_categories for the split)',
      sprite=dict(file=r'ideas\.gfx$|dod_ideas|tfv_ideas|lar_ideas\.gfx|_dynamic_modifiers\.gfx', name=r'^GFX_idea_', tex=r'interface/ideas/'),
      dirs=['gfx/interface/ideas::^idea_'], shelf=['icons/ideas'],
      script=[dict(dir='common/ideas', pat=PIC_PAT, prefixes=['GFX_idea_'], note='idea `picture = x` -> GFX_idea_x (an explicit GFX_ name is also accepted)')]),
    F('advisor_character_small', 'Advisor / character small portrait (65x67 GFX_idea_<token> and GFX_portrait_*_small)',
      sprite=dict(file=r'ideas_characters\.gfx$|_leader_portraits\.gfx$|_random_portraits\.gfx$', name=r'^GFX_idea_', tex=r'interface/ideas/'),
      dirs=['gfx/interface/ideas::^portrait'], shelf=['portraits/advisors'],
      script=[dict(dir='common/characters', pat=r'^\s*small\s*=\s*"?([A-Za-z0-9_\-\.]+)"?', prefixes=[''], note='character `portraits = { civilian|army|navy = { large = GFX_... small = GFX_... } }` (small = 65x67 dossier)'),
              dict(dir='common/characters', pat=r'idea_token\s*=\s*"?([A-Za-z0-9_\-\.]+)"?', prefixes=['GFX_idea_'], note='advisor role `idea_token = X` -> sprite GFX_idea_X (sprites are in *ideas_characters.gfx)')]),
    F('decision_icon', 'Decision / mission icon',
      sprite=dict(file=r'interface/decisions\.gfx$', name=r'^GFX_decision_', xname=r'^GFX_decision_(cat_|category_|item|ui)', tex=r'interface/decisions/'),
      dirs=['gfx/interface/decisions::^decision_(?!cat)'], shelf=['icons/decisions', 'icons/missions'],
      script=[dict(dir='common/decisions', pat=ICON_PAT, prefixes=['GFX_decision_', 'GFX_'], note='decision/mission `icon = x` (bare -> GFX_decision_x) or explicit `icon = GFX_decision_x`; `icon = { ... }` conditional blocks also exist'),
              dict(dir='common/decisions', pat=r'^\s*(days_mission_timeout)\s*=', prefixes=[''], note='missions are decisions with days_mission_timeout; they use the same icon key and sprites')]),
    F('decision_category_icon', 'Decision category icon (small)',
      sprite=dict(file=r'interface/decisions\.gfx$', name=r'^GFX_decision_category_', tex=r'interface/decisions/'),
      dirs=['gfx/interface/decisions::^decision_category'], shelf=['icons/decision_categories'],
      script=[dict(dir='common/decisions/categories', pat=ICON_PAT, prefixes=['GFX_decision_category_', 'GFX_decision_', 'GFX_'], note='category `icon = x` -> GFX_decision_category_x or explicit GFX_ name')]),
    F('decision_category_picture', 'Decision category picture (large header art)',
      sprite=dict(file=r'interface/decisions\.gfx$', name=r'^GFX_decision_cat_(?!egory)', tex=r'interface/decisions/'),
      dirs=['gfx/interface/decisions::^decision_cat_'], shelf=['icons/decision_categories/pictures'],
      script=[dict(dir='common/decisions/categories', pat=PIC_PAT, prefixes=['GFX_decision_cat_', 'GFX_'], note='category `picture = GFX_decision_cat_*` (always explicit)')]),
    F('dynamic_and_state_modifier_icon', 'Dynamic modifier / state modifier icon (GFX_modifiers_*)',
      sprite=dict(file=r'countrystateview\.gfx$', name=r'^GFX_modifiers_', tex=r'interface/state_modifiers/'),
      dirs=['gfx/interface/state_modifiers'], shelf=['icons/state_modifiers'],
      script=[dict(dir='common/dynamic_modifiers', pat=ICON_PAT, prefixes=['GFX_modifiers_', 'GFX_'], note='dynamic_modifier `icon = GFX_modifiers_*`; the same key is used for state-scope dynamic modifiers'),
              dict(dir='common/modifiers', pat=ICON_PAT, prefixes=['GFX_modifiers_', 'GFX_'], note='static modifier definitions (common/modifiers) `icon`'),
              dict(dir='common/triggered_modifiers.txt', pat=ICON_PAT, prefixes=['GFX_modifiers_', 'GFX_'], note='triggered modifiers `icon`')]),
    F('stat_modifier_tooltip_icon', 'Modifier-stat tooltip icons (MODIFIER_* / modifier_*; engine-bound by modifier name)',
      sprite=dict(file=r'.gfx$', name=r'^GFX_MODIFIER_', tex=r'interface/modifiers/'),
      dirs=['gfx/interface/modifiers'], shelf=['icons/modifiers'],
      script=[dict(dir='common/modifier_definitions', pat=r'^\s*(?:icon|gfx)\s*=\s*"?([A-Za-z0-9_\-\.]+)"?', prefixes=['GFX_'], note='modifier_definitions may name an icon explicitly')],
      notes=['engine binds GFX_MODIFIER_<MODIFIER_KEY_UPPERCASE> automatically when it exists']),
    F('special_project_icon', 'Special project icon (project_icons)',
      sprite=dict(file=r'special_projects?/project\.gfx$|special_projects\.gfx$', name=r'^GFX_sp_', tex=r'special_project/project_icons/'),
      dirs=['gfx/interface/special_project/project_icons'], shelf=['icons/special_projects'],
      script=[dict(dir='common/special_projects', pat=ICON_PAT, prefixes=['GFX_sp_', 'GFX_'], note='project `icon = GFX_sp_*`; project/prototype `blueprint_image` is another key')]),
    F('special_project_other', 'Special project UI pictures (blueprint images, specializations, facility mapicons)',
      sprite=dict(file=r'special_projects?/project\.gfx$|special_projects\.gfx$', name=r'^GFX_(?!sp_)', tex=r'interface/special_project/(?!project_icons/)'),
      dirs=['gfx/interface/special_project::^(?!.*project_icons)'], shelf=[],
      script=[dict(dir='common/special_projects', pat=r'^\s*blueprint_image\s*=\s*"?([A-Za-z0-9_\-\.]+)"?', prefixes=['GFX_'], note='blueprint_image')]),
    F('officer_corps_spirit', 'Officer corps spirit icon (45x45)',
      sprite=dict(file=r'officer_corp_spirits\.gfx$', name=r'^GFX_idea_', tex=r'officer_corp/spirits/'),
      dirs=['gfx/interface/officer_corp/spirits'], shelf=['icons/officer_corps_spirits'],
      script=[dict(dir='common/ideas', pat=PIC_PAT, prefixes=['GFX_idea_'], note='officer-corps spirits are ideas (category officer_corps_spirit-like) and use `picture = spirit_idea_*`')]),
    F('officer_corps_ui', 'Officer corps UI art (banners, backgrounds, special_forces)',
      sprite=dict(file=r'countryofficercorpview\.gfx$', name=r'.', tex=r'interface/officer_corp/(?!spirits/)'),
      dirs=['gfx/interface/officer_corp::^(?!.*spirits)'], shelf=[]),
    F('raid_icons', 'Raids: map icons, category icons, terrain art, UI',
      sprite=dict(file=r'military_raids\.gfx$', name=r'.', tex=r'interface/military_raids/'),
      dirs=['gfx/interface/military_raids'], shelf=['icons/military_raids'],
      script=[dict(dir='common/raids', pat=r'^\s*(?:custom_map_icon|target_icon)\s*=\s*"?([A-Za-z0-9_\-\.]+)"?', prefixes=['GFX_'], note='raid `custom_map_icon` / `target_icon` (explicit GFX_)'),
              dict(dir='common/raids/categories', pat=r'^\s*(?:icon|picture|[a-z_]*icon)\s*=\s*"?([A-Za-z0-9_\-\.]+)"?', prefixes=['GFX_'], note='raid category icon keys')]),
    F('mio_department_icon', 'MIO department icon',
      sprite=dict(file=r'industrial_organization_department_icons\.gfx$', name=r'.', tex=r'military_industrial_organization/department_icons/'),
      dirs=['gfx/interface/military_industrial_organization/department_icons'], shelf=['icons/military_industrial_organizations'],
      script=[dict(dir='common/military_industrial_organization', pat=ICON_PAT, prefixes=['GFX_'], note='MIO / trait / department `icon = GFX_*`; initial_trait, trait, organization, policy all use `icon`')]),
    F('mio_trait_policy_icon', 'MIO trait and policy icons',
      sprite=dict(file=r'industrial_organization_policies_and_traits_icons\.gfx$', name=r'.', tex=r'military_industrial_organization/(trait_icons|policy_icons)/'),
      dirs=['gfx/interface/military_industrial_organization/trait_icons', 'gfx/interface/military_industrial_organization/policy_icons'], shelf=[]),
    F('bop_side_icon', 'Balance of power side / bar art',
      sprite=dict(file=r'powerbalanceview\.gfx$', name=r'.', tex=r'interface/bop/'),
      dirs=['gfx/interface/bop'], shelf=['icons/balance_of_power'],
      script=[dict(dir='common/bop', pat=r'^\s*(?:icon|gfx|set_power_balance_gfx)\s*=\s*"?([A-Za-z0-9_\-\.]+)"?', prefixes=['GFX_bop_', 'GFX_'], note='BoP definitions: side `icon`, range/gfx, `set_power_balance_gfx`')]),
    F('medal_icon', 'Medals (unit medals, ribbons)',
      sprite=dict(file=r'(^|/)medals\.gfx$', name=r'.', tex=r'interface/medals/'),
      dirs=['gfx/interface/medals'], shelf=['icons/medals'],
      script=[dict(dir='common/unit_medals', pat=ICON_PAT, prefixes=['GFX_'], note='unit_medals `icon = "GFX_medal_icon_*"` + `frame = N` selects the frame of a multi-frame medal sprite')]),
    F('intel_agency_logo', 'Intelligence agency logo (picture)',
      sprite=dict(file=r'countryintelligenceagencyview\.gfx$', name=r'^GFX_intelligence_agency_logo', tex=r'operatives/agencies/'),
      dirs=['gfx/interface/operatives/agencies'], shelf=['icons/intelligence_agency'],
      script=[dict(dir='common/intelligence_agencies', pat=PIC_PAT, prefixes=['GFX_'], note='agency `picture = GFX_intelligence_agency_logo_*`')]),
    F('intel_agency_upgrade_icon', 'Intelligence agency upgrade icon (68x68)',
      sprite=dict(file=r'countryintelligenceagencyview\.gfx$', name=r'^GFX_agency_', tex=r'operatives/icons/'),
      dirs=['gfx/interface/operatives/icons'], shelf=[],
      script=[dict(dir='common/intelligence_agency_upgrades', pat=PIC_PAT, prefixes=['GFX_'], note='upgrade `picture = GFX_agency_*`')]),
    F('operation_icon', 'Intelligence operation icon / map icon / phase picture',
      sprite=dict(file=r'lar_operations\.gfx$', name=r'.', tex=r'interface/operations/'),
      dirs=['gfx/interface/operations'], shelf=['icons/intelligence_operations'],
      script=[dict(dir='common/operations', pat=r'^\s*(?:icon|map_icon)\s*=\s*"?([A-Za-z0-9_\-\.]+)"?', prefixes=['GFX_'], note='operation `icon` and `map_icon`'),
              dict(dir='common/operation_phases', pat=r'^\s*(?:icon|picture)\s*=\s*"?([A-Za-z0-9_\-\.]+)"?', prefixes=['GFX_'], note='operation phase `icon` / `picture` (picture reuses 210x176 report-event art)')]),
    F('operative_portrait', 'Operative portrait (large)',
      sprite=dict(file=r'lar_portraits\.gfx$', name=r'^GFX_', tex=r'gfx/leaders/'),
      dirs=['dlc/dlc028_la_resistance/gfx/leaders::operative'], shelf=['portraits/operatives'], kind='portrait',
      script=[dict(dir='common/characters', pat=r'^\s*large\s*=\s*"?([A-Za-z0-9_\-\.]+)"?', prefixes=['GFX_'], note='operative character portraits via `portraits = { civilian = { large = GFX_portrait_... } }` (operative portraits sit in the same field)')]),
    F('commander_trait_icon', 'Unit leader (commander) trait icon',
      sprite=dict(file=r'unitleaderwindow\.gfx$', name=r'^GFX_trait_', tex=r'interface/traits/trait_'),
      dirs=['gfx/interface/traits::^trait_'], shelf=['icons/commander_traits'],
      script=[dict(dir='common/unit_leader', pat=r'^\s*(?:icon|slot_icon)\s*=\s*"?([A-Za-z0-9_\-\.]+)"?', prefixes=['GFX_trait_', 'GFX_'], note='unit_leader traits: icon is usually engine-bound as GFX_trait_<trait token>')]),
    F('personal_trait_icon', 'Character personal trait icon (traits/personal)',
      sprite=dict(file=r'.gfx$', name=r'^GFX_trait_', tex=r'interface/traits/personal/'),
      dirs=['gfx/interface/traits/personal'], shelf=[]),
    F('achievement_icon', 'Achievement icon (engine-loaded from gfx/achievements by key name; not in any .gfx)',
      sprite={}, dirs=['gfx/achievements'], shelf=['icons/achievements'], kind='file',
      script=[dict(dir='common/achievements.txt', pat=r'^(\w+)\s*=\s*\{', prefixes=[''], note='achievement key; icon files are <key>.dds, <key>_grey.dds, <key>_not_eligible.dds')]),
    F('faction_icon', 'Faction icon (logos) and rule/goal/upgrade art',
      sprite=dict(file=r'factions/factions\.gfx$', name=r'^GFX_', tex=r'interface/factions/faction_logos/'),
      dirs=['gfx/interface/factions/faction_logos'], shelf=['icons/factions'],
      script=[dict(dir='common/factions', pat=r'^\s*icon\s*=\s*"?([A-Za-z0-9_\-\.]+)"?', prefixes=['GFX_'], note='faction templates `icon = GFX_faction_*`')]),
    F('faction_other_art', 'Faction UI art (goals, rules, tabs, upgrades)',
      sprite=dict(file=r'factions/factions\.gfx$', name=r'^GFX_', tex=r'interface/factions/(?!faction_logos/)'),
      dirs=['gfx/interface/factions::^(?!.*faction_logos)'], shelf=[]),
    F('building_icon', 'Building / landmark icon',
      sprite=dict(file=r'buildings/landmarks\.gfx$', name=r'.', tex=r'interface/buildings/'),
      dirs=['gfx/interface/buildings'], shelf=['icons/buildings'],
      script=[dict(dir='common/buildings', pat=r'^\s*(?:special_icon|icon_frame)\s*=\s*"?([A-Za-z0-9_\-\.]+)"?', prefixes=['GFX_'], note='buildings: `icon_frame = N` and landmark/special icons `special_icon = GFX_*`')]),
    F('technology_picture', 'Technology pictures (all sizes in gfx/interface/technologies)',
      sprite=dict(file=r'(?i)technolog', name=r'.', tex=r'interface/technologies/'),
      dirs=['gfx/interface/technologies'], shelf=['icons/technologies', 'units/equipment/technology_art'],
      script=[dict(dir='common/technologies', pat=r'^\s*picture\s*=\s*"?([A-Za-z0-9_\-\.]+)"?', prefixes=['GFX_'], note='technology `picture = GFX_*` is optional; if absent engine uses GFX_<tech_token>_medal-style names'),
              dict(dir='common/technology_sharing', pat=PIC_PAT, prefixes=['GFX_'], note='tech-sharing group picture')]),
    F('doctrine_icon', 'Doctrine / subdoctrine icons',
      sprite=dict(file=r'doctrines/doctrines\.gfx$|Technologies\.gfx$', name=r'^GFX_(doctrine|rangers|paras|marines|.*_doctrine)', tex=r'interface/doctrines/'),
      dirs=['gfx/interface/doctrines'], shelf=[],
      script=[dict(dir='common/doctrines', pat=ICON_PAT, prefixes=['GFX_'], note='doctrine / subdoctrine `icon = GFX_doctrine_*`')]),
    F('unit_counter', 'Unit counters (subunit category / on-map / template icons in gfx/interface/counters)',
      sprite=dict(file=r'subuniticons\.gfx$|subunitcategories\.gfx$|anniversary_subunit_icons\.gfx$|navalcombat\.gfx$', name=r'.', tex=r'interface/counters/'),
      dirs=['gfx/interface/counters'], shelf=['units/land/counters_large', 'units/land/map_counters', 'units/air/map_counters', 'units/naval/map_counters'],
      script=[dict(dir='common/units', pat=r'^\s*(?:sprite|icon|small_sprite|large_sprite)\s*=\s*"?([A-Za-z0-9_\-\.]+)"?', prefixes=['GFX_'], note='sub_unit `sprite` / `icon` keys (GFX_unit_*, GFX_onmap_*)')]),
    F('division_template_emblem', 'Division template emblem (custom_template_*, large/small)',
      sprite=dict(file=r'subuniticons\.gfx$', name=r'.', tex=r'interface/counters/division_templates_(large|small)/'),
      dirs=['gfx/interface/counters/division_templates_large', 'gfx/interface/counters/division_templates_small'], shelf=['units/land/division_template_emblems'],
      script=[dict(dir='common/units', pat=r'^\s*(?:template_icon|division_template_icon)\s*=\s*"?([A-Za-z0-9_\-\.]+)"?', prefixes=['GFX_'], note='no script key found if zero; emblems are chosen in the division-template UI (`division_template = { ... }` in history has no emblem key)')]),
    F('flag_large', 'Country flag, large (82x52 TGA)', dirs=['=gfx/flags'], kind='file', shelf=['flags/normal'],
      script=[dict(dir='common/countries', pat=r'^\s*(?:flag|flags)\s*=\s*"?([A-Za-z0-9_\-\.]+)"?', prefixes=[''], note='flags are file-name convention gfx/flags/<TAG>[_<ideology>|_<variant>].tga with /medium and /small copies; no script key')]),
    F('flag_medium', 'Country flag, medium (41x26 TGA)', dirs=['gfx/flags/medium'], kind='file', shelf=['flags/medium']),
    F('flag_small', 'Country flag, small (10x7 TGA)', dirs=['gfx/flags/small'], kind='file', shelf=['flags/small']),
    F('leader_portrait', 'Country leader / general / admiral large portrait (156x210)',
      sprite=dict(file=r'_leader_portraits\.gfx$|_random_portraits\.gfx$|ww_portraits\.gfx$|portraits\.gfx$', name=r'^GFX_', tex=r'gfx/leaders/'),
      dirs=['gfx/leaders'], shelf=['portraits/leaders', 'portraits/commanders'], kind='portrait',
      script=[dict(dir='common/characters', pat=r'^\s*large\s*=\s*"?([A-Za-z0-9_\-\.]+)"?', prefixes=['GFX_'], note='characters: `portraits = { civilian|army|navy = { large = GFX_portrait_... small = GFX_...} }`'),
              dict(dir='common/country_leader', pat=r'^\s*sprite\s*=\s*"?([A-Za-z0-9_\-\.]+)"?', prefixes=['GFX_'], note='legacy country_leader `sprite`')]),
    F('scientist_portrait', 'Scientist portrait',
      sprite=dict(file=r'_scientists_portraits\.gfx$', name=r'.', tex=r'gfx/leaders/'),
      dirs=[], shelf=[], kind='portrait'),
    F('event_picture_report', 'Event picture: report card (210x176)',
      sprite=dict(file=r'eventpictures\.gfx$', name=r'^GFX_report_event', tex=r'event_pictures/'),
      dirs=['gfx/event_pictures::^report_event'], shelf=['event_art/report'], kind='gfx',
      script=[dict(dir='events', pat=PIC_PAT, prefixes=['GFX_'], note='event `picture = GFX_report_event_*` (explicit GFX_)')]),
    F('event_picture_news', 'Event picture: news (397x153)',
      sprite=dict(file=r'eventpictures\.gfx$', name=r'^GFX_news_event', tex=r'event_pictures/'),
      dirs=['gfx/event_pictures::^news_event'], shelf=['event_art/news'], kind='gfx'),
    F('event_picture_other', 'Event pictures not named report_/news_ (super events, test, etc.)',
      sprite=dict(file=r'eventpictures\.gfx$', name=r'^GFX_(?!report_event|news_event)', tex=r'event_pictures/'),
      dirs=['gfx/event_pictures::^(?!report_event|news_event)'], shelf=[], kind='gfx'),
    F('model_materials', 'Unit 3D model diffuse textures (gfx/models/units)', dirs=['gfx/models/units::diffuse|_spec|_normal'], shelf=['units/models_3d/air_materials', 'units/models_3d/land_materials', 'units/models_3d/naval_materials'], kind='file'),
]

# ----------------------------------------------------------------------------
def files_in_dirs(specs):
    out = {}
    for spec in specs:
        nonrec = spec.startswith('=')
        spec = spec.lstrip('=')
        rel, _, pat = spec.partition('::')
        rx = re.compile(pat, re.I) if pat else None
        for rn, rp in ROOTS:
            base = os.path.join(rp, rel.replace('/', os.sep))
            if rel.startswith('dlc/'):
                base = os.path.join(GAME, rel.replace('/', os.sep))
            if not os.path.isdir(base):
                continue
            for dp, dn, fn in os.walk(base):
                if nonrec:
                    dn[:] = []
                for f in fn:
                    if not f.lower().endswith(('.dds', '.tga', '.png')):
                        continue
                    if rx and not rx.search(f):
                        continue
                    p = os.path.join(dp, f)
                    r = os.path.relpath(p, rp).replace(BS, '/')
                    out[norm(r)] = p
    return out

def clean_raw(raw, n=700):
    raw = re.sub(r'[ \t]+', ' ', raw)
    raw = re.sub(r'\n\s*\n+', '\n', raw)
    return raw.strip()[:n]

def shelf_counts(dirs):
    res = {}
    for d in dirs:
        p = os.path.join(SHELF, d.replace('/', os.sep))
        n = 0
        if os.path.isdir(p):
            for dp, dn, fn in os.walk(p):
                if 'template' in dp.split(os.sep)[-1:]:
                    continue
                for f in fn:
                    if f.lower().endswith('.png') and f != 'contact_sheet.png':
                        n += 1
        res[d] = n
    return res

def measure_family(fam, S, name_index, tex_users, sample):
    sp = fam['sprite']
    sel = []
    if sp:
        fre = re.compile(sp.get('file', '.'), re.I)
        nre = re.compile(sp.get('name', '.'), re.I)
        xre = re.compile(sp['xname'], re.I) if sp.get('xname') else None
        tre = re.compile(sp.get('tex', '.'), re.I)
        for s in S:
            if fre.search(s['file']) and nre.search(s['name']) and tre.search(norm(s['tex'])) and not (xre and xre.search(s['name'])):
                sel.append(s)
    # texture set
    tex_files = {}
    missing = []
    for s in sel:
        p = resolve_tex(s['tex'], s['root'])
        if p:
            tex_files[norm(s['tex'])] = p
        else:
            missing.append(s['name'] + ' -> ' + s['tex'])
    dirfiles = files_in_dirs(fam['dirs']) if fam['dirs'] else {}
    if fam['measure_files']:
        tex_files = {}
        for r in fam['measure_files']:
            p = resolve_tex(r)
            if p:
                tex_files[norm(r)] = p
    elif not sel:
        tex_files = dict(dirfiles)
    # headers over ALL measured textures
    sizes = collections.Counter(); fmts = collections.Counter(); mips = collections.Counter(); ex = {}
    for r, p in tex_files.items():
        h = header(p)
        if not h:
            sizes['unreadable'] += 1
            continue
        k = '%dx%d' % (h['w'], h['h'])
        sizes[k] += 1; fmts[h['fmt']] += 1; mips[h['mips']] += 1
        ex.setdefault(k, os.path.relpath(p, GAME).replace(BS, '/'))
    n = sum(sizes.values())
    dom = sizes.most_common(1)[0] if sizes else (None, 0)
    nov1 = sum(1 for p in tex_files.values() if os.path.getmtime(p) >= NOV1_TS)
    newer = [os.path.relpath(p, GAME).replace(BS, '/') for p in tex_files.values() if os.path.getmtime(p) >= UPDATE_TS]
    gfxnew = sorted({'%s:%s' % (x['root'], x['file']) for x in sel if os.path.getmtime(os.path.join(RP[x['root']], x['file'].replace('/', os.sep))) >= UPDATE_TS})
    # pixel stats on even sample
    stats = []; fails = []
    for r in sample_even(list(tex_files), sample):
        st, err = pixel_stats(tex_files[r])
        if st:
            stats.append(st)
        else:
            fails.append(dict(file=os.path.relpath(tex_files[r], GAME).replace(BS, '/'), error=err))
    pix = {}
    if stats:
        ns = len(stats)
        pix = dict(sampled=ns,
                   median_fully_transparent_fraction=med([s['transparent'] for s in stats]),
                   median_semi_transparent_fraction=med([s['semi'] for s in stats]),
                   median_opaque_fraction=med([s['opaque'] for s in stats]),
                   fraction_files_with_any_alpha_below_255=round(sum(s['any_alpha'] for s in stats) / ns, 3),
                   median_border_ring_alpha=med([s['border_alpha'] for s in stats]),
                   median_corner_alpha=med([s['corner_alpha'] for s in stats]),
                   median_content_margins_LTRB_px=[med([s['margins'][i] for s in stats]) for i in range(4)],
                   median_chroma_of_opaque_px=med([s['chroma'] for s in stats]),
                   median_red_minus_blue=med([s['r_minus_b'] for s in stats]),
                   median_luma=med([s['mean_luma'] for s in stats]))
        pix['alpha_class'] = ('opaque canvas (no transparent px)' if pix['median_fully_transparent_fraction'] < 0.001 and pix['fraction_files_with_any_alpha_below_255'] < 0.2
                              else 'transparent background' if pix['median_fully_transparent_fraction'] >= 0.05
                              else 'mostly opaque with soft/partial alpha')
    # sprite registration facts
    reg = {}
    if sel:
        gf = collections.Counter(('%s:%s' % (s['root'], s['file'])) for s in sel)
        types = collections.Counter(s['type'] for s in sel)
        fr = collections.Counter(s['frames'] or '1(none)' for s in sel)
        keys = collections.Counter(k for s in sel for k in s['attrs'])
        eff = collections.Counter(s['attrs'].get('effectfile') for s in sel if s['attrs'].get('effectfile'))
        lt = collections.Counter(s['attrs'].get('loadtype') for s in sel if s['attrs'].get('loadtype'))
        # name rule
        match = 0
        mism = []
        for s in sel:
            stem = os.path.splitext(os.path.basename(norm(s['tex'])))[0]
            if s['name'].lower() == 'gfx_' + stem:
                match += 1
            elif len(mism) < 4:
                mism.append((s['name'], os.path.basename(s['tex'])))
        pref = collections.Counter('_'.join(s['name'].split('_')[:2]) for s in sel)
        # frame widths
        fw = collections.Counter()
        for s in sel:
            if s['frames'] and s['frames'].isdigit() and int(s['frames']) > 1:
                p = resolve_tex(s['tex'], s['root'])
                h = header(p) if p else None
                if h:
                    nfr = int(s['frames'])
                    fw['%dx%d f%d -> frame %s x %d' % (h['w'], h['h'], nfr, ('%d' % (h['w'] // nfr)) + ('' if h['w'] % nfr == 0 else ' (NOT integral: %.2f)' % (h['w'] / nfr)), h['h'])] += 1
        # companions: sprites named <family name>_suffix (any) not in selection
        selnames = {s['name'].lower() for s in sel}
        suff = collections.Counter(); sufex = {}
        sample_names = sample_even([s['name'] for s in sel], 400)
        for nm in sample_names:
            for nm2 in name_index.get(nm.lower() + '_', []):
                if nm2.lower() not in selnames:
                    sf = nm2[len(nm):]
                    suff[sf] += 1
                    sufex.setdefault(sf, nm2)
        # alias sprites: other sprites that point at family textures with other names
        alias = 0
        aex = []
        for r in list(tex_files)[:3000]:
            for u in tex_users.get(r, []):
                if u['name'].lower() not in selnames:
                    alias += 1
                    if len(aex) < 4:
                        aex.append(u['name'] + ' @ ' + u['file'])
        sd = collections.defaultdict(collections.Counter)
        for s2 in sel:
            p2 = resolve_tex(s2['tex'], s2['root'])
            h2 = header(p2) if p2 else None
            if h2:
                lastok = s2['name'].split('_')[-1].lower()
                sd[lastok]['%dx%d' % (h2['w'], h2['h']) + (' f%s' % s2['frames'] if s2['frames'] else '')] += 1
        suffix_dims = {k: dict(v.most_common(2)) for k, v in sorted(sd.items(), key=lambda kv: -sum(kv[1].values()))[:6]}
        example = clean_raw(sel[len(sel) // 3]['raw'])
        # example with frames / effectFile if present
        special = [s for s in sel if s['frames'] and s['frames'].isdigit() and int(s['frames']) > 1]
        eff_ex = [s for s in sel if s['attrs'].get('effectfile') or s['attrs'].get('loadtype') or 'animation' in s['attrs']]
        reg = dict(sprite_count=len(sel), gfx_files=gf.most_common(8), sprite_block_types=dict(types), noOfFrames_values=dict(fr.most_common(8)),
                   attribute_keys=dict(keys.most_common(14)), effectFile_values=dict(eff.most_common(4)), loadType_values=dict(lt),
                   name_equals_GFX_plus_texture_stem_fraction=round(match / len(sel), 3), name_vs_texture_mismatch_examples=mism,
                   name_prefix_top=dict(pref.most_common(6)), frame_width_derivation=dict(fw.most_common(6)),
                   companion_name_suffixes_found=dict(suff.most_common(8)), companion_examples={k: v for k, v in list(sufex.items())[:6]},
                   other_sprites_aliasing_family_textures=dict(count=alias, examples=aex),
                   last_name_token_vs_dominant_canvas=suffix_dims,
                   example_block_verbatim=example,
                   example_with_frames_verbatim=clean_raw(special[0]['raw']) if special else None,
                   example_with_effect_or_animation_verbatim=clean_raw(eff_ex[0]['raw'], 1800) if eff_ex else None)
    # orphans
    reg_norm = {norm(s['tex']) for s in sel}
    orphan = [r for r in dirfiles if r not in {norm(x) for x in reg_norm}] if (sel and dirfiles) else []
    # name pattern in files
    stems = collections.Counter('_'.join(os.path.basename(r).split('_')[:1]) for r in tex_files)
    return dict(
        id=fam['id'], label=fam['label'], kind=fam['kind'],
        texture_folders=fam['dirs'] or sorted({os.path.dirname(r) for r in tex_files})[:6],
        measured_texture_count=n, canvas_dominant=dom[0], canvas_dominant_count=dom[1],
        canvas_distribution_top=dict(sizes.most_common(10)), canvas_distinct=len(sizes),
        canvas_outlier_examples={k: v for k, v in list(ex.items())[:8]},
        dds_tga_formats=dict(fmts.most_common(6)), mipmap_counts=dict(mips.most_common(4)),
        filename_first_token_top=dict(stems.most_common(6)),
        measured_from_files_unchanged_since_update=(len(newer) == 0 and len(gfxnew) == 0),
        textures_modified_since_1_19_4_1_update=dict(count=len(newer), examples=newer[:5]), textures_with_mtime_on_or_after_2026_11_01=nov1, gfx_modified_since_update=gfxnew[:5],
        pixels=pix, pixel_decode_failures=fails,
        registration=reg,
        textures_on_disk_in_dirs_not_registered_by_any_family_sprite=dict(count=len(orphan), examples=orphan[:5]) if sel else None,
        sprites_with_missing_texture=dict(count=len(missing), examples=missing[:5]),
        script_references=None,
        evidence_paths=dict(example_textures=list(ex.values())[:4], gfx_files=[g[0] for g in reg.get('gfx_files', [])[:4]] if reg else []),
        shelf_files=shelf_counts(fam['shelf']),
    )

def idea_categories(names_lc, name_to_sprite):
    res = collections.defaultdict(lambda: collections.Counter())
    counts = collections.Counter()
    for p in read_txt_files('common/ideas'):
        if not p.endswith('.txt'):
            continue
        txt = open(p, encoding='utf-8-sig', errors='replace').read()
        for typ, body, raw in blocks(txt):
            if typ != 'ideas':
                continue
            for cat, cbody, craw in blocks(body):
                for idea, ibody, iraw in blocks(cbody):
                    a = top_attrs(ibody)
                    pic = a.get('picture')
                    if not pic:
                        continue
                    counts[cat] += 1
                    cand = [pic] if pic.startswith('GFX_') else ['GFX_idea_' + pic]
                    sprite = None
                    for c in cand:
                        sprite = name_to_sprite.get(c.lower())
                        if sprite:
                            break
                    if not sprite:
                        res[cat]['unresolved'] += 1
                        continue
                    pth = resolve_tex(sprite['tex'], sprite['root'])
                    h = header(pth) if pth else None
                    res[cat]['%dx%d' % (h['w'], h['h']) if h else 'missing_texture'] += 1
    out = {}
    for cat, c in sorted(res.items(), key=lambda kv: -sum(kv[1].values())):
        out[cat] = dict(ideas_with_picture=counts[cat], dims=dict(c.most_common(4)))
    return out

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--out', default=os.path.dirname(os.path.abspath(__file__)))
    ap.add_argument('--sample', type=int, default=48)
    a = ap.parse_args()
    S = index_sprites()
    name_index = collections.defaultdict(list)
    names_lc = set()
    name_to_sprite = {}
    for s in S:
        names_lc.add(s['name'].lower())
        name_to_sprite[s['name'].lower()] = s
    # prefix index: for companion lookup, map lowercase "<name>_" -> names starting with it
    allnames = sorted({s['name'] for s in S}, key=str.lower)
    import bisect
    lows = [n.lower() for n in allnames]
    class PI(dict):
        def get(self, key, default=None):
            i = bisect.bisect_left(lows, key)
            out = []
            while i < len(lows) and lows[i].startswith(key) and len(out) < 6:
                out.append(allnames[i]); i += 1
            return out
    name_index = PI()
    tex_users = collections.defaultdict(list)
    for s in S:
        if s['tex']:
            tex_users[norm(s['tex'])].append(s)
    out = []
    for fam in FAMILIES:
        r = measure_family(fam, S, name_index, tex_users, a.sample)
        r['script_references'] = [script_scan(sc, names_lc) for sc in fam['script']]
        r['notes'] = fam['notes']
        out.append(r)
        print('%-34s tex=%-5s dom=%s' % (fam['id'], r['measured_texture_count'], r['canvas_dominant']), file=sys.stderr)
    ic = idea_categories(names_lc, name_to_sprite)
    chk = {}
    for label, d, depth in (('technology_keys', 'common/technologies', 1), ('equipment_keys', 'common/units/equipment', 1)):
        keys = []
        for p in read_txt_files(d):
            if not p.endswith('.txt'):
                continue
            t = open(p, encoding='utf-8-sig', errors='replace').read()
            for typ, body, raw in blocks(t):
                if typ in ('technologies', 'equipments'):
                    keys += [k for k, b, r in blocks(body)]
        res = collections.Counter()
        for k in keys:
            for suf in ('', '_small', '_medium', '_large'):
                if ('gfx_' + k + suf).lower() in names_lc:
                    res['GFX_<key>' + suf] += 1
        chk[label] = dict(total_keys=len(keys), has_sprite_named=dict(res))
    ic['_key_to_sprite_checks'] = chk
    # shelf inventory
    shelf = {}
    for dp, dn, fn in os.walk(SHELF):
        rel = os.path.relpath(dp, SHELF).replace(BS, '/')
        n = len([f for f in fn if f.lower().endswith('.png') and f != 'contact_sheet.png'])
        if n:
            shelf[rel] = n
    doc = dict(game=GAME, game_version='1.19.4.1 (Operation Postern, launcher v1.19.4.1.99a5; coordinator-reported update at 2026-11-01 19:46 local)', roots=[r for r, _ in ROOTS], sprite_definitions_indexed=len(S),
               families=out, idea_categories=ic, shelf_inventory_png_counts=shelf)
    with open(os.path.join(a.out, 'families.json'), 'w', encoding='utf-8', newline='\n') as f:
        json.dump(doc, f, indent=1)
    print('wrote', os.path.join(a.out, 'families.json'), file=sys.stderr)

if __name__ == '__main__':
    main()
