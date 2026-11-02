//! `.gfx` sprite snippets, the focus-icon shine companion and registration into a `.gfx` file.
//!
//! Registration parses the target with the shared lossless HOI4 parser (`hoi4-syntax`) and inserts the new
//! `spriteType` entries before the closing brace of the last top-level `spriteTypes` block at the right byte
//! offset. Everything else keeps its bytes (comments, indentation, line endings, byte order mark, Windows-1252
//! text), and the result is parsed again to prove it. Handoff mode, which only writes the snippet into
//! `gfx_handoff.md`, stays the default because the kit's rule is that asset workers do not edit `.gfx` files.

use std::collections::HashSet;

use hoi4_syntax::{Block, Document, Encoding, Item};

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
        text: format!(
            "	spriteType = {{
		name = \"{name}\"
		texturefile = \"{texture_path}\"
		noOfFrames = {frames}
	}}
"
        ),
        caveat: String::new(),
    }
}

/// The `_shine` companion every vanilla focus icon has (3,122 shine sprites against 3,131 goal sprites in
/// 1.19.3.0). The block is the recipe's vanilla-exact one ([`crate::recipes::focus_shine::focus_shine_block`],
/// which a test compares token for token with the installed game's `goals_shine.gfx`), including the
/// `animationrotationoffset` field an earlier version of this function left out. Whether a mod focus without
/// the companion logs an error has not been tested in game.
pub fn shine_sprite(icon_name: &str, texture_path: &str) -> Result<SpriteEntry> {
    let text = crate::recipes::focus_shine::focus_shine_block(icon_name, texture_path)?;
    Ok(SpriteEntry {
        name: format!("{icon_name}_shine"),
        text,
        caveat: "Shine block copied field for field from vanilla 1.19.4.1 goals_shine.gfx; required: a focus without it shows a ? in game (checked on 1.19.4.1, 2026-11-02).".into(),
    })
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

/// The names of every `name = ...` entry anywhere in `doc`, lower-cased.
///
/// This is deliberately wider than "sprites in a `spriteTypes` block": a duplicate hiding in a block the game
/// would still load (a `spriteTypes` inside another block, a `name` in a nested block) must not slip through.
fn all_names(doc: &Document) -> HashSet<String> {
    fn walk(doc: &Document, block: &Block, out: &mut HashSet<String>) {
        for item in &block.items {
            match item {
                Item::Entry(e) => {
                    if let Some(t) = e.value.as_scalar() {
                        if doc.key(e).eq_ignore_ascii_case("name") {
                            out.insert(doc.text(t).trim().to_ascii_lowercase());
                        }
                    } else if let Some(b) = e.value.as_block() {
                        walk(doc, b, out);
                    }
                }
                Item::Value(v) => {
                    if let Some(b) = v.as_block() {
                        walk(doc, b, out);
                    }
                }
                Item::Stray(_) => {}
            }
        }
    }
    let mut out = HashSet::new();
    walk(doc, &doc.root, &mut out);
    out
}

/// The last top-level `spriteTypes = { ... }` block (key compared without case), with its closing brace token.
fn last_sprite_block(doc: &Document) -> Option<(&Block, u32)> {
    let block = doc.root.entries().filter(|e| doc.key(e).eq_ignore_ascii_case("spriteTypes")).filter_map(|e| e.value.as_block()).last()?;
    Some((block, block.close?))
}

fn all_blank(bytes: &[u8]) -> bool {
    bytes.iter().all(|b| *b == b' ' || *b == b'\t')
}

/// Start offset of the line holding `offset`.
fn line_start(bytes: &[u8], offset: usize) -> usize {
    bytes[..offset].iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1)
}

/// The file's line ending: CRLF when at least half the lines end that way, else LF.
fn line_ending(bytes: &[u8]) -> &'static str {
    let lf = bytes.iter().filter(|b| **b == b'\n').count();
    let crlf = bytes.windows(2).filter(|w| *w == b"\r\n").count();
    if crlf > 0 && crlf * 2 >= lf {
        "\r\n"
    } else {
        "\n"
    }
}

/// Insert entries into an existing `.gfx` file, or build a new file when there is none.
///
/// The file is parsed with the lossless HOI4 parser (`hoi4-syntax`). The entries go before the closing brace of
/// the last top-level `spriteTypes` block, at that block's own indentation and in the file's line ending, and
/// every other byte of the file stays as it was (comments, BOM, encoding, other blocks). The result is parsed
/// again and must round-trip through the tree, keep the old syntax diagnostics and add none, and contain every
/// new sprite name. A name that is already defined anywhere in the file (without case) is refused, as is
/// non-ASCII text going into a Windows-1252 file.
pub fn register_bytes(existing: Option<&[u8]>, entries: &[SpriteEntry]) -> Result<Vec<u8>> {
    let mut wanted: HashSet<String> = HashSet::new();
    for e in entries {
        if !wanted.insert(e.name.to_ascii_lowercase()) {
            return Err(Error::Exists(format!("sprite {} appears twice in this install", e.name)));
        }
    }
    let Some(bytes) = existing else {
        return Ok(new_file(entries).into_bytes());
    };
    let doc = Document::parse(bytes.to_vec());
    let present = all_names(&doc);
    for e in entries {
        if present.contains(&e.name.to_ascii_lowercase()) {
            return Err(Error::Exists(format!("sprite {} is already defined in the target .gfx file", e.name)));
        }
    }
    let (block, close_token) =
        last_sprite_block(&doc).ok_or_else(|| Error::Invalid("the target .gfx file has no complete spriteTypes block to add to".into()))?;
    if entries.is_empty() {
        return Ok(bytes.to_vec());
    }
    let close = doc.token(close_token).start as usize;

    // where the closing brace sits: alone on its line (insert whole lines before it) or after other text
    let brace_line = line_start(bytes, close);
    let brace_indent: Vec<u8> = bytes[brace_line..close].iter().copied().take_while(|b| *b == b' ' || *b == b'\t').collect();
    let alone = all_blank(&bytes[brace_line..close]);
    // one indentation step: what the last child uses beyond the brace's own indentation (a tab when unknown)
    let mut unit: Vec<u8> = b"\t".to_vec();
    if let Some(child) = block.entries().last() {
        let key_at = doc.token(child.key).start as usize;
        let ls = line_start(bytes, key_at);
        let ws = &bytes[ls..key_at];
        if all_blank(ws) && ws.len() > brace_indent.len() && ws.starts_with(&brace_indent) {
            unit = ws[brace_indent.len()..].to_vec();
        }
    }
    let eol = line_ending(bytes);

    // the rendered text is tab-indented from depth 1: re-indent it to the block's style and line ending
    let mut insert: Vec<u8> = Vec::new();
    for e in entries {
        for line in e.text.split_inclusive('\n') {
            let body = line.trim_end_matches(['\r', '\n']);
            let tabs = body.bytes().take_while(|b| *b == b'\t').count();
            let rest = &body[tabs..];
            if !rest.is_empty() {
                insert.extend_from_slice(&brace_indent);
                for _ in 0..tabs {
                    insert.extend_from_slice(&unit);
                }
                insert.extend_from_slice(rest.as_bytes());
            }
            insert.extend_from_slice(eol.as_bytes());
        }
    }
    if doc.encoding == Encoding::Windows1252 && !insert.is_ascii() {
        return Err(Error::Invalid(
            "the target .gfx file is Windows-1252 and the new sprite text has non-ASCII characters; use ASCII names and paths, or convert the file to UTF-8 yourself".into(),
        ));
    }
    let (at, tail): (usize, Vec<u8>) = if alone {
        (brace_line, Vec::new())
    } else {
        // the brace follows other text on its line: start a new line for the entries, then put the brace on its own line
        let mut lead = eol.as_bytes().to_vec();
        lead.extend_from_slice(&insert);
        insert = lead;
        let mut tail = Vec::new();
        tail.extend_from_slice(&brace_indent);
        (close, tail)
    };
    let mut out = Vec::with_capacity(bytes.len() + insert.len() + tail.len());
    out.extend_from_slice(&bytes[..at]);
    out.extend_from_slice(&insert);
    out.extend_from_slice(&tail);
    out.extend_from_slice(&bytes[at..]);

    // prove the result: untouched bytes, lossless tree, no new syntax problems, every name present
    let after = Document::parse(out.clone());
    if after.write_tree() != out {
        return Err(Error::Invalid("internal: the registered .gfx file does not round-trip through the parser; nothing was written".into()));
    }
    if after.encoding != doc.encoding || after.diagnostics.len() != doc.diagnostics.len() {
        return Err(Error::Invalid("internal: registering changed the file's encoding or syntax diagnostics; nothing was written".into()));
    }
    let names = all_names(&after);
    if let Some(missing) = entries.iter().find(|e| !names.contains(&e.name.to_ascii_lowercase())) {
        return Err(Error::Invalid(format!("internal: {} is not in the registered .gfx file; nothing was written", missing.name)));
    }
    Ok(out)
}

/// Text form of [`register_bytes`] for UTF-8 callers.
pub fn register(existing: Option<&str>, entries: &[SpriteEntry]) -> Result<String> {
    let out = register_bytes(existing.map(str::as_bytes), entries)?;
    String::from_utf8(out).map_err(|_| Error::Invalid("internal: registered text is not UTF-8".into()))
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
        let s = shine_sprite("GFX_goal_wall", TEX).unwrap();
        assert_eq!(s.name, "GFX_goal_wall_shine");
        assert_eq!(s.text.matches("animation = {").count(), 2);
        assert!(s.text.contains("effectFile = \"gfx/FX/buttonstate.lua\""));
        assert!(s.text.contains("animationrotation = -90.0") && s.text.contains("animationrotation = 90.0"));
        assert!(s.text.contains("animationtime = 0.75") && s.text.contains("animationblendmode = \"add\""));
        assert!(s.text.contains("legacy_lazy_load = no"));
        assert!(s.text.contains("animationrotationoffset = { x = 0.0 y = 0.0 }"));
        assert!(s.caveat.contains("required"));
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

    /// Every byte of `orig` is still in `out`, in order, around one contiguous insertion.
    fn only_inserted(orig: &[u8], out: &[u8]) -> Vec<u8> {
        assert!(out.len() > orig.len());
        let prefix = orig.iter().zip(out).take_while(|(a, b)| a == b).count();
        let suffix = orig.iter().rev().zip(out.iter().rev()).take_while(|(a, b)| a == b).count();
        assert!(prefix + suffix >= orig.len(), "the original is not a prefix plus a suffix of the result");
        out[prefix..out.len() - (orig.len() - prefix)].to_vec()
    }

    fn reg(existing: &[u8], entries: &[SpriteEntry]) -> Vec<u8> {
        register_bytes(Some(existing), entries).unwrap()
    }

    fn lossless(bytes: &[u8]) {
        assert_eq!(Document::parse(bytes.to_vec()).write_tree(), bytes);
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
        lossless(out.as_bytes());
    }

    #[test]
    fn crlf_files_stay_crlf_and_lf_files_stay_lf() {
        let crlf = b"spriteTypes = {\r\n\tspriteType = {\r\n\t\tname = \"GFX_Old\"\r\n\t}\r\n}\r\n".to_vec();
        let out = reg(&crlf, &[plain_sprite("GFX_new", "x")]);
        assert!(!String::from_utf8_lossy(&out).replace("\r\n", "").contains('\n'), "line endings stay CRLF");
        let ins = only_inserted(&crlf, &out);
        assert_eq!(ins, plain_sprite("GFX_new", "x").text.replace('\n', "\r\n").into_bytes());
        lossless(&out);

        let lf = b"spriteTypes = {\n\tspriteType = {\n\t\tname = \"GFX_Old\"\n\t}\n}\n".to_vec();
        let out = reg(&lf, &[plain_sprite("GFX_new", "x")]);
        assert!(!out.contains(&b'\r'));
        assert_eq!(only_inserted(&lf, &out), plain_sprite("GFX_new", "x").text.into_bytes());
    }

    #[test]
    fn a_utf8_bom_is_kept() {
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend_from_slice(b"spriteTypes = {\r\n\tspriteType = { name = \"GFX_a\" texturefile = \"a.dds\" }\r\n}\r\n");
        let out = reg(&bom, &[plain_sprite("GFX_b", "b.dds")]);
        assert!(out.starts_with(&[0xEF, 0xBB, 0xBF]));
        assert_eq!(Document::parse(out.clone()).encoding, Encoding::Utf8Bom);
        only_inserted(&bom, &out);
        lossless(&out);
    }

    #[test]
    fn windows_1252_stays_windows_1252_and_refuses_non_ascii_names() {
        // "caf\xE9" and "r\xE9sum\xE9" are single bytes in cp1252 and not valid UTF-8
        let mut cp = b"# caf\xE9 sprites { r\xE9sum\xE9\r\nspriteTypes = {\r\n\t# \xE0 la carte }\r\n\tspriteType = {\r\n\t\tname = \"GFX_old\"\r\n\t}\r\n}\r\n".to_vec();
        assert_eq!(Document::parse(cp.clone()).encoding, Encoding::Windows1252);
        let out = reg(&cp, &[plain_sprite("GFX_new", "gfx/new.dds")]);
        assert_eq!(Document::parse(out.clone()).encoding, Encoding::Windows1252);
        only_inserted(&cp, &out);
        assert!(out.starts_with(b"# caf\xE9 sprites"), "the accented bytes are untouched");
        lossless(&out);
        let err = register_bytes(Some(&cp), &[plain_sprite("GFX_caf\u{e9}", "gfx/new.dds")]).unwrap_err();
        assert!(matches!(err, Error::Invalid(_)) && err.to_string().contains("Windows-1252"), "{err}");
        // a UTF-8 file takes the same name
        cp = b"spriteTypes = {\n}\n".to_vec();
        let out = reg(&cp, &[plain_sprite("GFX_caf\u{e9}", "gfx/new.dds")]);
        assert!(String::from_utf8(out).unwrap().contains("GFX_caf\u{e9}"));
    }

    #[test]
    fn braces_in_strings_and_comments_do_not_confuse_the_parser() {
        let existing = "spriteTypes = {\n\tspriteType = {\n\t\tname = \"GFX_a}\"\n\t\t# }\n\t}\n}\n";
        let out = register(Some(existing), &[plain_sprite("GFX_b", "x")]).unwrap();
        assert_eq!(out.matches("GFX_b").count(), 1);
        assert!(out.trim_end().ends_with('}'));
        assert!(out.find("GFX_a}").unwrap() < out.find("GFX_b").unwrap());
        // braces in comments outside the block, before and after
        let existing = "# { { {\nspriteTypes = {\n\tspriteType = { name = \"GFX_a\" texturefile = \"a.dds\" } # }}}\n}\n# }\n";
        let out = reg(existing.as_bytes(), &[plain_sprite("GFX_b", "b.dds")]);
        let ins = String::from_utf8(only_inserted(existing.as_bytes(), &out)).unwrap();
        assert_eq!(ins, plain_sprite("GFX_b", "b.dds").text);
        assert!(out.ends_with(b"}\n# }\n"));
    }

    #[test]
    fn nested_blocks_do_not_end_the_search() {
        let existing = "spriteTypes = {\n\tspriteType = {\n\t\tname = \"GFX_a\"\n\t\tanimation = {\n\t\t\tanimationtype = scrolling\n\t\t\toffset = { x = 0 y = 0 }\n\t\t}\n\t}\n}\n";
        let out = reg(existing.as_bytes(), &[plain_sprite("GFX_b", "b.dds")]);
        let doc = Document::parse(out.clone());
        assert!(doc.diagnostics.is_empty());
        let list = doc.root.entries().next().unwrap().value.as_block().unwrap();
        assert_eq!(list.entries().count(), 2, "the new sprite is a child of spriteTypes, not of the animation");
    }

    #[test]
    fn the_last_of_several_sprite_type_blocks_gets_the_entries() {
        let existing = "spriteTypes = {\n\tspriteType = { name = \"GFX_a\" texturefile = \"a.dds\" }\n}\nSPRITETYPES = {\n\tspriteType = { name = \"GFX_b\" texturefile = \"b.dds\" }\n}\nother = { x = 1 }\n";
        let out = reg(existing.as_bytes(), &[plain_sprite("GFX_c", "c.dds")]);
        let text = String::from_utf8(out.clone()).unwrap();
        let first_end = text.find("}\nSPRITETYPES").unwrap();
        assert!(!text[..first_end].contains("GFX_c"));
        assert!(text.find("GFX_c").unwrap() > text.find("GFX_b").unwrap());
        assert!(text.find("GFX_c").unwrap() < text.find("other = ").unwrap());
        only_inserted(existing.as_bytes(), &out);
        lossless(&out);
    }

    #[test]
    fn duplicates_are_refused_across_the_whole_file_without_case() {
        let existing = "spriteTypes = {\n\tspriteType = { name = \"GFX_Old\" texturefile = \"a.dds\" }\n}\nspriteTypes = {\n\tspriteType = { name = \"GFX_other\" texturefile = \"b.dds\" }\n}\n";
        // in the first block, while the last block is the target
        let err = register_bytes(Some(existing.as_bytes()), &[plain_sprite("gfx_old", "x")]).unwrap_err();
        assert!(matches!(err, Error::Exists(_)), "case-insensitive duplicate: {err}");
        let err = register_bytes(Some(existing.as_bytes()), &[plain_sprite("GFX_OTHER", "x")]).unwrap_err();
        assert!(matches!(err, Error::Exists(_)));
        // two new entries with one name
        let err = register_bytes(Some(existing.as_bytes()), &[plain_sprite("GFX_n", "x"), plain_sprite("gfx_N", "y")]).unwrap_err();
        assert!(matches!(err, Error::Exists(_)));
        // a name the file mentions only in a comment is not a definition
        let commented = "spriteTypes = {\n\t# name = \"GFX_n\"\n}\n";
        assert!(register_bytes(Some(commented.as_bytes()), &[plain_sprite("GFX_n", "x")]).is_ok());
    }

    #[test]
    fn missing_or_unfinished_sprite_blocks_are_refused() {
        assert!(register(Some("# nothing here\n"), &[plain_sprite("GFX_n", "x")]).is_err());
        assert!(register(Some("other = { a = b }"), &[plain_sprite("GFX_n", "x")]).is_err());
        assert!(register(Some(""), &[plain_sprite("GFX_n", "x")]).is_err());
        assert!(register(Some("spriteTypes = {\n\tspriteType = { name = \"GFX_a\"\n"), &[plain_sprite("GFX_n", "x")]).is_err());
        assert_eq!(register(None, &[plain_sprite("GFX_n", "x")]).unwrap(), new_file(&[plain_sprite("GFX_n", "x")]));
    }

    #[test]
    fn the_indentation_of_the_block_is_matched() {
        let spaces = "spriteTypes = {\n    spriteType = {\n        name = \"GFX_a\"\n        texturefile = \"a.dds\"\n    }\n}\n";
        let out = String::from_utf8(reg(spaces.as_bytes(), &[plain_sprite("GFX_b", "b.dds")])).unwrap();
        assert!(out.contains("    spriteType = {\n        name = \"GFX_b\"\n        texturefile = \"b.dds\"\n    }\n}\n"), "{out}");
        // a block that is itself indented
        let deep = "\tspriteTypes = {\n\t\tspriteType = { name = \"GFX_a\" texturefile = \"a.dds\" }\n\t}\n";
        let out = String::from_utf8(reg(deep.as_bytes(), &[plain_sprite("GFX_b", "b.dds")])).unwrap();
        assert!(out.contains("\t\tspriteType = {\n\t\t\tname = \"GFX_b\"\n"), "{out}");
    }

    #[test]
    fn one_line_and_empty_blocks_get_their_own_lines() {
        for existing in ["spriteTypes = { }\n", "spriteTypes = {}", "spriteTypes = {\n}\n", "spriteTypes = { spriteType = { name = \"GFX_a\" } }\n"] {
            let out = reg(existing.as_bytes(), &[plain_sprite("GFX_n", "gfx/n.dds")]);
            let doc = Document::parse(out.clone());
            assert!(doc.diagnostics.is_empty(), "{existing:?}");
            assert!(
                String::from_utf8_lossy(&out).contains("\tspriteType = {\n\t\tname = \"GFX_n\"\n"),
                "{existing:?} -> {}",
                String::from_utf8_lossy(&out)
            );
            lossless(&out);
        }
    }

    #[test]
    fn frame_strips_and_the_focus_shine_block_register() {
        let existing =
            b"spriteTypes = {\r\n\tspriteType = {\r\n\t\tname = \"GFX_goal_x\"\r\n\t\ttexturefile = \"gfx/interface/goals/x.dds\"\r\n\t}\r\n}\r\n"
                .to_vec();
        let strip = framed_sprite("GFX_unit_x_icon", "gfx/c.dds", 2);
        let shine = shine_sprite("GFX_goal_x", "gfx/interface/goals/x.dds").unwrap();
        let out = reg(&existing, &[strip.clone(), shine.clone()]);
        let text = String::from_utf8(out.clone()).unwrap();
        assert!(text.contains("noOfFrames = 2"));
        assert!(text.contains("name = \"GFX_goal_x_shine\"") && text.contains("animationrotationoffset = { x = 0.0 y = 0.0 }"));
        assert!(!text.replace("\r\n", "").contains('\n'));
        lossless(&out);
        // the parsed tree has the new blocks as children of spriteTypes
        let doc = Document::parse(out.clone());
        let list = doc.root.entries().next().unwrap().value.as_block().unwrap();
        assert_eq!(list.entries().count(), 3);
        let frame = list
            .entries()
            .find(|e| doc.key(e) == "spriteType" && e.value.as_block().is_some_and(|b| b.entries().any(|x| doc.key(x) == "noOfFrames")))
            .unwrap();
        let block = frame.value.as_block().unwrap();
        assert!(block.entries().any(|e| doc.key(e) == "noOfFrames" && doc.text(e.value.as_scalar().unwrap()) == "2"));
        // a frameAnimatedSpriteType block with noOfFrames, as vanilla writes animated strips
        let animated = SpriteEntry {
            name: "GFX_anim".into(),
            text: "	frameAnimatedSpriteType = {
		name = \"GFX_anim\"
		texturefile = \"gfx/anim.dds\"
		noOfFrames = 8
		animation_rotation_offset = { x = 0 y = 0 }
	}
"
            .into(),
            caveat: String::new(),
        };
        let out = reg(&existing, &[animated]);
        lossless(&out);
        let doc = Document::parse(out);
        assert!(doc.diagnostics.is_empty());
        let list = doc.root.entries().next().unwrap().value.as_block().unwrap();
        let anim = list.entries().find(|e| doc.key(e) == "frameAnimatedSpriteType").unwrap().value.as_block().unwrap();
        assert!(anim.entries().any(|e| doc.key(e) == "noOfFrames" && doc.text(e.value.as_scalar().unwrap()) == "8"));
        // and the vanilla-exact recipe block registers the same way
        let recipe = crate::recipes::focus_shine::focus_shine_block("GFX_goal_y", "gfx/interface/goals/y.dds").unwrap();
        let entry = SpriteEntry { name: "GFX_goal_y_shine".into(), text: recipe, caveat: String::new() };
        let out = reg(&existing, &[entry]);
        lossless(&out);
        assert!(Document::parse(out).diagnostics.is_empty());
    }

    #[test]
    fn every_untouched_byte_survives_a_registration() {
        // stress: odd spacing, tabs after the key, comments everywhere, a file without a final newline
        let existing = "#\tbanner\r\nspriteTypes\t=\t{ # open\r\n\r\n  spriteType={name=\"GFX_a\" texturefile=\"a.dds\"}#c\r\n\r\n}# closed";
        let out = reg(existing.as_bytes(), &[plain_sprite("GFX_b", "b.dds")]);
        let inserted = only_inserted(existing.as_bytes(), &out);
        assert!(String::from_utf8(inserted).unwrap().contains("GFX_b"));
        lossless(&out);
    }
    #[test]
    fn handoff_mentions_caveats() {
        let e = vec![plain_sprite("GFX_a", "gfx/a.dds"), shine_sprite("GFX_a", "gfx/a.dds").unwrap()];
        let h = handoff_block("f.a", &["gfx/a.dds".into()], "interface/f_goals.gfx", &e, &["check it".into()]);
        assert!(h.contains("## f.a") && h.contains("interface/f_goals.gfx") && h.contains("required") && h.contains("check it"));
    }
}
