//! Parity of the Rust recipes with the Agentic HOI4 Modding Kit's Python tools, plus the golden test against the
//! installed game's report-card alpha.
//!
//! Each recipe runs on deterministic synthetic input in both languages:
//!  * report card: the kit's `process_report_event_image.py` with the final defaults on its command line;
//!  * achievement triplet: `process_achievement_icons.py` composition with the kit's template PNGs (exact);
//!  * advisor card: `create_advisor_icon.py` `compose` with the measured opening and the kit's template;
//!  * portrait: `compute_portrait_crop` and `encode_processed_portrait`.
//!
//! Tolerances are justified in the comments next to each constant; the measured numbers are printed with
//! `--nocapture`. Tests print `SKIPPED <reason>` and pass vacuously when Python, Pillow, numpy or the kit are
//! missing, unless `IMAGEGEN_REQUIRE_PARITY=1`, in which case a missing prerequisite fails the test (CI sets it).
//! Set `HOI4_KIT_DIR` and `HOI4_IMAGEGEN_PYTHON` to point at a kit checkout and an interpreter, and
//! `HOI4_GAME_DIR` at the game for the golden test. Temporary files live under `<repo>/.work/`. Vanilla pixels are
//! read at test time and never written anywhere.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use imagegen_core::dds;
use imagegen_core::pixels::{decode, encode_png, DecodeLimits, Rgba};
use imagegen_core::recipes::achievement::{achievement_triplet, load_templates_from_files};
use imagegen_core::recipes::advisor::{advisor_card, measure_opening_geometry, AdvisorParams};
use imagegen_core::recipes::portrait::{compute_portrait_crop, portrait_from_crop};
use imagegen_core::recipes::report_card::{report_card, ReportCardParams};
use serde_json::{json, Value};

// ---------------------------------------------------------------- tolerances (see the measurements in the report)

/// Report card, interior of the card (both alpha 255). The tone chain, grain and compositing are bit-exact ports;
/// the remaining difference is Lanczos (Pillow keeps an 8-bit intermediate between passes, we stay in f64) and the
/// supersampled rotation, so a few levels at most.
const CARD_RGB_MAE_MAX: f64 = 0.75;
const CARD_RGB_ERR_MAX: i32 = 8;
/// Report card, whole canvas alpha. Pillow's GaussianBlur is three box blurs, ours is a true Gaussian.
const CARD_ALPHA_MAE_MAX: f64 = 1.0;
const CARD_ALPHA_ERR_MAX: i32 = 16;
/// The same for a translucent source, whose own alpha adds to the rotation difference on the card's anti-aliased
/// edge. Measured once the kit carried the padded shadow: 13 of 37,000 pixels over 12, all on that edge, max 17,
/// whole-canvas MAE 0.22 (the MAE limit above still applies).
const CARD_ALPHA_ERR_MAX_TRANSLUCENT: i32 = 20;
/// Advisor, whole card. Same bicubic arithmetic as Pillow, so the card should match to the last bit apart from
/// sepia/angle float noise.
const ADVISOR_MAE_MAX: f64 = 0.5;
const ADVISOR_ERR_MAX: i32 = 2;
/// Portrait: Lanczos resize only.
const PORTRAIT_MAE_MAX: f64 = 0.5;
const PORTRAIT_ERR_MAX: i32 = 8;
/// Golden: alpha RMSE against the vanilla templates (the coordinator's measured scores for the Python tool were
/// 10.01, 11.06, 15.96 and 16.88 against the 230, 180, 117 and 5-file templates).
const GOLDEN_RMSE_MAX: f64 = 12.0;

// ---------------------------------------------------------------- plumbing

fn require_parity() -> bool {
    std::env::var("IMAGEGEN_REQUIRE_PARITY").map(|v| v == "1").unwrap_or(false)
}

/// Either fail (parity required) or print SKIPPED and let the caller return.
fn skip(test: &str, reason: &str) {
    if require_parity() {
        panic!("IMAGEGEN_REQUIRE_PARITY=1 but {test} cannot run: {reason}");
    }
    eprintln!("SKIPPED {test}: {reason}; this parity check is UNAVAILABLE in this run");
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn find_kit() -> Option<PathBuf> {
    let candidates = [std::env::var("HOI4_KIT_DIR").ok().map(PathBuf::from), Some(repo_root().join("..").join("agentic-hoi4-modding"))];
    candidates.into_iter().flatten().find(|p| p.join(".claude/skills/hoi4-feature-assets/tools/process_report_event_image.py").is_file())
}

fn find_python() -> Option<Vec<String>> {
    let mut options: Vec<Vec<String>> = Vec::new();
    if let Ok(p) = std::env::var("HOI4_IMAGEGEN_PYTHON") {
        options.push(vec![p]);
    }
    options.push(vec!["py".into(), "-3.13".into()]);
    options.push(vec!["py".into(), "-3".into()]);
    options.push(vec!["python3".into()]);
    options.push(vec!["python".into()]);
    options.into_iter().find(|cmd| {
        Command::new(&cmd[0])
            .args(&cmd[1..])
            .args(["-c", "import sys, numpy, PIL; assert sys.version_info >= (3, 10)"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
}

struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn workdir(tag: &str) -> (PathBuf, Cleanup) {
    let dir = repo_root().join(".work").join(format!("recipe-parity-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    (dir.clone(), Cleanup(dir))
}

struct Env {
    kit: PathBuf,
    python: Vec<String>,
}

fn env(test: &str) -> Option<Env> {
    let Some(kit) = find_kit() else {
        skip(test, "no kit checkout found (set HOI4_KIT_DIR)");
        return None;
    };
    let Some(python) = find_python() else {
        skip(test, "no Python >= 3.10 with numpy and Pillow found (set HOI4_IMAGEGEN_PYTHON)");
        return None;
    };
    Some(Env { kit, python })
}

fn run_python(env: &Env, script: &str, work: &Path, extra: &[&str]) {
    std::fs::write(work.join("driver.py"), script).unwrap();
    let out = Command::new(&env.python[0])
        .args(&env.python[1..])
        .arg(work.join("driver.py"))
        .arg(&env.kit)
        .arg(work)
        .args(extra)
        .output()
        .expect("python starts");
    assert!(out.status.success(), "python driver failed:\n{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
}

fn read_rgba(path: &Path, w: u32, h: u32) -> Rgba {
    Rgba::from_raw(w, h, std::fs::read(path).unwrap()).unwrap()
}

struct Rng(u32);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
}

/// A picture with edges, gradients, a disc and a little noise, opaque unless `alpha` is set.
fn scene(w: u32, h: u32, seed: u32, alpha: bool) -> Rgba {
    let mut rng = Rng(seed);
    let mut img = Rgba::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (f64::from(x) / f64::from(w), f64::from(y) / f64::from(h));
            let d = ((fx - 0.4).powi(2) + (fy - 0.45).powi(2)).sqrt();
            let mut c = [(40.0 + 160.0 * fx) as i32, (30.0 + 120.0 * fy) as i32, (200.0 - 150.0 * fx * fy) as i32];
            if d < 0.22 {
                c = [230, 200, 150];
            }
            if (x / 16 + y / 16) % 7 == 0 {
                c[0] = 255 - c[0];
            }
            let n = (rng.next() % 17) as i32 - 8;
            let a = if alpha && d > 0.45 { 0 } else { 255 };
            img.set_px(x, y, [(c[0] + n).clamp(0, 255) as u8, (c[1] + n).clamp(0, 255) as u8, (c[2] + n).clamp(0, 255) as u8, a]);
        }
    }
    img
}

#[derive(Debug, Default)]
struct Diff {
    /// Mean absolute error per channel R, G, B, A over the compared pixels.
    mae: [f64; 4],
    max: [i32; 4],
    compared: usize,
    exact: usize,
}

fn diff(a: &Rgba, b: &Rgba, mask: impl Fn(usize) -> bool) -> Diff {
    assert_eq!((a.width, a.height), (b.width, b.height), "sizes differ");
    let mut d = Diff::default();
    let mut sum = [0u64; 4];
    for (i, (pa, pb)) in a.data.chunks_exact(4).zip(b.data.chunks_exact(4)).enumerate() {
        if !mask(i) {
            continue;
        }
        d.compared += 1;
        let mut same = true;
        for c in 0..4 {
            let e = (i32::from(pa[c]) - i32::from(pb[c])).abs();
            sum[c] += e as u64;
            d.max[c] = d.max[c].max(e);
            same &= e == 0;
        }
        d.exact += usize::from(same);
    }
    for (m, s) in d.mae.iter_mut().zip(sum) {
        *m = s as f64 / d.compared.max(1) as f64;
    }
    d
}

fn rgb_mae(d: &Diff) -> f64 {
    (d.mae[0] + d.mae[1] + d.mae[2]) / 3.0
}

fn bbox_close(a: &Rgba, b: &Rgba, tol: i64) {
    let (ba, bb) = (a.alpha_bbox(1).expect("rust bbox"), b.alpha_bbox(1).expect("python bbox"));
    for k in 0..4 {
        assert!((i64::from(ba[k]) - i64::from(bb[k])).abs() <= tol, "alpha bbox differs: rust {ba:?} python {bb:?}");
    }
}

// ---------------------------------------------------------------- report card

const REPORT_DRIVER: &str = r#"
import subprocess, sys, json
from pathlib import Path
from PIL import Image
kit = Path(sys.argv[1]); work = Path(sys.argv[2])
tool = kit / '.claude/skills/hoi4-feature-assets/tools/process_report_event_image.py'
src = tool.read_text(encoding='utf-8')
fixed = 'ceil' in src
cases = json.loads((work / 'cases.json').read_text())
for case in cases:
    name = case['name']
    cmd = [sys.executable, str(tool), str(work / (name + '.png')), str(work / (name + '.py.png'))] + case['args']
    subprocess.run(cmd, check=True, capture_output=True)
    im = Image.open(work / (name + '.py.png')).convert('RGBA')
    (work / (name + '.py.rgba')).write_bytes(im.tobytes())
(work / 'meta.json').write_text(json.dumps({'padded_shadow_in_python': fixed}))
"#;

#[test]
fn report_card_matches_the_kit_python() {
    let Some(env) = env("report_card parity") else { return };
    let (work, _guard) = workdir("report");
    // The Rust defaults are the final ones; the Python tool is run with the same numbers on its command line, so
    // the comparison does not depend on which defaults the kit copy on disk carries.
    let args: Vec<String> =
        ["--card-size", "192x154", "--angle", "4.0", "--shadow-offset", "0", "1", "--shadow-blur", "2.5", "--shadow-opacity", "0.96"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    let cases = [
        ("wide", scene(320, 200, 11, false)),
        ("tall", scene(150, 260, 12, false)),
        ("exact", scene(192, 154, 13, false)),
        ("translucent", scene(240, 180, 14, true)),
    ];
    let manifest: Vec<Value> = cases.iter().map(|(n, _)| json!({"name": n, "args": args})).collect();
    for (name, img) in &cases {
        std::fs::write(work.join(format!("{name}.png")), encode_png(img).unwrap()).unwrap();
    }
    std::fs::write(work.join("cases.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
    run_python(&env, REPORT_DRIVER, &work, &[]);
    let meta: Value = serde_json::from_slice(&std::fs::read(work.join("meta.json")).unwrap()).unwrap();
    let python_padded = meta["padded_shadow_in_python"].as_bool().unwrap();

    let params = ReportCardParams::default();
    for (name, img) in &cases {
        let ours = report_card(img, &params).unwrap();
        let theirs = read_rgba(&work.join(format!("{name}.py.rgba")), 210, 176);
        assert_eq!((ours.width, ours.height), (210, 176), "{name}: canvas size");
        // Geometry: the card is the same silhouette. The old Python shadow was clipped at the card box, which only
        // shrinks the visible bbox by the shadow skirt, so the bbox is compared on the card (alpha above 200).
        let core = |i: &Rgba| {
            let mut c = i.clone();
            for p in c.data.chunks_exact_mut(4) {
                p[3] = if p[3] > 200 { 255 } else { 0 };
            }
            c
        };
        bbox_close(&core(&ours), &core(&theirs), 1);
        // Interior of the card: both opaque. Compares the tone chain, grain, rotation and compositing.
        let interior = |i: usize| ours.data[i * 4 + 3] == 255 && theirs.data[i * 4 + 3] == 255;
        let inner = diff(&ours, &theirs, interior);
        eprintln!(
            "report_card {name}: card interior {} px, RGB MAE {:.3} (R {:.3} G {:.3} B {:.3}), max RGB error {:?}, exact pixels {:.1}%",
            inner.compared,
            rgb_mae(&inner),
            inner.mae[0],
            inner.mae[1],
            inner.mae[2],
            &inner.max[..3],
            100.0 * inner.exact as f64 / inner.compared as f64
        );
        assert!(inner.compared > 10_000, "{name}: card interior too small");
        assert!(rgb_mae(&inner) <= CARD_RGB_MAE_MAX, "{name}: RGB MAE {}", rgb_mae(&inner));
        assert!(inner.max[..3].iter().all(|m| *m <= CARD_RGB_ERR_MAX), "{name}: max error {:?}", inner.max);
        // Whole-canvas alpha, shadow included. Only meaningful when the Python tool has the padded shadow.
        let all = diff(&ours, &theirs, |_| true);
        eprintln!("report_card {name}: whole canvas alpha MAE {:.3}, max {} (python padded shadow: {python_padded})", all.mae[3], all.max[3]);
        if python_padded {
            assert!(all.mae[3] <= CARD_ALPHA_MAE_MAX, "{name}: alpha MAE {}", all.mae[3]);
            let limit = if *name == "translucent" { CARD_ALPHA_ERR_MAX_TRANSLUCENT } else { CARD_ALPHA_ERR_MAX };
            assert!(all.max[3] <= limit, "{name}: alpha max error {}", all.max[3]);
        } else {
            // The kit copy still clips the shadow at the card box, so the shadow skirt is expected to differ; the
            // card-interior comparison above is what this run proves.
            eprintln!("report_card {name}: kit copy has the OLD shadow; alpha outside the card interior was reported, not asserted");
        }
    }
}

// ---------------------------------------------------------------- golden test against the game

fn find_game() -> Option<PathBuf> {
    let candidates = [
        std::env::var("HOI4_GAME_DIR").ok().map(PathBuf::from),
        Some(PathBuf::from("C:/Program Files (x86)/Steam/steamapps/common/Hearts of Iron IV")),
        Some(PathBuf::from("C:/Program Files/Steam/steamapps/common/Hearts of Iron IV")),
    ];
    candidates.into_iter().flatten().find(|p| p.join("gfx/event_pictures").is_dir())
}

fn alpha_rmse(a: &Rgba, template: &[u8]) -> f64 {
    let n = template.len();
    let sum: f64 = a.data.chunks_exact(4).zip(template).map(|(p, t)| (f64::from(p[3]) - f64::from(*t)).powi(2)).sum();
    (sum / n as f64).sqrt()
}

#[test]
fn report_card_alpha_matches_the_vanilla_templates() {
    // the golden needs the installed game, which no CI runner has: always a plain skip, even when Python parity is
    // required (IMAGEGEN_REQUIRE_PARITY covers the kit's Python tools, not the game)
    let Some(game) = find_game() else {
        eprintln!("SKIPPED report_card golden: the game is not installed (set HOI4_GAME_DIR); UNAVAILABLE in this run");
        return;
    };
    let dir = game.join("gfx/event_pictures");
    let mut groups: HashMap<Vec<u8>, usize> = HashMap::new();
    let mut files = 0;
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            n.starts_with("report_event") && n.ends_with(".dds")
        })
        .collect();
    entries.sort();
    for path in entries {
        let bytes = std::fs::read(&path).unwrap();
        let Ok(info) = dds::parse_header(&bytes) else { continue };
        if (info.width, info.height) != (210, 176) {
            continue;
        }
        let Ok(img) = dds::decode(&bytes) else { continue };
        files += 1;
        let alpha: Vec<u8> = img.data.chunks_exact(4).map(|p| p[3]).collect();
        *groups.entry(alpha).or_insert(0) += 1;
    }
    if files == 0 {
        skip("report_card golden", "no 210x176 report_event DDS files found in the game folder");
        return;
    }
    let mut ranked: Vec<(Vec<u8>, usize)> = groups.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let counts: Vec<usize> = ranked.iter().map(|(_, n)| *n).collect();
    eprintln!("golden: {files} report_event files of 210x176 fall into {} alpha groups {counts:?}", ranked.len());

    let ours = report_card(&scene(320, 200, 11, false), &ReportCardParams::default()).unwrap();
    let scores: Vec<f64> = ranked.iter().map(|(a, _)| alpha_rmse(&ours, a)).collect();
    for (i, ((_, n), s)) in ranked.iter().zip(&scores).enumerate() {
        eprintln!("golden: alpha RMSE against template {i} ({n} files): {s:.2}");
    }
    // The two largest templates must match; the others are reported (they are rarer tilts and the 5-file one is an
    // outlier by design).
    for (i, s) in scores.iter().take(2).enumerate() {
        assert!(*s <= GOLDEN_RMSE_MAX, "alpha RMSE {s:.2} against vanilla template {i} exceeds {GOLDEN_RMSE_MAX}");
    }
}

// ---------------------------------------------------------------- achievement

const ACHIEVEMENT_DRIVER: &str = r#"
import importlib.util, json, sys
from pathlib import Path
kit = Path(sys.argv[1]); work = Path(sys.argv[2])
tools = kit / '.claude/skills/hoi4-feature-assets/tools'
spec = importlib.util.spec_from_file_location('ach', tools / 'process_achievement_icons.py')
import types
m = importlib.util.module_from_spec(spec); sys.modules['ach'] = m; spec.loader.exec_module(m)
from PIL import Image
t = m.load_templates()
def load(n): return Image.frombytes('RGBA', (64, 64), (work / (n + '.rgba')).read_bytes())
src = m.StateImages(load('completed'), load('grey'), load('not_eligible'))
out = m._compose_states(src, t)
for n, im in (('completed', out.completed), ('grey', out.grey), ('not_eligible', out.not_eligible)):
    (work / (n + '.py.rgba')).write_bytes(im.tobytes())
"#;

#[test]
fn achievement_triplet_is_identical_to_the_kit_python() {
    let Some(env) = env("achievement parity") else { return };
    let tdir = env.kit.join(".claude/skills/hoi4-feature-assets/assets/vanilla_reference/icons/achievements/template");
    if !tdir.join("achievement_template.png").is_file() {
        skip("achievement parity", "the kit's achievement template PNGs are missing");
        return;
    }
    let (work, _guard) = workdir("achievement");
    let templates = load_templates_from_files(&tdir.join("achievement_template.png"), &tdir.join("achievement_template_grey.png"), true).unwrap();
    // Inner scenes that leave the 6 px ring transparent so the "already templated" guard does not fire, with
    // soft edges so the integer composite is exercised on partial alpha.
    let mut states = Vec::new();
    for (i, name) in ["completed", "grey", "not_eligible"].iter().enumerate() {
        let mut img = scene(64, 64, 30 + i as u32, false);
        for y in 0..64u32 {
            for x in 0..64u32 {
                let d = ((f64::from(x) - 32.0).powi(2) + (f64::from(y) - 32.0).powi(2)).sqrt();
                let a = (255.0 - (d - 22.0) * 40.0).clamp(0.0, 255.0) as u8;
                let p = img.px(x, y);
                img.set_px(x, y, [p[0], p[1], p[2], a]);
            }
        }
        std::fs::write(work.join(format!("{name}.rgba")), &img.data).unwrap();
        states.push(img);
    }
    run_python(&env, ACHIEVEMENT_DRIVER, &work, &[]);
    let out = achievement_triplet(&templates, &states[0], &states[1], &states[2], false).unwrap();
    for (name, ours) in [("completed", &out.completed), ("grey", &out.grey), ("not_eligible", &out.not_eligible)] {
        let theirs = read_rgba(&work.join(format!("{name}.py.rgba")), 64, 64);
        let d = diff(ours, &theirs, |_| true);
        eprintln!("achievement {name}: exact pixels {}/{} max error {:?}", d.exact, d.compared, d.max);
        assert_eq!(ours.data, theirs.data, "{name}: composite differs from Pillow's alpha_composite");
    }
}

// ---------------------------------------------------------------- advisor

const ADVISOR_DRIVER: &str = r#"
import importlib.util, json, sys
from pathlib import Path
kit = Path(sys.argv[1]); work = Path(sys.argv[2])
tools = kit / '.claude/skills/hoi4-feature-assets/tools'
spec = importlib.util.spec_from_file_location('adv', tools / 'create_advisor_icon.py')
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
from PIL import Image
template = m.load_template(kit / '.claude/skills/hoi4-feature-assets/assets/vanilla_reference/portraits/advisors/advisor_template.png')
geom = m.measure_opening_geometry(template)
(work / 'geometry.json').write_text(json.dumps({'center': geom['center'], 'size': geom['size'], 'rotation': geom['rotation']}))
for name in sys.argv[3:]:
    w, h = map(int, name.split('x'))
    src = Image.frombytes('RGBA', (w, h), (work / (name + '.src.rgba')).read_bytes())
    card = m.compose(src, template, geom['center'], geom['size'], (0.0, 0.0), geom['rotation'], 0.18)
    (work / (name + '.py.rgba')).write_bytes(card.tobytes())
"#;

#[test]
fn advisor_card_matches_the_kit_python() {
    let Some(env) = env("advisor parity") else { return };
    let tpath = env.kit.join(".claude/skills/hoi4-feature-assets/assets/vanilla_reference/portraits/advisors/advisor_template.png");
    if !tpath.is_file() {
        skip("advisor parity", "the kit's advisor_template.png is missing");
        return;
    }
    let (work, _guard) = workdir("advisor");
    let (template, _) = decode(&std::fs::read(&tpath).unwrap(), Some("advisor_template.png"), DecodeLimits::default()).unwrap();
    let sizes = [(156u32, 210u32), (300, 200), (200, 300), (65, 67)];
    let mut names = Vec::new();
    let mut sources = Vec::new();
    for (i, (w, h)) in sizes.iter().enumerate() {
        let img = scene(*w, *h, 50 + i as u32, false);
        let name = format!("{w}x{h}");
        std::fs::write(work.join(format!("{name}.src.rgba")), &img.data).unwrap();
        names.push(name);
        sources.push(img);
    }
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    run_python(&env, ADVISOR_DRIVER, &work, &name_refs);

    // geometry first: it drives everything
    let geom: Value = serde_json::from_slice(&std::fs::read(work.join("geometry.json")).unwrap()).unwrap();
    let ours_geom = measure_opening_geometry(&template).unwrap();
    let f = |v: &Value| v.as_f64().unwrap();
    eprintln!(
        "advisor geometry: python centre ({:.4}, {:.4}) size ({:.4}, {:.4}) rotation {:.3}; rust centre ({:.4}, {:.4}) size ({:.4}, {:.4}) rotation {:.3}",
        f(&geom["center"][0]),
        f(&geom["center"][1]),
        f(&geom["size"][0]),
        f(&geom["size"][1]),
        f(&geom["rotation"]),
        ours_geom.center.0,
        ours_geom.center.1,
        ours_geom.size.0,
        ours_geom.size.1,
        ours_geom.rotation
    );
    assert!((f(&geom["rotation"]) - ours_geom.rotation).abs() < 1e-6, "opening rotation");
    assert!((f(&geom["size"][0]) - ours_geom.size.0).abs() < 1e-6 && (f(&geom["size"][1]) - ours_geom.size.1).abs() < 1e-6, "opening size");
    assert!((f(&geom["center"][0]) - ours_geom.center.0).abs() < 1e-6 && (f(&geom["center"][1]) - ours_geom.center.1).abs() < 1e-6, "opening centre");

    for (name, src) in names.iter().zip(&sources) {
        let ours = advisor_card(src, &template, &AdvisorParams::default()).unwrap();
        assert_eq!((ours.card.width, ours.card.height), (65, 67));
        assert!(ours.coverage.is_clean());
        let theirs = read_rgba(&work.join(format!("{name}.py.rgba")), 65, 67);
        bbox_close(&ours.card, &theirs, 0);
        let d = diff(&ours.card, &theirs, |_| true);
        eprintln!(
            "advisor {name}: MAE R {:.3} G {:.3} B {:.3} A {:.3}, max {:?}, exact pixels {}/{}",
            d.mae[0], d.mae[1], d.mae[2], d.mae[3], d.max, d.exact, d.compared
        );
        assert!(d.mae.iter().all(|m| *m <= ADVISOR_MAE_MAX), "{name}: MAE {:?}", d.mae);
        assert!(d.max.iter().all(|m| *m <= ADVISOR_ERR_MAX), "{name}: max error {:?}", d.max);
    }
}

// ---------------------------------------------------------------- portrait

const PORTRAIT_DRIVER: &str = r#"
import importlib.util, io, json, sys
from pathlib import Path
kit = Path(sys.argv[1]); work = Path(sys.argv[2])
tools = kit / '.claude/skills/hoi4-feature-assets/tools'
spec = importlib.util.spec_from_file_location('por', tools / 'extract_portrait_source_crop.py')
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
from PIL import Image
cases = json.loads((work / 'cases.json').read_text())
out = []
for c in cases:
    w, h = c['size']; face = tuple(c['face'])
    try:
        crop = list(m.compute_portrait_crop((w, h), face))
    except ValueError as e:
        out.append({'name': c['name'], 'error': str(e)}); continue
    src = Image.frombytes('RGBA', (w, h), (work / (c['name'] + '.src.rgba')).read_bytes())
    piece = src.crop(tuple(crop))
    png = m.encode_processed_portrait(piece)
    im = Image.open(io.BytesIO(png)).convert('RGBA')
    (work / (c['name'] + '.py.rgba')).write_bytes(im.tobytes())
    out.append({'name': c['name'], 'crop': crop})
(work / 'results.json').write_text(json.dumps(out))
"#;

#[test]
fn portrait_crop_and_resize_match_the_kit_python() {
    let Some(env) = env("portrait parity") else { return };
    if !env.kit.join(".claude/skills/hoi4-feature-assets/tools/extract_portrait_source_crop.py").is_file() {
        skip("portrait parity", "extract_portrait_source_crop.py is missing");
        return;
    }
    let (work, _guard) = workdir("portrait");
    let cases = [
        ("centre", (800u32, 900u32), (300i64, 200i64, 80i64, 100i64)),
        ("corner", (800, 900), (0, 0, 80, 100)),
        ("wide_face", (900, 700), (350, 150, 120, 110)),
        ("tight", (400, 300), (150, 60, 100, 120)),
        ("nofit", (100, 100), (0, 0, 100, 100)),
        ("small", (156, 210), (50, 40, 50, 60)),
    ];
    let mut manifest = Vec::new();
    let mut sources = HashMap::new();
    for (i, (name, (w, h), face)) in cases.iter().enumerate() {
        let img = scene(*w, *h, 70 + i as u32, false);
        std::fs::write(work.join(format!("{name}.src.rgba")), &img.data).unwrap();
        manifest.push(json!({"name": name, "size": [w, h], "face": [face.0, face.1, face.2, face.3]}));
        sources.insert(*name, img);
    }
    std::fs::write(work.join("cases.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
    run_python(&env, PORTRAIT_DRIVER, &work, &[]);
    let results: Vec<Value> = serde_json::from_slice(&std::fs::read(work.join("results.json")).unwrap()).unwrap();
    for ((name, (w, h), face), res) in cases.iter().zip(&results) {
        let ours = compute_portrait_crop((*w, *h), *face);
        match (&ours, res.get("crop")) {
            (Ok(crop), Some(py)) => {
                let py: Vec<u32> = py.as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect();
                assert_eq!(crop.to_vec(), py, "{name}: crop rectangle");
                let src = &sources[name];
                let piece = src.crop(crop[0], crop[1], crop[2] - crop[0], crop[3] - crop[1]).unwrap();
                let portrait = portrait_from_crop(&piece).unwrap();
                let theirs = read_rgba(&work.join(format!("{name}.py.rgba")), 156, 210);
                let d = diff(&portrait, &theirs, |_| true);
                eprintln!("portrait {name}: crop {crop:?} equal; MAE R {:.3} G {:.3} B {:.3}, max {:?}", d.mae[0], d.mae[1], d.mae[2], &d.max[..3]);
                assert!(rgb_mae(&d) <= PORTRAIT_MAE_MAX, "{name}: MAE {:?}", d.mae);
                assert!(d.max[..3].iter().all(|m| *m <= PORTRAIT_ERR_MAX), "{name}: max {:?}", d.max);
                assert_eq!(d.max[3], 0, "{name}: portrait must be opaque");
            }
            (Err(_), None) => eprintln!("portrait {name}: both refuse the crop (fail closed)"),
            (a, b) => panic!("{name}: rust {a:?} vs python {b:?} disagree about whether a crop exists"),
        }
    }
}
