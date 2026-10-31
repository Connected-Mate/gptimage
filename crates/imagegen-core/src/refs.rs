//! Reference images: loading, validation and the vanilla-art opt-in (decision D4).
//!
//! Upstream read any path, trusted the file extension as a fallback and sent whatever it found. Here a
//! reference must be a real PNG, JPEG, WebP, DDS or TGA found inside the project; its type comes from its
//! bytes, markup is refused, size and pixel caps apply, and it is re-encoded to a clean PNG so metadata and
//! trailing data never reach a provider. Paradox's own art (the installed game or the kit's vanilla
//! reference shelf) is sent only when the project opted in.

use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::PolicyConfig;
use crate::error::{Error, Result, RetryClass};
use crate::fsio::{display_path, read_limited, ProjectFs};
use crate::hashing::sha256_hex;
use crate::pixels::{decode, encode_png, DecodeLimits, SourceFormat};
use crate::resize::resize_lanczos3;

/// The part a reference plays in the request. The kit's generation rules number references by role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RefRole {
    /// Shows the target style (approved project art, or opted-in vanilla art).
    StyleAnchor,
    /// Shows the subject to depict.
    SubjectAnchor,
    /// Shows composition or silhouette to follow.
    GeometryAnchor,
    /// The image an edit applies to.
    EditTarget,
}

impl RefRole {
    /// Phrase used in prompts.
    pub fn phrase(self) -> &'static str {
        match self {
            RefRole::StyleAnchor => "style anchor: match its painting style, palette and level of detail",
            RefRole::SubjectAnchor => "subject anchor: depict this subject",
            RefRole::GeometryAnchor => "geometry anchor: follow its composition and silhouette",
            RefRole::EditTarget => "edit target: change this image as instructed and keep everything else",
        }
    }
}

/// Where a reference came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RefOrigin {
    /// A file in the project.
    Project,
    /// An earlier candidate of this tool.
    PreviousCandidate,
    /// The kit's vanilla reference shelf (Paradox-derived). Needs the D4 opt-in.
    KitShelf,
    /// The installed game. Needs the D4 opt-in.
    InstalledGame,
}

impl RefOrigin {
    /// True for Paradox's art.
    pub fn is_vanilla(self) -> bool {
        matches!(self, RefOrigin::KitShelf | RefOrigin::InstalledGame)
    }
}

/// A reference as the caller names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReferenceInput {
    /// Project-relative path (an absolute path inside the installed game folder is accepted only with the D4 opt-in).
    pub path: String,
    /// Role in the request.
    pub role: RefRole,
}

/// A reference ready to send.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LoadedRef {
    /// Display path (project-relative where possible).
    pub path: String,
    /// Role.
    pub role: RefRole,
    /// Origin.
    pub origin: RefOrigin,
    /// SHA-256 of the file as found.
    pub source_sha256: String,
    /// Container of the file as found.
    pub source_format: String,
    /// SHA-256 of the clean PNG that is sent.
    pub sha256: String,
    /// Width of the clean PNG.
    pub width: u32,
    /// Height of the clean PNG.
    pub height: u32,
    /// Things the caller should know (extension mismatch, downscaling).
    pub notes: Vec<String>,
    /// The clean PNG bytes.
    #[serde(skip)]
    pub png: Vec<u8>,
}

/// Hard limits for references.
pub const MAX_REFERENCE_BYTES: usize = 24 * 1024 * 1024;
/// Longest side kept when re-encoding.
pub const MAX_REFERENCE_SIDE: u32 = 2048;
/// Most references one request may carry.
pub const MAX_REFERENCES: usize = 8;
/// Total clean PNG bytes one request may carry.
pub const MAX_TOTAL_REFERENCE_BYTES: usize = 64 * 1024 * 1024;

const KIT_SHELF_MARKER: &str = "assets/vanilla_reference/";

/// Classify a project-relative path.
pub fn classify_project_path(rel: &str) -> RefOrigin {
    let unified = rel.replace('\\', "/").to_ascii_lowercase();
    if unified.contains(KIT_SHELF_MARKER) {
        RefOrigin::KitShelf
    } else {
        RefOrigin::Project
    }
}

/// Refuse Paradox art unless the project opted in (D4).
pub fn check_vanilla_policy(origin: RefOrigin, policy: &PolicyConfig, what: &str) -> Result<()> {
    if origin.is_vanilla() && !policy.allow_vanilla_references {
        return Err(Error::refused(
            "vanilla_reference_not_allowed",
            format!(
                "{what} is Paradox art. Sending it to a provider is a per-project opt-in (decision D4); set policy.allow_vanilla_references = true in .hoi4-imagegen/config.toml to allow it for this project"
            ),
            RetryClass::RequiresAuthorization,
        ));
    }
    Ok(())
}

/// Turn raw bytes into a clean, capped PNG reference.
pub fn clean_reference(
    bytes: &[u8],
    name_hint: &str,
    display: &str,
    role: RefRole,
    origin: RefOrigin,
) -> Result<LoadedRef> {
    let (img, format) = decode(bytes, Some(name_hint), DecodeLimits::default())
        .map_err(|e| Error::Image(format!("reference '{display}': {e}")))?;
    let mut notes = Vec::new();
    let lower = name_hint.to_ascii_lowercase();
    let expected_ext: &[&str] = match format {
        SourceFormat::Png => &["png"],
        SourceFormat::Jpeg => &["jpg", "jpeg"],
        SourceFormat::WebP => &["webp"],
        SourceFormat::Dds => &["dds"],
        SourceFormat::Tga => &["tga"],
    };
    if !expected_ext.iter().any(|e| lower.ends_with(&format!(".{e}"))) {
        notes.push(format!("file extension does not match its content, which is {}", format.name()));
    }
    let mut work = img;
    let longest = work.width.max(work.height);
    if longest > MAX_REFERENCE_SIDE {
        let scale = f64::from(MAX_REFERENCE_SIDE) / f64::from(longest);
        let nw = ((f64::from(work.width) * scale).round() as u32).max(1);
        let nh = ((f64::from(work.height) * scale).round() as u32).max(1);
        notes.push(format!("downscaled from {}x{} to {nw}x{nh}", work.width, work.height));
        work = resize_lanczos3(&work, nw, nh)?;
    }
    let png = encode_png(&work)?;
    Ok(LoadedRef {
        path: display.to_string(),
        role,
        origin,
        source_sha256: sha256_hex(bytes),
        source_format: format.name().to_string(),
        sha256: sha256_hex(&png),
        width: work.width,
        height: work.height,
        notes,
        png,
    })
}

/// Load a reference named by the caller.
///
/// `game_dir` is the installed game folder, used only to recognise (and gate) absolute paths into it.
pub fn load_reference(
    fs: &ProjectFs,
    game_dir: Option<&Path>,
    policy: &PolicyConfig,
    input: &ReferenceInput,
) -> Result<LoadedRef> {
    let raw = input.path.as_str();
    let as_path = Path::new(raw);
    if as_path.is_absolute() || raw.starts_with('/') || raw.starts_with('\\') {
        // The only absolute locations accepted: inside the project, or inside the installed game (opt-in).
        let canon = std::fs::canonicalize(as_path).map_err(|_| Error::NotFound(raw.to_string()))?;
        if canon.starts_with(fs.root()) {
            let rel = fs.rel(&canon)?;
            return load_project_relative(fs, policy, &rel, input.role);
        }
        if let Some(game) = game_dir.and_then(|g| std::fs::canonicalize(g).ok()) {
            if canon.starts_with(&game) {
                check_vanilla_policy(RefOrigin::InstalledGame, policy, &format!("'{}'", display_path(&canon)))?;
                let bytes = read_limited(&canon, MAX_REFERENCE_BYTES)?;
                let name = canon.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                return clean_reference(&bytes, &name, &display_path(&canon), input.role, RefOrigin::InstalledGame);
            }
        }
        return Err(Error::PathRejected(format!(
            "reference '{raw}' is outside the project; copy it into the project's references/ folder"
        )));
    }
    load_project_relative(fs, policy, raw, input.role)
}

fn load_project_relative(fs: &ProjectFs, policy: &PolicyConfig, rel: &str, role: RefRole) -> Result<LoadedRef> {
    let origin = classify_project_path(rel);
    check_vanilla_policy(origin, policy, &format!("'{rel}'"))?;
    let bytes = fs.read(rel, MAX_REFERENCE_BYTES)?;
    let name = rel.rsplit(['/', '\\']).next().unwrap_or(rel);
    clean_reference(&bytes, name, &rel.replace('\\', "/"), role, origin)
}

/// Check the request-wide caps.
pub fn check_set_caps(refs: &[LoadedRef]) -> Result<()> {
    if refs.len() > MAX_REFERENCES {
        return Err(Error::Invalid(format!("at most {MAX_REFERENCES} references per request, got {}", refs.len())));
    }
    let total: usize = refs.iter().map(|r| r.png.len()).sum();
    if total > MAX_TOTAL_REFERENCE_BYTES {
        return Err(Error::Invalid(format!("references total {total} bytes, the limit is {MAX_TOTAL_REFERENCE_BYTES}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pixels::Rgba;

    fn png(w: u32, h: u32) -> Vec<u8> {
        encode_png(&Rgba::filled(w, h, [10, 20, 30, 255])).unwrap()
    }

    fn setup() -> (tempfile::TempDir, ProjectFs) {
        let dir = tempfile::tempdir().unwrap();
        let fs = ProjectFs::new(dir.path()).unwrap();
        (dir, fs)
    }

    #[test]
    fn project_png_is_loaded_and_cleaned() {
        let (_d, fs) = setup();
        fs.write_new("references/style.png", &png(32, 24)).unwrap();
        let r = load_reference(
            &fs,
            None,
            &PolicyConfig::default(),
            &ReferenceInput { path: "references/style.png".into(), role: RefRole::StyleAnchor },
        )
        .unwrap();
        assert_eq!((r.width, r.height), (32, 24));
        assert_eq!(r.origin, RefOrigin::Project);
        assert_eq!(r.source_format, "png");
        assert!(r.notes.is_empty());
        assert_eq!(r.sha256, sha256_hex(&r.png));
    }

    #[test]
    fn wrong_extension_is_noted_not_trusted() {
        let (_d, fs) = setup();
        fs.write_new("references/real_png.jpg", &png(8, 8)).unwrap();
        let r = load_reference(
            &fs,
            None,
            &PolicyConfig::default(),
            &ReferenceInput { path: "references/real_png.jpg".into(), role: RefRole::SubjectAnchor },
        )
        .unwrap();
        assert_eq!(r.source_format, "png");
        assert!(r.notes.iter().any(|n| n.contains("extension")));
    }

    #[test]
    fn svg_text_and_missing_files_are_refused() {
        let (_d, fs) = setup();
        fs.write_new("references/a.png", b"<svg xmlns='http://www.w3.org/2000/svg'/>").unwrap();
        fs.write_new("references/b.png", b"not an image at all").unwrap();
        let policy = PolicyConfig::default();
        for p in ["references/a.png", "references/b.png"] {
            let err = load_reference(&fs, None, &policy, &ReferenceInput { path: p.into(), role: RefRole::StyleAnchor }).unwrap_err();
            assert!(matches!(err, Error::Image(_)), "{p}: {err}");
        }
        let err = load_reference(&fs, None, &policy, &ReferenceInput { path: "references/none.png".into(), role: RefRole::StyleAnchor }).unwrap_err();
        assert!(matches!(err, Error::NotFound(_)));
    }

    #[test]
    fn traversal_and_outside_absolute_paths_are_refused() {
        let (_d, fs) = setup();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("x.png"), png(4, 4)).unwrap();
        let policy = PolicyConfig::default();
        let abs = outside.path().join("x.png").display().to_string();
        for p in ["../x.png", abs.as_str(), "/etc/passwd", "C:\\Windows\\win.ini"] {
            let err = load_reference(&fs, None, &policy, &ReferenceInput { path: p.into(), role: RefRole::StyleAnchor }).unwrap_err();
            assert!(matches!(err, Error::PathRejected(_) | Error::NotFound(_)), "{p}: {err}");
        }
    }

    #[test]
    fn kit_shelf_art_needs_the_opt_in() {
        let (_d, fs) = setup();
        fs.write_new(".claude/skills/hoi4-feature-assets/assets/vanilla_reference/icons/national_focus/a.png", &png(16, 16)).unwrap();
        let input = ReferenceInput {
            path: ".claude/skills/hoi4-feature-assets/assets/vanilla_reference/icons/national_focus/a.png".into(),
            role: RefRole::StyleAnchor,
        };
        let err = load_reference(&fs, None, &PolicyConfig::default(), &input).unwrap_err();
        assert_eq!(err.code(), "vanilla_reference_not_allowed");
        assert_eq!(err.retry_class(), RetryClass::RequiresAuthorization);
        let allowed = PolicyConfig { allow_vanilla_references: true, ..Default::default() };
        let ok = load_reference(&fs, None, &allowed, &input).unwrap();
        assert_eq!(ok.origin, RefOrigin::KitShelf);
    }

    #[test]
    fn installed_game_art_needs_the_opt_in_and_decodes_dds() {
        let (_d, fs) = setup();
        let game = tempfile::tempdir().unwrap();
        let dds = crate::dds::write_a8r8g8b8(&Rgba::filled(100, 88, [200, 50, 50, 255]));
        let file = game.path().join("gfx/interface/goals");
        std::fs::create_dir_all(&file).unwrap();
        std::fs::write(file.join("focus_x.dds"), dds).unwrap();
        let abs = file.join("focus_x.dds").display().to_string();
        let input = ReferenceInput { path: abs, role: RefRole::StyleAnchor };
        let err = load_reference(&fs, Some(game.path()), &PolicyConfig::default(), &input).unwrap_err();
        assert_eq!(err.code(), "vanilla_reference_not_allowed");
        let allowed = PolicyConfig { allow_vanilla_references: true, ..Default::default() };
        let r = load_reference(&fs, Some(game.path()), &allowed, &input).unwrap();
        assert_eq!(r.origin, RefOrigin::InstalledGame);
        assert_eq!(r.source_format, "dds");
        assert_eq!((r.width, r.height), (100, 88));
    }

    #[test]
    fn large_references_are_downscaled_and_caps_apply() {
        let (_d, fs) = setup();
        fs.write_new("references/big.png", &png(3000, 1500)).unwrap();
        let r = load_reference(
            &fs,
            None,
            &PolicyConfig::default(),
            &ReferenceInput { path: "references/big.png".into(), role: RefRole::GeometryAnchor },
        )
        .unwrap();
        assert_eq!(r.width.max(r.height), MAX_REFERENCE_SIDE);
        assert!(r.notes.iter().any(|n| n.contains("downscaled")));
        let many: Vec<LoadedRef> = (0..9).map(|_| r.clone()).collect();
        assert!(check_set_caps(&many).is_err());
        assert!(check_set_caps(&many[..2]).is_ok());
    }
}
