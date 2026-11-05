"""Per-brief style anchors from the family's whole vanilla folder, so the examples of one family do not all copy the
same few reference files (the automatic set gave every focus icon one Japanese plaque, 2026-11-05).

For each brief: score every vanilla file of the family's main size by the words it shares with the brief, prefer
generic files, leave out country-tag files unless the brief names that country, weigh down files used before, and
never hand out the same set twice. The chosen files are copied into the scratch mod (anchors must live inside it) as
PNG and passed as caller style_anchor references, which win over the vanilla form.
"""
import hashlib
import json
import os
import re

from PIL import Image

GAME = os.environ.get("HOI4_GAME_DIR", "C:/Program Files (x86)/Steam/steamapps/common/Hearts of Iron IV")
PER_BRIEF = 3
TAG = re.compile(r"(?:^|_)([A-Z]{3,})(?:_|$)")
NATIONS = {
    "finnish": "FIN", "brazilian": "BRA", "turkish": "TUR", "polish": "POL", "japanese": "JAP", "chinese": "CHI", "british": "ENG",
    "american": "USA", "soviet": "SOV", "italian": "ITA", "french": "FRA", "ethiopian": "ETH", "persian": "PER", "swedish": "SWE",
    "argentine": "ARG", "yugoslav": "YUG", "spanish": "SPR", "australian": "AST", "thai": "SIA", "siamese": "SIA", "mexican": "MEX",
    "german": "GER", "indian": "RAJ", "irish": "IRE", "greek": "GRE", "romanian": "ROM", "hungarian": "HUN", "czech": "CZE",
    "dutch": "HOL", "belgian": "BEL", "norwegian": "NOR", "portuguese": "POR", "canadian": "CAN", "egyptian": "EGY",
}
STOP = {"with", "from", "over", "into", "under", "their", "behind", "beside", "small", "large", "single", "feel", "style", "plain",
        "simple", "sturdy", "object", "invented", "colours", "colors", "national", "flags", "flag", "before", "across", "along"}
THEMES = {  # a theme word in the brief also matches these file-name words
    "industry": {"industry", "industrial", "factory", "steel", "production", "construction", "civilian"},
    "military": {"army", "military", "infantry", "artillery", "attack", "defence", "defense", "fortification", "doctrine"},
    "diplomacy": {"diplomacy", "treaty", "alliance", "ally", "befriend", "approach", "relation"},
    "politic": {"political", "politic", "government", "election", "democratic", "parliament", "propaganda", "ideology"},
    "naval": {"naval", "navy", "fleet", "battleship", "submarine", "destroyer", "carrier", "convoy"},
    "air": {"aviation", "bomber", "fighter", "airforce", "plane", "aircraft", "naval_bomber"},
    "research": {"research", "science", "technology", "scientist", "laboratory"},
}
# names that point at one country or region even inside generic files (befriend_usa, attack_yugoslavia, italy_propaganda)
PLACE = re.compile(r"_(befriend|attack|claim|annex|invade|demand|integrate|approach)_|_(usa|germany|italy|britain|england|france|japan|china|soviet|"
                   r"russia|turkey|sweden|bohemia|slovakia|yugoslavia|poland|asia|europe|africa|african|balkan|balkans|nordic|islamic|guyanas|"
                   r"waziristan|philippines|latin|american|arab|ottoman|baltic|scandinavian|georgia|albania|tuva|saudi|benelux|liechtenstein|iberia|iberian|vatican|scandinavia|scandinavism|coa|nordic|london|japanese|spanish|chinese|manchu|siamese)(_|$)")
# sub-families inside a family (ideas split into war, economy, politics and so on): a vanilla
# file belongs to the sub-family whose words its name shares most, and a brief draws its anchors only from its own
SUBFAMILIES = {
    "war": {"army", "military", "infantry", "artillery", "tank", "armor", "armour", "attack", "war", "warfare", "defence", "defense",
            "fortification", "fort", "doctrine", "soldier", "weapon", "rifle", "conscription", "mobilization", "mobilisation", "battle",
            "bunker", "trench", "guerilla", "guerrilla", "officer", "general", "division", "offensive", "border"},
    "naval": {"naval", "navy", "fleet", "ship", "battleship", "submarine", "destroyer", "carrier", "convoy", "port", "marine", "harbour",
              "harbor", "shipyard", "dockyard", "torpedo", "anchor", "cruiser"},
    "air": {"air", "aviation", "plane", "aircraft", "bomber", "fighter", "airforce", "airfield", "pilot", "bombing", "parachute", "paratrooper"},
    "economy": {"industry", "industrial", "factory", "economy", "economic", "trade", "resource", "production", "steel", "oil", "fuel", "mining",
                "mine", "agriculture", "farm", "farming", "railway", "railroad", "construction", "investment", "money", "gold", "export",
                "import", "infrastructure", "rubber", "coal", "electricity", "energy", "concern", "manufacturing", "works", "plant",
                "foundry", "assembly", "currency", "bank", "market", "harvest", "grain", "wheat", "ration", "supply"},
    "politics": {"political", "politic", "government", "election", "democratic", "democracy", "fascist", "fascism", "communist",
                 "communism", "propaganda", "party", "revolution", "ideology", "monarchy", "monarchist", "king", "crown", "parliament",
                 "ballot", "vote", "rally", "dictator", "regime", "leader", "council", "reform", "national", "nationalism", "unity"},
    "society": {"society", "social", "people", "education", "school", "health", "hospital", "religion", "religious", "church", "faith",
                "culture", "worker", "labour", "labor", "population", "women", "youth", "refugee", "family", "volunteer", "union",
                "spirit", "morale", "civil", "citizen", "welfare", "veteran", "festival"},
    "diplomacy": {"diplomacy", "diplomatic", "treaty", "alliance", "ally", "foreign", "embassy", "relation", "influence", "pact", "envoy",
                  "handshake", "peace", "neutrality", "guarantee", "league"},
    "research": {"research", "science", "scientific", "technology", "laboratory", "scientist", "nuclear", "rocket", "radar", "atomic",
                 "engineering", "university", "experiment", "jet", "computer"},
    "intelligence": {"spy", "intelligence", "agent", "cipher", "code", "secret", "police", "security", "espionage", "sabotage", "informant",
                     "surveillance", "codebreaking", "operative", "covert"},
}
_INDEX = {}
USED = {}  # family -> {"files": {path: count}, "sets": set of tuples}


_TAGS = None


def country_tags():
    """Every country tag the game has a flag for (GER, SPR, HOL ...), to spot lowercase tags such as focus_hol_ or _spr_."""
    global _TAGS
    if _TAGS is None:
        p = os.path.join(GAME, "common", "country_tags", "00_countries.txt")
        text = open(p, encoding="utf-8", errors="replace").read() if os.path.exists(p) else ""
        _TAGS = {t.lower() for t in re.findall(r"^\s*([A-Z][A-Z0-9]{2})\s*=", text, re.M)} - {"man", "can"}  # also English words
    return _TAGS


def nation_file(name, named):
    """True when the file belongs to one country or region; allowed only when the brief names that country."""
    tokens = name.lower().split("_")
    tagged = {t for t in tokens if t in country_tags()}  # real tags only (MODIFIER_ and NORDIC_ are not countries; places are in PLACE)
    placed = bool(PLACE.search(name.lower()))
    if not tagged and not placed:
        return False
    wanted = {t.lower() for t in named} | {adj for adj, t in NATIONS.items() if t in named}
    return not (wanted and (tagged & wanted or any(w in name.lower() for w in wanted)))


def subfamily(word_set):
    best = max(SUBFAMILIES, key=lambda s: len(word_set & SUBFAMILIES[s]))
    return best if word_set & SUBFAMILIES[best] else "other"


def name_words(rel):
    name = os.path.splitext(os.path.basename(rel))[0]
    raw = set(re.findall(r"[a-z]+", name.lower()))
    return words(name.replace("_", " ")) | {w for w in raw if len(w) == 3}  # keep short theme words such as war and air


def words(text):
    out = set()
    for w in re.findall(r"[a-z]+", text.lower()):
        if len(w) >= 4 and w not in STOP:
            out.add(w[:-1] if w.endswith("s") and len(w) > 4 else w)
    return out


def index(family, info, work):
    """Vanilla files of the family's main size, cached per family in .work/anchor-index/."""
    if family in _INDEX:
        return _INDEX[family]
    cache = os.path.join(work, "anchor-index", family + ".json")
    if os.path.exists(cache):
        _INDEX[family] = json.load(open(cache, encoding="utf-8"))
        return _INDEX[family]
    folder = info["folder"].split("{")[0].rstrip("/")
    size = (((info.get("measured") or {}).get("distribution") or [{}])[0] or {}).get("size") or info.get("canvas")
    prefixes = tuple([info.get("prefix") or ""] + list(info.get("alt_prefixes") or []))
    root = os.path.join(GAME, folder)

    def scan(prefix_filter):
        found = []
        for d, _, fs in os.walk(root):
            for f in fs:
                if not f.lower().endswith((".dds", ".png", ".tga")) or (prefix_filter and not f.startswith(prefixes)):
                    continue
                p = os.path.join(d, f)
                try:
                    with Image.open(p) as im:
                        if size and list(im.size) != list(size):
                            continue
                except Exception:  # noqa: BLE001 - a file Pillow cannot open is simply not a candidate
                    continue
                found.append(os.path.relpath(p, GAME).replace("\\", "/"))
        return found

    files = scan(bool(prefixes[0]))
    if len(files) < 2 * PER_BRIEF and prefixes[0]:
        files = scan(False)  # vanilla names some families differently from the mod prefix (MIO, modifiers, GUI)
    os.makedirs(os.path.dirname(cache), exist_ok=True)
    json.dump(sorted(files), open(cache, "w", encoding="utf-8"), indent=0)
    _INDEX[family] = sorted(files)
    return _INDEX[family]


def pick(family, info, asset, brief, mod, work):
    files = index(family, info, work)
    if len(files) < PER_BRIEF:
        return []
    named = {t for n, t in NATIONS.items() if n in brief.lower()}
    bw = words(brief)
    for theme, extra in THEMES.items():
        if re.search(r"\b" + theme, brief.lower()):
            bw |= extra
    used = USED.setdefault(family, {"files": {}, "sets": set()})
    seed = int(hashlib.sha256(asset.encode()).hexdigest()[:8], 16)
    generic = [f for f in files if "generic" in os.path.basename(f)]
    if len(generic) >= 30:
        files = generic  # enough generic art: never borrow a country's own icon
    # a family whose vanilla art is nearly all one country's or another (bop sides, category pictures) may use
    # country files, still varied per brief, rather than fall back to the one automatic set; the refs say so
    open_files = [f for f in files if not nation_file(os.path.splitext(os.path.basename(f))[0], named)]
    nation_fallback = len(open_files) < 2 * PER_BRIEF
    scored, theme = [], {}
    for i, rel in enumerate(files):
        name = os.path.splitext(os.path.basename(rel))[0]
        if not nation_fallback and nation_file(name, named):
            continue
        tags = set(TAG.findall(name))
        nw = words(name.replace("_", " "))
        score = 2.0 * len(bw & nw) + (0.5 if "generic" in name else 0) + (1.0 if tags & named else 0)
        score -= 0.6 * used["files"].get(rel, 0)
        scored.append((score, (seed + i * 2654435761) % 1000003, rel))
        theme[rel] = len(bw & nw)
    scored.sort(key=lambda s: (-s[0], s[1]))
    # each successive brief of the family draws from the next style group, so the examples span vanilla's range of
    # frames and palettes; within the group the theme-matched files come first
    allowed = {r for _, _, r in scored}
    # the brief's sub-family: anchors come only from the vanilla files of the same sub-family when it has enough
    subs = {r: subfamily(name_words(r)) for r in allowed}
    write_subfamilies(family, subs, work)
    # the asset name says the theme even when the brief does not (ration_coupon_booklet is economy)
    brief_words = bw | words(asset.replace("_", " ")) | {w for w in re.findall(r"[a-z]+", (brief + " " + asset.replace("_", " ")).lower()) if len(w) == 3}
    sub = subfamily(brief_words)
    members = {r for r in allowed if subs[r] == sub}
    if sub != "other" and len(members) >= PER_BRIEF + 1:
        allowed = members
        scored = [s for s in scored if s[2] in members]
        key = f"{family}.{sub}"
    else:
        sub, key = "all", family
    groups = [[f for f in grp if f in allowed] for grp in style_groups(key, sorted(allowed), work)] if len(allowed) >= 4 * PER_BRIEF else []
    groups = [grp for grp in groups if len(grp) >= PER_BRIEF]
    if groups:
        # the least-used group that holds at least one file on the brief's theme (any group if nothing matches)
        gused = used.setdefault("groups:" + key, [0] * len(groups))
        fits = [j for j, grp in enumerate(groups) if any(theme.get(f) for f in grp)] or list(range(len(groups)))
        j = min(fits, key=lambda j: (gused[j], j))
        gused[j] += 1
        grp = set(groups[j])
        scored = [s for s in scored if s[2] in grp]
    # numbered variants of one icon (guerilla_base_0, _1, _2) count once
    pool, bases = [], set()
    for _, _, r in scored:
        base = re.sub(r"_?\d+$", "", os.path.splitext(os.path.basename(r))[0])
        if base not in bases:
            bases.add(base)
            pool.append(r)
        if len(pool) == 24:
            break
    chosen = None
    for start in range(0, max(1, len(pool) - PER_BRIEF + 1)):
        cand = tuple(sorted(pool[start:start + PER_BRIEF]))
        if len(cand) == PER_BRIEF and cand not in used["sets"]:
            chosen = cand
            break
    if not chosen:
        return []
    used["sets"].add(chosen)
    refs = []
    for rel in chosen:
        used["files"][rel] = used["files"].get(rel, 0) + 1
        dst_rel = f"references/vanilla/{family}/{os.path.splitext(os.path.basename(rel))[0]}.png"
        dst = os.path.join(mod, dst_rel)
        if not os.path.exists(dst):
            os.makedirs(os.path.dirname(dst), exist_ok=True)
            Image.open(os.path.join(GAME, rel)).convert("RGBA").save(dst)
        refs.append({"path": dst_rel, "role": "style_anchor", "vanilla_source": rel, "subfamily": sub, "country_files_allowed": nation_fallback})
    return refs


def write_subfamilies(family, subs, work):
    """The sub-family groups of a family's vanilla files, once, for the gallery page: .work/anchor-index/<family>-subfamilies.json."""
    cache = os.path.join(work, "anchor-index", family + "-subfamilies.json")
    if os.path.exists(cache):
        return
    out = {}
    for rel, s in sorted(subs.items()):
        out.setdefault(s, []).append(rel)
    json.dump(out, open(cache, "w", encoding="utf-8"), indent=0)


STYLE_GROUPS = 8
_GROUPS = {}


def _style_feature(path):
    import numpy as np
    im = Image.open(path).convert("RGBA")
    a = np.asarray(im.getchannel("A").resize((12, 12)), dtype=np.float32).flatten() / 255
    hsv = np.asarray(im.convert("RGB").convert("HSV").resize((24, 24)), dtype=np.float32) / 255
    m = np.asarray(im.getchannel("A").resize((24, 24))) > 128
    px = hsv[m] if m.any() else hsv.reshape(-1, 3)
    hh = np.histogram(px[:, 0], bins=8, range=(0, 1), weights=px[:, 1])[0]
    hh = hh / (hh.sum() + 1e-6)
    return np.concatenate([a * 1.5, hh, [px[:, 1].mean() * 3, px[:, 2].mean() * 2, m.mean() * 3]])


def style_groups(family, files, work):
    """The family's candidate files split into frame-and-colour style groups (k-means on alpha shape and palette),
    largest first, cached in .work/anchor-index/<family>-styles.json. Vanilla's own icons differ in frame, plaque,
    colour and composition, and a family's examples should cover that range rather than one look."""
    if family in _GROUPS:
        return _GROUPS[family]
    cache = os.path.join(work, "anchor-index", family + "-styles.json")
    if os.path.exists(cache):
        _GROUPS[family] = json.load(open(cache, encoding="utf-8"))
        return _GROUPS[family]
    import numpy as np
    X = np.array([_style_feature(os.path.join(GAME, f)) for f in files])
    k = min(STYLE_GROUPS, max(1, len(files) // 6))
    rng = np.random.default_rng(0)
    C = X[rng.choice(len(X), k, replace=False)]
    for _ in range(50):
        lab = ((X[:, None, :] - C[None]) ** 2).sum(-1).argmin(1)
        C = np.array([X[lab == j].mean(0) if (lab == j).any() else C[j] for j in range(k)])
    groups = {}
    for f, j in zip(files, lab):
        groups.setdefault(int(j), []).append(f)
    out = [groups[j] for j in sorted(groups, key=lambda j: -len(groups[j])) if len(groups[j]) >= PER_BRIEF]
    json.dump(out, open(cache, "w", encoding="utf-8"), indent=0)
    _GROUPS[family] = out
    return out
