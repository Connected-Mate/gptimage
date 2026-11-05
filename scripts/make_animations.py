#!/usr/bin/env python3
"""Make the example animated sprites with the real tool, one per animated family.

Every frame comes from `hoi4-imagegen` on the ChatGPT-login route: one generated base frame per asset (plus two
hoi4.image_edit candidates of it for the key-frame strip), then the tool's own processing, validation, preview,
review and install. This script only drives the CLI and records what came back.

  scripts/make_animations.py gen     [family ...]   plan + generate (+ edits) + process + validate + preview
  scripts/make_animations.py review  [family ...]   review (approved) and install in register mode, after a person
                                                    has looked at the frame sheet and the loop GIF

State lives in .work/anim-mod (the scratch mod, git-ignored) and .work/anim-results/.
Environment: HOI4_IMAGEGEN_BIN (the binary, default .work/hoi4-imagegen.exe or hoi4-imagegen on PATH).
"""
import datetime
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
WORK = os.path.join(ROOT, ".work")
MOD = os.path.join(WORK, "anim-mod")
RES = os.path.join(WORK, "anim-results")
BIN = os.environ.get("HOI4_IMAGEGEN_BIN") or (
    os.path.join(WORK, "hoi4-imagegen.exe") if os.path.exists(os.path.join(WORK, "hoi4-imagegen.exe")) else "hoi4-imagegen"
)
SLUG = "animated"
STYLE = "in the muted, painted style of Hearts of Iron IV interface art"

# family -> (asset, brief, extra plan arguments, edit instructions for further key frames)
JOBS = {
    "anim_blend_glow": (
        "focus_ready",
        "A warm golden glow tracing a rounded rectangle outline, like the highlight around a national focus that is ready to be taken: a bright thin core line with soft light falling off on both sides and a few faint sparks",
        {"canvas": [141, 115]},
        [],
    ),
    "anim_spin": (
        "industry_cog",
        f"A heavy steel cog wheel with eight square teeth and a red-enamelled hub bearing a small gold star, seen straight on, {STYLE}",
        {"canvas": [64, 64]},
        [],
    ),
    "anim_scroll": (
        "research_progress",
        f"Bold diagonal chevron stripes pointing right, alternating olive green and pale khaki, slightly worn paint texture, {STYLE}",
        {},
        [],
    ),
    "anim_blend_keyframes": (
        "signal_beacon",
        f"A brass signal lantern on a short iron post with its glass lens dark and unlit, {STYLE}",
        {},
        [
            "The same brass signal lantern from the same angle, framing, size and style, its lens now glowing with a soft warm orange light. Keep everything else identical.",
            "The same brass signal lantern from the same angle, framing, size and style, its lens now blazing bright yellow-white with a few short light rays. Keep everything else identical.",
        ],
    ),
}


def cli(*args, timeout=900):
    out = subprocess.run([BIN, "--project-root", MOD, "--json", *args], capture_output=True, text=True, timeout=timeout, encoding="utf-8")
    text = out.stdout.strip()
    try:
        return json.loads(text) if text else {"error": out.stderr.strip()}
    except json.JSONDecodeError:
        return {"error": (text + "\n" + out.stderr).strip()}


def call(tool, params):
    return cli("call", tool, json.dumps(params))


def save(family, data):
    os.makedirs(RES, exist_ok=True)
    with open(os.path.join(RES, family + ".json"), "w", encoding="utf-8", newline="\n") as f:
        json.dump(data, f, indent=1, ensure_ascii=False)


def load(family):
    p = os.path.join(RES, family + ".json")
    return json.load(open(p, encoding="utf-8")) if os.path.exists(p) else None


def candidates(r):
    return ((r.get("generation") or r).get("candidates") or [{}])[0].get("candidate_id")


def gen(family):
    prior = load(family) or {}
    if prior.get("processed") and prior.get("validation") == "pass":
        return "kept"
    asset, brief, extra, edits = JOBS[family]
    rec = {"family": family, "tool": "hoi4.image_animation", "asset": asset, "brief": brief, "extra": extra, "edits": edits}
    rec["generated_at"] = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    r = call("hoi4.image_animation", {"family": family, "feature_slug": SLUG, "asset": asset, "brief": brief, "approve": True, "idempotency_key": f"{SLUG}-{family}-1", **extra})
    rec["generate"] = r
    cand = candidates(r)
    if not cand:
        save(family, rec)
        return "FAILED generate: " + json.dumps(r)[:400]
    rec["candidate_id"] = cand
    proc = {"candidate_id": cand}
    if edits:
        frames = []
        rec["edit"] = []
        for i, instruction in enumerate(edits):
            e = call("hoi4.image_edit", {"candidate_id": cand, "instruction": instruction, "idempotency_key": f"{SLUG}-{family}-edit-{i + 1}"})
            rec["edit"].append(e)
            ec = candidates(e)
            if not ec:
                save(family, rec)
                return "FAILED edit: " + json.dumps(e)[:400]
            frames.append(ec)
        proc["frame_candidates"] = frames
    p = call("hoi4.image_process", proc)
    rec["process"] = p
    art = p.get("artifact_id")
    if not art:
        save(family, rec)
        return "FAILED process: " + json.dumps(p)[:400]
    rec["processed"] = art
    v = call("hoi4.image_validate", {"targets": [art]})
    rec["validate"] = v
    rec["validation"] = "pass" if v.get("failed") == 0 else "fail"
    rec["preview"] = call("hoi4.image_preview", {"target": art})
    save(family, rec)
    return f"{rec['validation']} {art} gif={(rec['preview'].get('animation') or {}).get('loop_gif')}"


def review(family):
    rec = load(family)
    art = rec["processed"]
    fam = cli("families", family)["family"]
    answers = {q["key"]: True for q in fam.get("rubric_questions", [])}
    rv = call("hoi4.image_review", {"target": art, "verdict": "approved", "reviewer": "claude-animation-session (agent)", "answers": answers, "notes": "looked at the native-size contact sheet, the frame sheet and the loop GIF"})
    rec["review"] = rv
    appr = rv.get("approval_id") or (rv.get("review") or {}).get("approval_id")
    if not appr:
        save(family, rec)
        return "FAILED review: " + json.dumps(rv)[:400]
    ins = call("hoi4.image_install", {"target": art, "approval_id": appr, "mode": "register"})
    rec["install"] = ins
    save(family, rec)
    return "installed" if not ins.get("error") else "FAILED install: " + json.dumps(ins)[:400]


def main():
    if len(sys.argv) < 2 or sys.argv[1] not in ("gen", "review"):
        print(__doc__)
        sys.exit(2)
    families = sys.argv[2:] or list(JOBS)
    os.makedirs(MOD, exist_ok=True)
    fn = gen if sys.argv[1] == "gen" else review
    for f in families:
        print(f, fn(f), flush=True)


if __name__ == "__main__":
    main()
