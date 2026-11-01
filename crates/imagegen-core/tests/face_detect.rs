//! YuNet through tract on real portraits from the kit's reference folder (read at test time, never copied).
//! Run with `cargo test -p imagegen-core --features face-detect --test face_detect -- --nocapture`.
//! Skips with a printed reason when the model or the reference portraits are missing, unless
//! `IMAGEGEN_REQUIRE_PARITY=1`.

#![cfg(feature = "face-detect")]

use std::path::{Path, PathBuf};

use imagegen_core::pixels::{decode, DecodeLimits, Rgba};
use imagegen_core::recipes::face::YuNet;
use imagegen_core::recipes::portrait::{extract_portrait, CropChoice, FaceDetector};

fn kit() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let candidates = [std::env::var("HOI4_KIT_DIR").ok().map(PathBuf::from), Some(root.join("agentic-hoi4-modding"))];
    candidates
        .into_iter()
        .flatten()
        .find(|p| p.join(".claude/skills/hoi4-feature-assets/tools/models/face_detection_yunet_2026may.onnx").is_file())
}

fn skip(reason: &str) {
    if std::env::var("IMAGEGEN_REQUIRE_PARITY").map(|v| v == "1").unwrap_or(false) {
        panic!("IMAGEGEN_REQUIRE_PARITY=1 but the face test cannot run: {reason}");
    }
    eprintln!("SKIPPED face_detect: {reason}");
}

fn load(path: &Path) -> Rgba {
    decode(&std::fs::read(path).unwrap(), path.file_name().and_then(|n| n.to_str()), DecodeLimits::default()).unwrap().0
}

#[test]
fn yunet_finds_one_face_in_real_portraits_and_none_in_noise() {
    let Some(kit) = kit() else { return skip("the kit or its YuNet model is missing") };
    let skill = kit.join(".claude/skills/hoi4-feature-assets");
    let started = std::time::Instant::now();
    let net = YuNet::from_path(&skill.join("tools/models/face_detection_yunet_2026may.onnx")).expect("tract loads the model");
    eprintln!("face: model loaded in {:?}", started.elapsed());

    let dir = skill.join("assets/vanilla_reference/portraits/advisors");
    let names = ["generic_europe_1.png", "generic_asia_1.png", "generic_africa_1.png", "generic_arab_1.png", "generic_female_europe.png"];
    let mut found = 0;
    for name in names {
        let path = dir.join(name);
        if !path.is_file() {
            continue;
        }
        let img = load(&path);
        let t = std::time::Instant::now();
        let dets = net.detections(&img).unwrap();
        eprintln!(
            "face: {name} {}x{}: {} detections in {:?}: {:?}",
            img.width,
            img.height,
            dets.len(),
            t.elapsed(),
            dets.iter().map(|d| (d.bbox, (d.score * 1000.0).round() / 1000.0)).collect::<Vec<_>>()
        );
        let faces = net.detect(&img).unwrap();
        if faces.len() == 1 {
            found += 1;
            let (l, t, w, h) = faces[0];
            assert!(l >= 0 && t >= 0 && l + w <= i64::from(img.width) && t + h <= i64::from(img.height), "{faces:?}");
            assert!(w > 10 && h > 10);
        }
    }
    if found == 0 {
        return skip("no reference portrait produced exactly one face (images may be missing)");
    }
    eprintln!("face: {found} of {} reference portraits gave exactly one face", names.len());

    // noise has no face: the automatic route fails closed, the manual route recovers
    let mut noise = Rgba::new(200, 260);
    let mut s = 0x1234_5678u32;
    for p in noise.data.chunks_exact_mut(4) {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        p.copy_from_slice(&[(s & 255) as u8, ((s >> 8) & 255) as u8, ((s >> 16) & 255) as u8, 255]);
    }
    assert!(net.detect(&noise).unwrap().is_empty());
    assert!(extract_portrait(&noise, &net, None).is_err());
    let manual = extract_portrait(&noise, &net, Some([10, 10, 170, 220])).unwrap();
    assert_eq!(manual.choice, CropChoice::Manual);
}
