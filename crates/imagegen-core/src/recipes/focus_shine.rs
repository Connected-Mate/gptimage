//! The `_shine` companion registration for a focus icon.
//!
//! This is text, not pixels: every vanilla focus icon has a second `SpriteType` named `<icon>_shine` that reuses
//! the icon's texture as an animation mask. The block below is the structure vanilla `interface/goals_shine.gfx`
//! uses (HOI4 1.19.4.1): `effectFile = "gfx/FX/buttonstate.lua"` and two scrolling animations over
//! `gfx/interface/goals/shine_overlay.dds`, one at rotation -90 and one at 90, then `legacy_lazy_load = no`.
//!
//! [`crate::sprite::shine_sprite`] writes a close variant that leaves out `animationrotationoffset`; this one
//! reproduces the vanilla block field for field, which a test confirms against the installed game when it is present.

use crate::error::{Error, Result};

/// Overlay texture every vanilla shine animation uses.
pub const SHINE_OVERLAY: &str = "gfx/interface/goals/shine_overlay.dds";
/// Effect file every vanilla shine sprite uses.
pub const SHINE_EFFECT_FILE: &str = "gfx/FX/buttonstate.lua";

fn check_token(label: &str, value: &str, allow_path: bool) -> Result<()> {
    let ok = !value.is_empty() && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || (allow_path && (c == '/' || c == '.')));
    if !ok || value.contains("..") {
        return Err(Error::Invalid(format!("{label} {value:?} contains characters that do not belong in a .gfx name or path")));
    }
    Ok(())
}

fn animation(texture_path: &str, rotation: &str) -> String {
    let mut s = String::new();
    s.push_str("\t\tanimation = {\n");
    s.push_str(&format!("\t\t\tanimationmaskfile = \"{texture_path}\"\n"));
    s.push_str(&format!("\t\t\tanimationtexturefile = \"{SHINE_OVERLAY}\"\n"));
    s.push_str(&format!("\t\t\tanimationrotation = {rotation}\n"));
    s.push_str("\t\t\tanimationlooping = no\n");
    s.push_str("\t\t\tanimationtime = 0.75\n");
    s.push_str("\t\t\tanimationdelay = 0\n");
    s.push_str("\t\t\tanimationblendmode = \"add\"\n");
    s.push_str("\t\t\tanimationtype = \"scrolling\"\n");
    s.push_str("\t\t\tanimationrotationoffset = { x = 0.0 y = 0.0 }\n");
    s.push_str("\t\t\tanimationtexturescale = { x = 1.0 y = 1.0 }\n");
    s.push_str("\t\t}\n");
    s
}

/// The `SpriteType` block for `<icon_name>_shine`, one tab indented, ending in a newline.
///
/// `icon_name` is the plain sprite's name (for example `GFX_goal_iron_wall`) and `texture_path` the icon texture
/// (`gfx/interface/goals/goal_iron_wall.dds`).
pub fn focus_shine_block(icon_name: &str, texture_path: &str) -> Result<String> {
    check_token("icon name", icon_name, false)?;
    check_token("texture path", texture_path, true)?;
    if icon_name.ends_with("_shine") {
        return Err(Error::Invalid(format!("{icon_name:?} is already a shine sprite name; pass the plain icon name")));
    }
    let mut s = String::new();
    s.push_str("\tSpriteType = {\n");
    s.push_str(&format!("\t\tname = \"{icon_name}_shine\"\n"));
    s.push_str(&format!("\t\ttexturefile = \"{texture_path}\"\n"));
    s.push_str(&format!("\t\teffectFile = \"{SHINE_EFFECT_FILE}\"\n"));
    s.push_str(&animation(texture_path, "-90.0"));
    s.push_str(&animation(texture_path, "90.0"));
    s.push_str("\t\tlegacy_lazy_load = no\n");
    s.push_str("\t}\n");
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_has_the_vanilla_structure() {
        let b = focus_shine_block("GFX_goal_iron_wall", "gfx/interface/goals/goal_iron_wall.dds").unwrap();
        assert!(b.starts_with("\tSpriteType = {\n\t\tname = \"GFX_goal_iron_wall_shine\"\n"));
        assert!(b.contains("\t\ttexturefile = \"gfx/interface/goals/goal_iron_wall.dds\"\n\t\teffectFile = \"gfx/FX/buttonstate.lua\"\n"));
        assert_eq!(b.matches("animation = {").count(), 2);
        assert_eq!(b.matches("animationmaskfile = \"gfx/interface/goals/goal_iron_wall.dds\"").count(), 2);
        assert_eq!(b.matches("animationtexturefile = \"gfx/interface/goals/shine_overlay.dds\"").count(), 2);
        let first = b.find("animationrotation = -90.0").unwrap();
        let second = b.find("animationrotation = 90.0").unwrap();
        assert!(first < second);
        assert!(b.ends_with("\t\tlegacy_lazy_load = no\n\t}\n"));
        assert_eq!(b.matches('{').count(), b.matches('}').count());
    }

    #[test]
    fn hostile_names_are_refused() {
        assert!(focus_shine_block("GFX_x\" } evil = {", "gfx/a.dds").is_err());
        assert!(focus_shine_block("GFX_x", "gfx/../../etc/passwd").is_err());
        assert!(focus_shine_block("GFX_x", "gfx/a b.dds").is_err());
        assert!(focus_shine_block("GFX_x_shine", "gfx/a.dds").is_err());
        assert!(focus_shine_block("", "gfx/a.dds").is_err());
    }

    fn tokens(text: &str) -> Vec<String> {
        let mut out = Vec::new();
        for line in text.lines() {
            let code = line.split('#').next().unwrap_or("");
            let spaced = code.replace('{', " { ").replace('}', " } ").replace('=', " = ");
            out.extend(spaced.split_whitespace().map(str::to_string));
        }
        out
    }

    #[test]
    fn matches_the_installed_game_block_token_for_token() {
        let game = std::env::var("HOI4_GAME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| "C:/Program Files (x86)/Steam/steamapps/common/Hearts of Iron IV".into());
        let Ok(text) = std::fs::read_to_string(game.join("interface/goals_shine.gfx")) else {
            eprintln!("SKIPPED focus_shine vs game: interface/goals_shine.gfx not found under {}", game.display());
            return;
        };
        let start = text.find("name = \"GFX_goal_continuous_air_production_shine\"").expect("vanilla sprite present");
        let begin = text[..start].rfind("SpriteType").expect("block start");
        let mut depth = 0i32;
        let mut end = begin;
        for (i, ch) in text[begin..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = begin + i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        let vanilla = tokens(&text[begin..end]);
        let ours =
            tokens(&focus_shine_block("GFX_goal_continuous_air_production", "gfx/interface/goals/goal_continuous_air_production.dds").unwrap());
        assert_eq!(ours, vanilla, "our block differs from vanilla's");
    }
}
