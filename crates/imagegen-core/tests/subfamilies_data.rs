//! `data/subfamilies.json` regenerates identically from the installed game.
//!
//! The data holds file names, groups and measured numbers only, never image data. With `UPDATE_SUBFAMILIES=1` the test
//! writes the file instead of comparing. Without the game it prints `SKIPPED` and passes (set `HOI4_GAME_DIR`).

use std::path::{Path, PathBuf};

use imagegen_core::subfamily;

fn find_game() -> Option<PathBuf> {
    let candidates = [
        std::env::var("HOI4_GAME_DIR").ok().map(PathBuf::from),
        Some(PathBuf::from("C:/Program Files (x86)/Steam/steamapps/common/Hearts of Iron IV")),
        Some(PathBuf::from("C:/Program Files/Steam/steamapps/common/Hearts of Iron IV")),
        std::env::var("HOME").ok().map(|h| PathBuf::from(h).join("Library/Application Support/Steam/steamapps/common/Hearts of Iron IV")),
    ];
    candidates.into_iter().flatten().find(|p| p.join("interface/subuniticons.gfx").is_file())
}

#[test]
fn subfamily_data_regenerates_identically_from_the_game() {
    let Some(game) = find_game() else {
        eprintln!("SKIPPED: the installed game was not found");
        return;
    };
    let data = subfamily::build(&game);
    let text = serde_json::to_string_pretty(&data).unwrap() + "\n";
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/subfamilies.json");
    if std::env::var("UPDATE_SUBFAMILIES").is_ok() {
        std::fs::write(&path, &text).unwrap();
        eprintln!("wrote {}", path.display());
        return;
    }
    let shipped = std::fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
    if data.game_version != subfamily::data().game_version {
        eprintln!("SKIPPED: the data was measured on {:?}, the game is {:?}", subfamily::data().game_version, data.game_version);
        return;
    }
    assert!(shipped == text, "data/subfamilies.json is stale: regenerate it with UPDATE_SUBFAMILIES=1");
    // the families the maintainer named are split when their pools are large enough
    for f in ["focus", "idea_spirit", "decision", "technology"] {
        assert!(data.families.iter().any(|x| x.family.as_str() == f), "{f} has no sub-families");
    }
}
