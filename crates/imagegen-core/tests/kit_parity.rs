//! Parity with the Agentic HOI4 Modding Kit's Python tools, on synthetic images.
//!
//! Checks, against the kit's own code run in a throwaway process:
//!  1. `convert_to_dds.py write_bgra_dds` writes byte-identical files to `dds::write_a8r8g8b8`;
//!  2. the kit's `strict_dds` rule (canonical one-level BGRA header and exact length) accepts our file and
//!     decodes the same pixels;
//!  3. `gfx.py alpha_stats` and `size_check` (margin and undersized-subject rules) agree with `frame_stats`
//!     and `validate_image`.
//!
//! `gfx.py` imports scipy, which is not always installed, so its pure-numpy functions are extracted with `ast`
//! and executed without importing the module. If Python, Pillow, numpy or the kit checkout is missing the test
//! prints `SKIPPED` with the reason and passes vacuously; the report must then call the parity check
//! unavailable. Set `HOI4_KIT_DIR` and `HOI4_IMAGEGEN_PYTHON` to point at a kit checkout and an interpreter.
//! All temporary files live under `<repo>/.work/`.

use std::path::{Path, PathBuf};
use std::process::Command;

use imagegen_core::dds;
use imagegen_core::family::Family;
use imagegen_core::pixels::Rgba;
use imagegen_core::profile::ProfileSet;
use imagegen_core::validate::{frame_stats, validate_image, Options};
use serde_json::{json, Value};

const SCRIPT: &str = r#"
import ast, importlib.util, json, sys
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence, Tuple, Union
import numpy as np
from PIL import Image

kit = Path(sys.argv[1]); work = Path(sys.argv[2])
manifest = json.loads((work / 'manifest.json').read_text())

spec = importlib.util.spec_from_file_location('dds_converter', kit / '.claude/skills/hoi4-feature-assets/tools/convert_to_dds.py')
conv = importlib.util.module_from_spec(spec); spec.loader.exec_module(conv)

tree = ast.parse((kit / '.tools/gfx/gfx.py').read_text(encoding='utf-8'))
ns = {'np': np, 'Image': Image, 'Any': Any, 'Dict': Dict, 'List': List, 'Optional': Optional,
      'Sequence': Sequence, 'Tuple': Tuple, 'Union': Union}
data = json.loads((kit / '.tools/gfx/profiles.json').read_text(encoding='utf-8'))['profiles']
ns['PROFILE_DATA'] = data
ns['PROFILES'] = {n: (int(p['canvas'][0]), int(p['canvas'][1]), int(p['margin'])) for n, p in data.items()}
for node in tree.body:
    if isinstance(node, ast.FunctionDef) and node.name in ('frames', 'alpha_stats', 'size_check'):
        exec(compile(ast.Module([node], []), 'gfx.py', 'exec'), ns)

def strict_dds(raw):
    # same rule as gfx.py strict_dds
    import struct
    if len(raw) < 128: return False
    magic, *words = struct.unpack('<4s31I', raw[:128])
    h, w = words[2:4]
    expected = [124, 0x100f, h, w, w * 4, 0, 0, *([0] * 11), 32, 65, 0, 32, 0xff0000, 0xff00, 0xff, 0xff000000, 0x1000, 0, 0, 0, 0]
    if magic != b'DDS ' or words != expected or not w or not h: return False
    return len(raw) == 128 + w * h * 4

results = []
for item in manifest:
    w, h, name = item['w'], item['h'], item['name']
    raw = (work / (name + '.rgba')).read_bytes()
    img = Image.frombytes('RGBA', (w, h), raw)
    out = work / (name + '.py.dds')
    conv.write_bgra_dds(out, w, h, img.tobytes('raw', 'BGRA'))
    rust_bytes = (work / (name + '.rust.dds')).read_bytes()
    entry = {'name': name, 'strict_ok': strict_dds(rust_bytes)}
    if entry['strict_ok']:
        decoded = Image.frombytes('RGBA', (w, h), rust_bytes[128:], 'raw', 'BGRA')
        entry['strict_pixels_equal'] = decoded.tobytes() == raw
    try:
        entry['pillow_open_equal'] = Image.open(work / (name + '.rust.dds')).convert('RGBA').tobytes() == raw
    except Exception as e:
        entry['pillow_open_equal'] = 'error: ' + type(e).__name__
    entry['alpha'] = ns['alpha_stats'](img)
    if item.get('profile'):
        entry['size'] = ns['size_check'](img, item['profile'])
    results.append(entry)
(work / 'results.json').write_text(json.dumps(results))
"#;

struct Rng(u32);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
}

fn noise(w: u32, h: u32, seed: u32) -> Rgba {
    let mut rng = Rng(seed);
    let mut img = Rgba::new(w, h);
    for p in img.data.chunks_exact_mut(4) {
        let v = rng.next();
        p.copy_from_slice(&v.to_le_bytes());
    }
    img
}

fn blob(w: u32, h: u32, x0: u32, y0: u32, x1: u32, y1: u32, alpha: u8) -> Rgba {
    let mut img = Rgba::new(w, h);
    for y in y0..y1 {
        for x in x0..x1 {
            img.set_px(x, y, [(x * 5 % 256) as u8, (y * 7 % 256) as u8, 90, alpha]);
        }
    }
    img
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn find_kit() -> Option<PathBuf> {
    let candidates = [std::env::var("HOI4_KIT_DIR").ok().map(PathBuf::from), Some(repo_root().join("..").join("agentic-hoi4-modding"))];
    candidates
        .into_iter()
        .flatten()
        .find(|p| p.join(".claude/skills/hoi4-feature-assets/tools/convert_to_dds.py").is_file() && p.join(".tools/gfx/gfx.py").is_file())
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
            .args(["-c", "import sys, numpy, PIL; assert sys.version_info >= (3, 9)"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
}

#[test]
fn dds_writer_and_pixel_rules_match_the_kits_python() {
    let Some(kit) = find_kit() else {
        eprintln!("SKIPPED kit_parity: no kit checkout found (set HOI4_KIT_DIR); parity is UNAVAILABLE in this run");
        return;
    };
    let Some(python) = find_python() else {
        eprintln!("SKIPPED kit_parity: no Python >= 3.9 with numpy and Pillow found (set HOI4_IMAGEGEN_PYTHON); parity is UNAVAILABLE in this run");
        return;
    };

    // Synthetic corpus. The first group is DDS-only; the second group is 52x40 and also goes through size_check.
    let mut corpus: Vec<(String, Rgba, Option<&str>)> = vec![
        ("one_by_one".into(), Rgba::filled(1, 1, [1, 2, 3, 4]), None),
        ("noise_3x2".into(), noise(3, 2, 0x1234_5678), None),
        ("noise_17x13".into(), noise(17, 13, 0x0bad_cafe), None),
        ("noise_100x88".into(), noise(100, 88, 0xdead_beef), None),
        ("noise_65x67".into(), noise(65, 67, 0x0102_0304), None),
        ("opaque_210x176".into(), Rgba::filled(210, 176, [10, 120, 200, 255]), None),
        ("centred".into(), blob(52, 40, 4, 3, 48, 37, 255), Some("category")),
        ("touching_left".into(), blob(52, 40, 0, 3, 46, 37, 255), Some("category")),
        ("tiny".into(), blob(52, 40, 20, 15, 30, 25, 255), Some("category")),
        ("empty".into(), Rgba::new(52, 40), Some("category")),
        ("fully_opaque".into(), Rgba::filled(52, 40, [9, 9, 9, 255]), Some("category")),
        ("translucent".into(), blob(52, 40, 4, 3, 48, 37, 120), Some("category")),
        ("alpha_noise".into(), noise(52, 40, 0x7777_1111), Some("category")),
    ];
    // make the noise images' alpha vary but never be all-zero
    for (_, img, _) in corpus.iter_mut() {
        if img.data.chunks_exact(4).all(|p| p[3] == 0) && img.width > 1 && img.data.iter().any(|b| *b != 0) {
            img.data[3] = 255;
        }
    }

    let work = repo_root().join(".work").join(format!("parity-{}", std::process::id()));
    std::fs::create_dir_all(&work).unwrap();
    let cleanup = Cleanup(work.clone());

    let mut manifest = Vec::new();
    for (name, img, profile) in &corpus {
        std::fs::write(work.join(format!("{name}.rgba")), &img.data).unwrap();
        std::fs::write(work.join(format!("{name}.rust.dds")), dds::write_a8r8g8b8(img)).unwrap();
        manifest.push(json!({"name": name, "w": img.width, "h": img.height, "profile": profile}));
    }
    std::fs::write(work.join("manifest.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
    std::fs::write(work.join("parity.py"), SCRIPT).unwrap();

    let out = Command::new(&python[0]).args(&python[1..]).arg(work.join("parity.py")).arg(&kit).arg(&work).output().expect("python starts");
    assert!(out.status.success(), "python parity script failed:\n{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let results: Vec<Value> = serde_json::from_slice(&std::fs::read(work.join("results.json")).unwrap()).unwrap();
    assert_eq!(results.len(), corpus.len());

    let set = ProfileSet::builtin();
    let category = set.for_family(Family::DecisionCategoryIcon).unwrap();
    let mut pillow_equal = 0;
    for ((name, img, profile), res) in corpus.iter().zip(&results) {
        assert_eq!(res["name"], *name);
        // 1. byte-identical DDS
        let ours = std::fs::read(work.join(format!("{name}.rust.dds"))).unwrap();
        let theirs = std::fs::read(work.join(format!("{name}.py.dds"))).unwrap();
        assert_eq!(ours, theirs, "{name}: Rust DDS differs from the kit's write_bgra_dds");
        // 2. strict header rule and pixel round trip
        assert_eq!(res["strict_ok"], true, "{name}: kit strict_dds rejects our header");
        assert_eq!(res["strict_pixels_equal"], true, "{name}: pixels differ after the kit's decode");
        if res["pillow_open_equal"] == true {
            pillow_equal += 1;
        }
        // 3. alpha statistics
        let stats = frame_stats(img);
        let py = &res["alpha"];
        assert_eq!(py["alpha_min"].as_u64().unwrap(), u64::from(stats.alpha_min), "{name}: alpha_min");
        assert_eq!(py["alpha_max"].as_u64().unwrap(), u64::from(stats.alpha_max), "{name}: alpha_max");
        assert_eq!(py["subject_pixels"].as_u64().unwrap(), stats.subject_pixels, "{name}: subject_pixels");
        assert_eq!(py["opaque_pixels"].as_u64().unwrap(), stats.opaque_pixels, "{name}: opaque_pixels");
        assert!((py["subject_alpha_mean"].as_f64().unwrap() - stats.subject_alpha_mean).abs() < 1e-9, "{name}: subject_alpha_mean");
        let py_bounds: Option<Vec<u64>> = py["visible_bounds"].as_array().map(|a| a.iter().map(|v| v.as_u64().unwrap()).collect());
        let my_bounds = img.alpha_bbox(1).map(|b| b.iter().map(|v| u64::from(*v)).collect::<Vec<_>>());
        assert_eq!(py_bounds, my_bounds, "{name}: visible bounds");
        assert_eq!(py["ghosted"].as_bool().unwrap(), stats.alpha_max == 0 || stats.subject_alpha_mean < 200.0, "{name}: ghosted");

        // 4. margin and undersized-subject rules
        if profile.is_some() {
            let messages: Vec<String> = res["size"].as_array().unwrap().iter().map(|m| m.as_str().unwrap().to_string()).collect();
            let report = validate_image(img, category, &["size".into(), "alpha".into(), "margin".into(), "fill".into()], &Options::default());
            let ours_margin = report.findings.iter().any(|f| f.code == "GFX-MARGIN");
            let ours_fill = report.findings.iter().any(|f| f.code == "GFX-UNDERSIZED-SUBJECT");
            let ours_empty = report.findings.iter().any(|f| f.code == "GFX-EMPTY-SUBJECT");
            let py_margin = messages.iter().any(|m| m.contains("margins L/T/R/B"));
            let py_fill = messages.iter().any(|m| m.contains("subject span"));
            let py_empty = messages.iter().any(|m| m.contains("empty subject"));
            assert_eq!(py_margin, ours_margin, "{name}: margin rule disagrees: python {messages:?} ours {:?}", report.findings);
            assert_eq!(py_fill, ours_fill, "{name}: undersized-subject rule disagrees: python {messages:?}");
            assert_eq!(py_empty, ours_empty, "{name}: empty-subject rule disagrees: python {messages:?}");
            if let (true, Some(m)) = (py_margin, stats.margins) {
                let text = messages.iter().find(|m| m.contains("margins L/T/R/B")).unwrap();
                assert!(text.contains(&format!("{:?}", [m[0], m[1], m[2], m[3]])), "{name}: margins {m:?} not in {text}");
            }
        }
    }
    eprintln!(
        "kit_parity: {} synthetic images compared with the kit's Python (kit {}, python {:?}); Pillow opened {} of {} of our DDS files",
        corpus.len(),
        kit.display(),
        python,
        pillow_equal,
        corpus.len()
    );
    drop(cleanup);
}

struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
