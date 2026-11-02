//! The sprite names this tool registers, checked against the installed game's own `.gfx` files.
//!
//! For every family whose name the game builds (a short key with a prefix, a database key, a symbol number) or
//! looks up next to the main sprite (a `_miniature`, a `_large`, a `_small`, a text icon), a vanilla example is
//! run through the tool's own naming and every resulting name must be a vanilla sprite with the same case, the
//! same `noOfFrames` and, where the family's size is fixed, the same frame size. The game is read, never written,
//! and never started. Without the game every test prints `SKIPPED` and passes (set `HOI4_GAME_DIR` to point at it).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use hoi4_syntax::{Block, Document, Item};
use imagegen_core::dds;
use imagegen_core::export::{final_files, sprite_entries};
use imagegen_core::family::Family;
use imagegen_core::lookup::{div_template_numbers, lookup_of, next_div_template_number, LookupKind, VANILLA_DIV_TEMPLATE_MAX};
use imagegen_core::profile::ProfileSet;
use imagegen_core::recipes::derived;

fn find_game() -> Option<PathBuf> {
    let candidates = [
        std::env::var("HOI4_GAME_DIR").ok().map(PathBuf::from),
        Some(PathBuf::from("C:/Program Files (x86)/Steam/steamapps/common/Hearts of Iron IV")),
        Some(PathBuf::from("C:/Program Files/Steam/steamapps/common/Hearts of Iron IV")),
        std::env::var("HOME").ok().map(|h| PathBuf::from(h).join("Library/Application Support/Steam/steamapps/common/Hearts of Iron IV")),
    ];
    candidates.into_iter().flatten().find(|p| p.join("interface/subuniticons.gfx").is_file())
}

/// One vanilla sprite definition.
#[derive(Debug, Clone)]
struct Sprite {
    texture: String,
    frames: u32,
    file: String,
}

fn unquote(s: &str) -> String {
    s.trim().trim_matches('"').replace('\\', "/").replace("//", "/")
}

/// Every `*spriteType` block of a parsed `.gfx` file, at any depth.
fn collect(doc: &Document, block: &Block, file: &str, out: &mut HashMap<String, Sprite>) {
    for item in &block.items {
        let Item::Entry(e) = item else { continue };
        let Some(b) = e.value.as_block() else { continue };
        if doc.key(e).to_ascii_lowercase().ends_with("spritetype") {
            let mut name = None;
            let mut texture = String::new();
            let mut frames = 1;
            for f in b.entries() {
                let Some(v) = f.value.as_scalar() else { continue };
                let v = unquote(&doc.text(v));
                match doc.key(f).to_ascii_lowercase().as_str() {
                    "name" => name = Some(v),
                    "texturefile" => texture = v,
                    "noofframes" => frames = v.parse().unwrap_or(1),
                    _ => {}
                }
            }
            if let Some(n) = name {
                out.entry(n).or_insert(Sprite { texture, frames, file: file.to_string() });
            }
        } else {
            collect(doc, b, file, out);
        }
    }
}

fn vanilla_sprites(game: &Path) -> HashMap<String, Sprite> {
    let mut out = HashMap::new();
    let mut stack = vec![game.join("interface")];
    while let Some(dir) = stack.pop() {
        for ent in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = ent.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|x| x.eq_ignore_ascii_case("gfx")) {
                let bytes = std::fs::read(&path).unwrap();
                let doc = Document::parse(bytes);
                let rel = path.strip_prefix(game).unwrap().display().to_string().replace('\\', "/");
                collect(&doc, &doc.root, &rel, &mut out);
            }
        }
    }
    out
}

/// Width and height of a vanilla texture, from its DDS header.
fn texture_size(game: &Path, texture: &str) -> Option<[u32; 2]> {
    let bytes = std::fs::read(game.join(texture)).ok()?;
    let info = dds::parse_header(&bytes).ok()?;
    Some([info.width, info.height])
}

/// Families whose vanilla example has the family's fixed frame size (the idea and decision examples are free-sized
/// or belong to a sibling family, the modifier icons vary).
const SIZE_CHECKED: &[Family] = &[
    Family::CounterLarge,
    Family::CounterSmall,
    Family::DivisionEmblem,
    Family::Raid,
    Family::OperationIcon,
    Family::OperationPicture,
    Family::Medal,
    Family::Building,
    Family::FactionLogo,
    Family::CommanderTrait,
    Family::PersonalTrait,
    Family::OfficerCorpsSpirit,
    Family::DecisionCategoryPicture,
];

#[test]
fn every_built_or_companion_name_is_a_vanilla_sprite_with_the_same_frames() {
    let Some(game) = find_game() else {
        eprintln!("SKIPPED vanilla lookups: the game is not installed (set HOI4_GAME_DIR); UNAVAILABLE in this run");
        return;
    };
    let sprites = vanilla_sprites(&game);
    assert!(sprites.len() > 20_000, "only {} vanilla sprites were read", sprites.len());
    let set = ProfileSet::builtin();
    let mut checked = 0;
    for f in Family::ALL.iter().copied() {
        let lookup = lookup_of(f);
        let Some(example) = lookup.vanilla_example else { continue };
        let prof = set.for_family(f).unwrap();
        let (asset, number) = if lookup.kind == LookupKind::Numbered { ("emblem", Some(example.parse::<u32>().unwrap())) } else { (example, None) };
        let files = final_files(prof, prof.canvas, "vanilla", asset).unwrap();
        let (_, entries) = sprite_entries(prof, "vanilla", asset, &files, number).unwrap().unwrap();
        for (i, e) in entries.iter().enumerate() {
            if e.name.ends_with("_shine") {
                continue;
            }
            let v = sprites.get(&e.name).unwrap_or_else(|| {
                let ci = sprites.keys().find(|k| k.eq_ignore_ascii_case(&e.name));
                panic!("{f}: {} is not a vanilla sprite (same name in another case: {ci:?}); lookup evidence: {}", e.name, lookup.evidence)
            });
            let ours = e.text.split("noOfFrames = ").nth(1).and_then(|t| t.split_whitespace().next()).map_or(1, |n| n.parse::<u32>().unwrap());
            // vanilla medals are 4- and 7-frame strips picked with frame = N; the tool makes one frame and script uses frame = 1
            if f != Family::Medal {
                assert_eq!(ours, v.frames, "{f}: {} has noOfFrames {ours}, vanilla {} ({})", e.name, v.frames, v.file);
            }
            if SIZE_CHECKED.contains(&f) {
                let size = texture_size(&game, &v.texture).unwrap_or_else(|| panic!("{f}: cannot read {}", v.texture));
                let frame = [size[0] / v.frames.max(1), size[1]];
                let want = [files[i].canvas[0] / ours, files[i].canvas[1]];
                assert_eq!(
                    frame, want,
                    "{f}: vanilla {} ({}) has {}x{} frames, the tool makes {}x{}",
                    e.name, v.texture, frame[0], frame[1], want[0], want[1]
                );
            }
            println!("{f}: {} matches {} ({} frames)", e.name, v.file, v.frames);
            checked += 1;
        }
    }
    assert!(checked >= 20, "only {checked} names were checked");
}

#[test]
fn the_counter_icons_are_named_by_the_sub_unit_key_not_its_sprite() {
    let Some(game) = find_game() else {
        eprintln!("SKIPPED sub-unit key: the game is not installed; UNAVAILABLE in this run");
        return;
    };
    let unit = std::fs::read_to_string(game.join("common/units/engineer.txt")).unwrap();
    assert!(unit.contains("engineer = {") && unit.contains("sprite = infantry"), "the engineer sub-unit's sprite key is infantry");
    let sprites = vanilla_sprites(&game);
    assert!(sprites.contains_key("GFX_unit_engineer_icon_medium"), "yet its battalion icon is named after the sub-unit key");
    assert!(sprites.contains_key("GFX_unit_infantry_icon_medium_white") && sprites.contains_key("GFX_unit_infantry_icon_small"));
    // the old names this tool used for the on-map counter and the battalion icon are something else in vanilla
    let small = &sprites["GFX_unit_infantry_icon_small"];
    assert!(small.texture.starts_with("gfx/texticons/"), "GFX_unit_*_icon_small is the text icon: {}", small.texture);
    assert!(!sprites.contains_key("GFX_unit_infantry_icon"), "vanilla has no GFX_unit_<key>_icon sprite");
}

#[test]
fn division_template_numbers_follow_vanilla_and_a_new_one_is_free() {
    let Some(game) = find_game() else {
        eprintln!("SKIPPED division template numbers: the game is not installed; UNAVAILABLE in this run");
        return;
    };
    let text = std::fs::read(game.join("interface/subuniticons.gfx")).unwrap();
    let numbers = div_template_numbers(&String::from_utf8_lossy(&text));
    let want: Vec<u32> = (0..=43).chain(65..=121).collect();
    assert_eq!(numbers, want, "vanilla 1.19.4.1 symbols are 0-43 and 65-121");
    assert_eq!(numbers.iter().copied().max(), Some(VANILLA_DIV_TEMPLATE_MAX));
    let sprites = vanilla_sprites(&game);
    for n in &numbers {
        assert!(sprites.contains_key(&format!("GFX_div_templ_{n}_large")) && sprites.contains_key(&format!("GFX_div_templ_{n}_small")), "{n}");
    }
    let next = next_div_template_number(VANILLA_DIV_TEMPLATE_MAX, numbers.iter().copied());
    assert_eq!(next, 122);
    assert!(!sprites.contains_key(&format!("GFX_div_templ_{next}_large")));
}

/// Premultiplied mean absolute error over RGBA, 0..255.
fn mae(a: &imagegen_core::Rgba, b: &imagegen_core::Rgba) -> f64 {
    let mut sum = 0.0;
    for (p, q) in a.data.chunks_exact(4).zip(b.data.chunks_exact(4)) {
        let (pa, qa) = (f64::from(p[3]) / 255.0, f64::from(q[3]) / 255.0);
        for c in 0..3 {
            sum += (f64::from(p[c]) * pa - f64::from(q[c]) * qa).abs();
        }
        sum += (f64::from(p[3]) - f64::from(q[3])).abs();
    }
    sum / (a.data.len() as f64)
}

#[test]
fn the_derived_small_emblem_is_close_to_vanilla_and_grey_like_it() {
    let Some(game) = find_game() else {
        eprintln!("SKIPPED small emblem golden: the game is not installed; UNAVAILABLE in this run");
        return;
    };
    let mut ours = Vec::new();
    let mut naive = Vec::new();
    for n in (0..=43).chain(65..=121) {
        let read = |dir: &str| {
            std::fs::read(game.join(format!("gfx/interface/counters/{dir}/custom_template_{n:03}.dds"))).ok().and_then(|b| dds::decode(&b).ok())
        };
        let (Some(large), Some(small)) = (read("division_templates_large"), read("division_templates_small")) else { continue };
        if (small.width, small.height) != (30, 12) {
            continue;
        }
        let made = derived::small_emblem(&large).unwrap();
        ours.push(mae(&made, &small));
        // the obvious alternative: the whole 76x42 canvas squeezed to 30x12
        let squeezed = imagegen_core::resize::resize_lanczos3(&large, 30, 12).unwrap();
        naive.push(mae(&squeezed, &small));
    }
    if ours.len() < 50 {
        eprintln!("SKIPPED small emblem golden: only {} vanilla pairs decode as uncompressed DDS", ours.len());
        return;
    }
    ours.sort_by(f64::total_cmp);
    naive.sort_by(f64::total_cmp);
    let (m_ours, m_naive) = (ours[ours.len() / 2], naive[naive.len() / 2]);
    println!("small emblem: {} pairs, median MAE derived {m_ours:.2}, whole-canvas squeeze {m_naive:.2}", ours.len());
    assert!(m_ours < m_naive, "the derivation must be closer to vanilla than a plain squeeze");
    assert!(m_ours < 20.0, "median MAE {m_ours:.2}");
}
