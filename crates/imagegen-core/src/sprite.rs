//! `.gfx` sprite snippets, the focus-icon shine companion and text-level registration.
//!
//! Registration here is deliberately plain text surgery: it inserts new `spriteType` entries before the
//! closing brace of the last `spriteTypes` block and touches nothing else, so comments, indentation and line
//! endings survive. A lossless parser with spans (the shared Rust HOI4 engine, work package W4) will replace
//! the brace scan later. Handoff mode, which only writes the snippet into `gfx_handoff.md`, stays the default
//! because the kit's rule is that asset workers do not edit `.gfx` files.

use crate::error::{Error, Result};

/// One sprite to register.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpriteEntry {
    /// Sprite name, for example `GFX_goal_iron_wall`.
    pub name: String,
    /// Complete `spriteType = { ... }` text, tab-indented one level, ending in a newline.
    pub text: String,
    /// Honest note about how much is known (empty for plain sprites).
    pub caveat: String,
}

/// Plain icon sprite.
pub fn plain_sprite(name: &str, texture_path: &str) -> SpriteEntry {
    SpriteEntry {
        name: name.to_string(),
        text: format!("\tspriteType = {{\n\t\tname = \"{name}\"\n\t\ttexturefile = \"{texture_path}\"\n\t}}\n"),
        caveat: String::new(),
    }
}

/// Sprite of a frame strip: `noOfFrames` tells the engine how many equal frames the texture holds.
pub fn framed_sprite(name: &str, texture_path: &str, frames: u32) -> SpriteEntry {
    SpriteEntry {
        name: name.to_string(),
        text: format!("	spriteType = {{
		name = \"{name}\"
		texturefile = \"{texture_path}\"
		noOfFrames = {frames}
	}}
"),
        caveat: String::new(),
    }
}

/// The `_shine` companion every vanilla focus icon has (3,122 shine sprites against 3,131 goal sprites in
/// 1.19.3.0). The pattern is copied from what the survey saw; whether a mod focus without it logs an error
/// has not been tested in game.
pub fn shine_sprite(icon_name: &str, texture_path: &str) -> SpriteEntry {
    let name = format!("{icon_name}_shine");
    let anim = |rotation: &str| {
        format!(
            "\t\tanimation = {{\n\t\t\tanimationmaskfile = \"{texture_path}\"\n\t\t\tanimationtexturefile = \"gfx/interface/goals/shine_overlay.dds\"\n\t\t\tanimationrotation = {rotation}\n\t\t\tanimationlooping = no\n\t\t\tanimationtime = 0.75\n\t\t\tanimationdelay = 0\n\t\t\tanimationblendmode = \"add\"\n\t\t\tanimationtype = \"scrolling\"\n\t\t\tanimationtexturescale = {{ x = 1.0 y = 1.0 }}\n\t\t}}\n"
        )
    };
    let text = format!(
        "\tspriteType = {{\n\t\tname = \"{name}\"\n\t\ttexturefile = \"{texture_path}\"\n\t\teffectFile = \"gfx/FX/buttonstate.lua\"\n{}{}\t\tlegacy_lazy_load = no\n\t}}\n",
        anim("-90.0"),
        anim("90.0"),
    );
    SpriteEntry {
        name,
        text,
        caveat: "Shine pattern observed in vanilla 1.19.3.0 headers and .gfx text; game behaviour without it is untested.".into(),
    }
}

/// A complete new `.gfx` file.
pub fn new_file(entries: &[SpriteEntry]) -> String {
    let mut out = String::from("spriteTypes = {\n");
    for e in entries {
        out.push_str(&e.text);
    }
    out.push_str("}\n");
    out
}

/// True when `text` already defines a sprite called `name` (case-insensitive, whitespace tolerant).
pub fn sprite_defined(text: &str, name: &str) -> bool {
    let hay = text.to_ascii_lowercase();
    let want = name.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = hay[from..].find("name") {
        let start = from + i;
        let before_ok = start == 0 || !hay.as_bytes()[start - 1].is_ascii_alphanumeric() && hay.as_bytes()[start - 1] != b'_';
        let mut j = start + 4;
        let bytes = hay.as_bytes();
        while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
            j += 1;
        }
        if before_ok && j < bytes.len() && bytes[j] == b'=' {
            j += 1;
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'"' && hay[j + 1..].starts_with(&want) && hay.as_bytes().get(j + 1 + want.len()) == Some(&b'"') {
                return true;
            }
        }
        from = start + 4;
    }
    false
}

/// Index of the closing brace of the last `spriteTypes = { ... }` block, skipping comments and strings.
fn last_sprite_block_close(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let lower = text.to_ascii_lowercase();
    let mut depth = 0i32;
    let mut i = 0;
    let mut in_block_depth: Option<i32> = None;
    let mut last_close = None;
    while i < bytes.len() {
        match bytes[i] {
            b'#' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += 1;
                }
            }
            b'{' => {
                if depth == 0 && in_block_depth.is_none() {
                    // is this the brace of `spriteTypes =`?
                    let head = lower[..i].trim_end();
                    let head = head.strip_suffix('=').map(str::trim_end).unwrap_or("");
                    if head.ends_with("spritetypes") {
                        in_block_depth = Some(depth);
                    }
                }
                depth += 1;
            }
            b'}' => {
                depth -= 1;
                if in_block_depth == Some(depth) {
                    last_close = Some(i);
                    in_block_depth = None;
                }
            }
            _ => {}
        }
        i += 1;
    }
    last_close
}

/// Insert entries into an existing `.gfx` text, or build a new file when there is none.
/// Refuses to add a sprite whose name is already defined.
pub fn register(existing: Option<&str>, entries: &[SpriteEntry]) -> Result<String> {
    let Some(text) = existing else {
        return Ok(new_file(entries));
    };
    for e in entries {
        if sprite_defined(text, &e.name) {
            return Err(Error::Exists(format!("sprite {} is already defined in the target .gfx file", e.name)));
        }
    }
    let close = last_sprite_block_close(text)
        .ok_or_else(|| Error::Invalid("the target .gfx file has no spriteTypes block to add to".into()))?;
    let crlf = text.contains("\r\n");
    let mut insert = String::new();
    // make sure the new entries start on their own line
    let before = &text[..close];
    if !before.ends_with('\n') {
        insert.push('\n');
    }
    for e in entries {
        insert.push_str(&e.text);
    }
    if crlf {
        insert = insert.replace("\r\n", "\n").replace('\n', "\r\n");
    }
    let mut out = String::with_capacity(text.len() + insert.len());
    out.push_str(before);
    out.push_str(&insert);
    out.push_str(&text[close..]);
    Ok(out)
}

/// Markdown block for `gfx_handoff.md`.
pub fn handoff_block(asset_id: &str, final_paths: &[String], gfx_file: &str, entries: &[SpriteEntry], notes: &[String]) -> String {
    let mut s = format!("## {asset_id}\n\n");
    s.push_str("Final file(s):\n\n");
    for p in final_paths {
        s.push_str(&format!("- `{p}`\n"));
    }
    s.push_str(&format!("\nSuggested target `.gfx` file: `{gfx_file}`\n\n```\nspriteTypes = {{\n"));
    for e in entries {
        s.push_str(&e.text);
    }
    s.push_str("}\n```\n");
    let mut caveats: Vec<&str> = entries.iter().map(|e| e.caveat.as_str()).filter(|c| !c.is_empty()).collect();
    caveats.dedup();
    for c in caveats {
        s.push_str(&format!("\nNote: {c}\n"));
    }
    for n in notes {
        s.push_str(&format!("\nNote: {n}\n"));
    }
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEX: &str = "gfx/interface/goals/iron/goal_wall.dds";

    #[test]
    fn plain_and_shine_snippets_have_the_documented_shape() {
        let p = plain_sprite("GFX_goal_wall", TEX);
        assert!(p.text.contains("name = \"GFX_goal_wall\"") && p.text.contains(&format!("texturefile = \"{TEX}\"")));
        let s = shine_sprite("GFX_goal_wall", TEX);
        assert_eq!(s.name, "GFX_goal_wall_shine");
        assert_eq!(s.text.matches("animation = {").count(), 2);
        assert!(s.text.contains("effectFile = \"gfx/FX/buttonstate.lua\""));
        assert!(s.text.contains("animationrotation = -90.0") && s.text.contains("animationrotation = 90.0"));
        assert!(s.text.contains("animationtime = 0.75") && s.text.contains("animationblendmode = \"add\""));
        assert!(s.text.contains("legacy_lazy_load = no"));
        assert!(s.caveat.contains("untested"));
        assert_eq!(s.text.matches('{').count(), s.text.matches('}').count());
    }

    #[test]
    fn frame_strips_register_with_no_of_frames() {
        let f = framed_sprite("GFX_unit_x_icon", "gfx/c.dds", 2);
        assert!(f.text.contains("noOfFrames = 2") && f.text.contains("texturefile = \"gfx/c.dds\""));
        assert_eq!(f.text.matches('{').count(), f.text.matches('}').count());
    }

    #[test]
    fn new_file_wraps_entries() {
        let f = new_file(&[plain_sprite("GFX_a", "gfx/a.dds")]);
        assert!(f.starts_with("spriteTypes = {\n") && f.ends_with("}\n"));
    }

    #[test]
    fn registration_preserves_everything_else() {
        let existing = "# my sprites\nspriteTypes = {\n\t# keep me\n\tspriteType = {\n\t\tname = \"GFX_old\"\n\t\ttexturefile = \"gfx/old.dds\"\n\t}\n}\n\n# trailing comment { with brace\n";
        let out = register(Some(existing), &[plain_sprite("GFX_new", "gfx/new.dds")]).unwrap();
        assert!(out.starts_with("# my sprites\nspriteTypes = {\n\t# keep me\n"));
        assert!(out.contains("name = \"GFX_old\"") && out.contains("name = \"GFX_new\""));
        assert!(out.ends_with("\n}\n\n# trailing comment { with brace\n"));
        // removing the inserted text restores the original exactly
        let inserted = plain_sprite("GFX_new", "gfx/new.dds").text;
        assert_eq!(out.replacen(&inserted, "", 1), existing);
    }

    #[test]
    fn duplicates_missing_blocks_and_crlf() {
        let existing = "spriteTypes = {\r\n\tspriteType = {\r\n\t\tname = \"GFX_Old\"\r\n\t}\r\n}\r\n";
        let err = register(Some(existing), &[plain_sprite("gfx_old", "x")]).unwrap_err();
        assert!(matches!(err, Error::Exists(_)), "case-insensitive duplicate: {err}");
        let out = register(Some(existing), &[plain_sprite("GFX_new", "x")]).unwrap();
        assert!(!out.replace("\r\n", "").contains('\n'), "line endings stay CRLF");
        assert!(register(Some("# nothing here\n"), &[plain_sprite("GFX_n", "x")]).is_err());
        assert!(register(Some("other = { a = b }"), &[plain_sprite("GFX_n", "x")]).is_err());
        assert_eq!(register(None, &[plain_sprite("GFX_n", "x")]).unwrap(), new_file(&[plain_sprite("GFX_n", "x")]));
    }

    #[test]
    fn braces_in_strings_and_comments_do_not_confuse_the_scan() {
        let existing = "spriteTypes = {\n\tspriteType = {\n\t\tname = \"GFX_a}\"\n\t\t# }\n\t}\n}\n";
        let out = register(Some(existing), &[plain_sprite("GFX_b", "x")]).unwrap();
        assert!(out.matches("GFX_b").count() == 1);
        assert!(out.trim_end().ends_with('}'));
        let a_pos = out.find("GFX_a}").unwrap();
        let b_pos = out.find("GFX_b").unwrap();
        assert!(a_pos < b_pos);
    }

    #[test]
    fn handoff_mentions_caveats() {
        let e = vec![plain_sprite("GFX_a", "gfx/a.dds"), shine_sprite("GFX_a", "gfx/a.dds")];
        let h = handoff_block("f.a", &["gfx/a.dds".into()], "interface/f_goals.gfx", &e, &["check it".into()]);
        assert!(h.contains("## f.a") && h.contains("interface/f_goals.gfx") && h.contains("untested") && h.contains("check it"));
    }
}
