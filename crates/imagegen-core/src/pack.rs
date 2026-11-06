//! Project style packs: `.hoi4-imagegen/packs/<name>/pack.toml`.
//!
//! A mod with a house style of its own (the first case was a total conversion whose interface is drawn as a teal
//! terminal) needs that style across many families at once, not one style profile at a time. A style pack is one
//! named object that holds, per family:
//!
//! * reference images inside the mod, pinned by SHA-256 and checked again at plan time (a changed file is refused,
//!   as pinned styles do),
//! * a `look` line appended to the prompt's style,
//! * an optional form override that replaces the family's vanilla form rules (`form`) and negatives (`negative`),
//! * an optional canvas,
//! * optional processing switches where the mod's art differs from vanilla processing ([`ProcessingSwitches`]).
//!
//! A `gui_texture` entry may key its references, look and form by texture kind (`kinds.panel` and so on).
//!
//! A pack has no authority of its own: it never changes what an agent may do, only which references, prompt text,
//! canvas and finishing a plan uses. The pack's references stay the mod's own files; the tool never copies them
//! anywhere but into the mod that imported them.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::chain::Recipe;
use crate::error::{Error, Result, RetryClass};
use crate::family::{Family, GuiTextureKind};
use crate::fsio::ProjectFs;
use crate::hashing::sha256_hex;
use crate::naming::validate_snake;
use crate::profile::{Background, Profile};
use crate::style::StyleRef;
use crate::vanillamatch::{Calibration, Features};

/// Folder of the packs.
pub const PACK_DIR: &str = ".hoi4-imagegen/packs";
/// File name of a pack inside its folder.
pub const PACK_FILE: &str = "pack.toml";
/// The pack name that means "no pack" in a plan (it turns a project default off).
pub const NO_PACK: &str = "none";
/// Most references one family entry (or kind) may hold.
pub const MAX_REFS: usize = 16;
/// Most pack references a plan sends.
pub const MAX_SEND: usize = 8;
/// Largest pack file read.
pub const MAX_PACK_BYTES: usize = 512 * 1024;
/// Version of the calibration rule of the review's pack-match check. Version 2 added the shade floor
/// ([`SHADE_STEP`]) and the features left unchecked for unit art ([`unchecked`]).
pub const MATCH_VERSION: &str = "pack_match_v2";
/// Pack sets are small (about eight images a family), too few for percentiles. The accepted range of a feature is
/// the pack files' minimum to maximum, widened on each side by this many times that width.
///
/// Why one width: for a normally distributed feature the range of eight samples is about 2.85 standard deviations on
/// average, so the accepted span is about 8.5 of them. The vanilla rule (1st to 99th percentile, 4.65 deviations,
/// widened by half on each side) spans about 9.3. The two rules accept about the same spread, and the small-set
/// rule does not depend on percentiles that eight files cannot estimate.
pub const MATCH_WIDENING: f64 = 1.0;
/// The accepted range is never narrower than this many times the vanilla rule's floor for the feature: a pack of
/// near-identical files (one flat colour in every reference) would otherwise refuse any variation at all.
pub const MATCH_FLOOR_FACTOR: f64 = 2.0;
/// One shade step: every channel of a reference's visible pixels moved by this many levels, lighter and darker.
/// Where a pack's references barely spread in a feature, its accepted half-width is at least [`SHADE_MARGIN`] times
/// the largest change one step makes to any reference, so a flat glyph drawn one shade off still passes. Measured on
/// TNO's battalion counters (one teal, value 0.780, saturation 0.553): one step darker moves the value to 0.718 and
/// the saturation to 0.601, and moves every glyph pixel into the middle value band (`mid_tone_share` 0 to 1).
pub const SHADE_STEP: u8 = 16;
/// The shade floor's margin over one [`SHADE_STEP`]: one and a half steps.
pub const SHADE_MARGIN: f64 = 1.5;

fn is_false(b: &bool) -> bool {
    !*b
}

/// Finishing that differs from vanilla processing where the mod's own art differs. Every switch is off by default.
/// Each one is measured on the mod's references and documented in `docs/reference.md` (style packs).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProcessingSwitches {
    /// Unit art (`counter_large`, `counter_small`, `division_emblem`): keep the art's own colours instead of
    /// redrawing it in vanilla's green (or grey) material. The glyph is fitted into vanilla's glyph box and frame 2 of
    /// a counter is the NATO box drawn as an outline in the glyph's own colour, with no fill. Size, frames and alpha
    /// still conform. The `unit_material` check is left out.
    #[serde(default, skip_serializing_if = "is_false")]
    pub keep_colours: bool,
    /// `report_event` and `operation_picture`: an upright print with no vanilla tilt, mask, sepia or shadow. The art
    /// is fitted to the canvas as it is (its own alpha kept), and the `card_corners` check is left out.
    #[serde(default, skip_serializing_if = "is_false")]
    pub upright_print: bool,
    /// Transparent icon families (for example `technology` and `gui_texture`): the art is a panel that fills the
    /// canvas, so the source may be opaque. It is cover-resized to the canvas, the model is asked for an opaque
    /// picture, and the margin, fill, opaque-corner and side-view checks are left out.
    #[serde(default, skip_serializing_if = "is_false")]
    pub fill_canvas: bool,
    /// Flat terminal-style art: every resize uses an area (box) filter instead of Lanczos3, so no ringing halo is
    /// added and every colour stays inside the art's own palette, and no defringe erosion runs. The `fringe` check is
    /// left out because thin green lines are the style, not a key colour.
    #[serde(default, skip_serializing_if = "is_false")]
    pub crisp: bool,
    /// With `keep_colours`: snap each glyph pixel to the palette measured on the pack's references for the family
    /// (fill, outline and accent colours), alpha kept. On by default with `keep_colours` (`None`), `false` turns it
    /// off, and it cannot be set without `keep_colours`. Needs the family's references; it never runs without a pack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snap_palette: Option<bool>,
    /// With `keep_colours`: draw the soft edge glow measured on the pack's references around the glyph and the NATO
    /// outline, frame by frame. On by default with `keep_colours` (`None`), `false` turns it off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_glow: Option<bool>,
}

impl ProcessingSwitches {
    /// True when every switch is off.
    pub fn is_off(&self) -> bool {
        *self == ProcessingSwitches::default()
    }

    /// True when the palette snap runs: `keep_colours` is on and `snap_palette` is not `false`.
    pub fn snaps_palette(&self) -> bool {
        self.keep_colours && self.snap_palette != Some(false)
    }

    /// True when the edge glow is drawn: `keep_colours` is on and `edge_glow` is not `false`.
    pub fn draws_edge_glow(&self) -> bool {
        self.keep_colours && self.edge_glow != Some(false)
    }

    /// Names of the switches in force (`snap_palette` and `edge_glow` also when on by default).
    pub fn names(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.keep_colours {
            out.push("keep_colours");
        }
        if self.snaps_palette() {
            out.push("snap_palette");
        }
        if self.draws_edge_glow() {
            out.push("edge_glow");
        }
        if self.upright_print {
            out.push("upright_print");
        }
        if self.fill_canvas {
            out.push("fill_canvas");
        }
        if self.crisp {
            out.push("crisp");
        }
        out
    }
}

/// What a pack changed in a family's effective profile: its name and the processing switches. The profile's hash
/// covers it, so plans, validations and approvals made with a pack stay tied to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PackProfile {
    /// Pack name.
    pub name: String,
    /// Processing switches in force.
    #[serde(default, skip_serializing_if = "ProcessingSwitches::is_off")]
    pub processing: ProcessingSwitches,
}

/// A `gui_texture` entry for one texture kind.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KindEntry {
    /// One line appended to the prompt's style for this kind (the family's look when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<String>,
    /// Form rules that replace the family's vanilla form rules for this kind.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub form: Vec<String>,
    /// Negatives that replace the family's vanilla negatives for this kind.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub negative: Vec<String>,
    /// References for this kind, pinned by SHA-256 (the family's references when empty).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<StyleRef>,
}

/// One family's entry in a pack.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackEntry {
    /// One line appended to the prompt's style, for example "flat teal pictogram on transparent, crisp pixel edges".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<String>,
    /// Form rules that replace the family's vanilla form rules in the prompt, one sentence each. `{w}` and `{h}`
    /// are the canvas.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub form: Vec<String>,
    /// Negatives that replace the family's vanilla negatives in the prompt.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub negative: Vec<String>,
    /// Canvas `[width, height]` of one frame, replacing the family's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas: Option<[u32; 2]>,
    /// Processing switches, all off by default.
    #[serde(default, skip_serializing_if = "ProcessingSwitches::is_off")]
    pub processing: ProcessingSwitches,
    /// References inside the mod, pinned by SHA-256.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<StyleRef>,
    /// `gui_texture` only: references, look and form per texture kind.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub kinds: BTreeMap<GuiTextureKind, KindEntry>,
}

/// A style pack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StylePack {
    /// Schema version.
    #[serde(default = "one")]
    pub schema: u32,
    /// Name (snake_case); also the folder name.
    pub name: String,
    /// What the pack is for.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Who approved the pack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_by: Option<String>,
    /// When (RFC 3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_on: Option<String>,
    /// Entries by family id.
    #[serde(default)]
    pub families: BTreeMap<Family, PackEntry>,
}

fn one() -> u32 {
    1
}

/// The part of a pack that applies to one plan: a family entry, narrowed to a texture kind where the pack keys one.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    /// `decision`, or `gui_texture.panel` for a kind entry.
    pub entry: String,
    /// The look line, if any.
    pub look: Option<String>,
    /// Form rules that replace the vanilla ones (empty: the vanilla rules stand).
    pub form: Vec<String>,
    /// Negatives that replace the vanilla ones (empty: the vanilla negatives stand).
    pub negative: Vec<String>,
    /// References.
    pub references: Vec<StyleRef>,
}

/// Which families a switch may be used for, by the recipe the family has without switches.
pub fn switch_applies(switch: &str, profile: &Profile) -> bool {
    let recipe = crate::chain::recipe_of(&base_of(profile));
    match switch {
        "keep_colours" => matches!(recipe, Recipe::UnitCounter | Recipe::DivisionSymbol),
        "upright_print" => matches!(recipe, Recipe::ReportCard | Recipe::PhaseCard),
        "fill_canvas" => recipe == Recipe::Icon,
        "crisp" => matches!(recipe, Recipe::Icon | Recipe::UnitCounter | Recipe::DivisionSymbol),
        "snap_palette" | "edge_glow" => matches!(recipe, Recipe::UnitCounter | Recipe::DivisionSymbol),
        _ => false,
    }
}

fn base_of(profile: &Profile) -> Profile {
    let mut p = profile.clone();
    p.pack = None;
    p
}

fn check_lines(what: &str, lines: &[String], max: usize) -> Result<()> {
    if lines.len() > max {
        return Err(Error::Invalid(format!("{what}: at most {max} lines")));
    }
    for l in lines {
        if l.trim().is_empty() || l.chars().count() > 600 || l.chars().any(|c| c.is_control()) {
            return Err(Error::Invalid(format!("{what}: each line is 1 to 600 characters of plain text")));
        }
    }
    Ok(())
}

fn check_look(what: &str, look: &Option<String>) -> Result<()> {
    if let Some(l) = look {
        check_lines(what, std::slice::from_ref(l), 1)?;
    }
    Ok(())
}

fn check_refs(what: &str, refs: &[StyleRef]) -> Result<()> {
    if refs.len() > MAX_REFS {
        return Err(Error::Invalid(format!("{what}: at most {MAX_REFS} references")));
    }
    for r in refs {
        crate::fsio::check_relative(&r.path)?;
        if r.note.chars().count() > 600 {
            return Err(Error::Invalid(format!("{what}: a reference note is at most 600 characters")));
        }
    }
    Ok(())
}

impl StylePack {
    /// Check names, sizes and which switches each family may use.
    pub fn validate(&self) -> Result<()> {
        validate_snake("pack name", &self.name)?;
        if self.name == NO_PACK {
            return Err(Error::Invalid(format!("'{NO_PACK}' is reserved: a plan's style_pack \"{NO_PACK}\" turns the project default off")));
        }
        if self.description.chars().count() > 2000 {
            return Err(Error::Invalid("a pack description is at most 2000 characters".into()));
        }
        let builtin = crate::profile::ProfileSet::builtin();
        for (family, e) in &self.families {
            let at = family.as_str();
            check_look(&format!("{at}.look"), &e.look)?;
            check_lines(&format!("{at}.form"), &e.form, 12)?;
            check_lines(&format!("{at}.negative"), &e.negative, 40)?;
            check_refs(&format!("{at}.references"), &e.references)?;
            if let Some(c) = e.canvas {
                if !(1..=4096).contains(&c[0]) || !(1..=4096).contains(&c[1]) {
                    return Err(Error::Invalid(format!("{at}.canvas: sides must be 1 to 4096")));
                }
            }
            if !e.kinds.is_empty() && *family != Family::GuiTexture {
                return Err(Error::Invalid(format!("{at}.kinds: only gui_texture has texture kinds")));
            }
            for (kind, k) in &e.kinds {
                let at = format!("{at}.kinds.{kind}");
                check_look(&format!("{at}.look"), &k.look)?;
                check_lines(&format!("{at}.form"), &k.form, 12)?;
                check_lines(&format!("{at}.negative"), &k.negative, 40)?;
                check_refs(&format!("{at}.references"), &k.references)?;
            }
            let profile = builtin.for_family(*family).expect("every family has a profile");
            for s in e.processing.names() {
                if !switch_applies(s, profile) {
                    return Err(Error::Invalid(format!("{at}.processing.{s} does not apply to {family} (see docs/reference.md, style packs)")));
                }
            }
            for (on, s) in [(e.processing.snap_palette, "snap_palette"), (e.processing.edge_glow, "edge_glow")] {
                if on == Some(true) && !e.processing.keep_colours {
                    return Err(Error::Invalid(format!("{at}.processing.{s} works on art kept in its own colours: it needs keep_colours")));
                }
            }
            if e.processing.keep_colours && e.processing.fill_canvas {
                return Err(Error::Invalid(format!("{at}.processing: keep_colours and fill_canvas cannot be combined")));
            }
        }
        Ok(())
    }

    /// The entry of a family, narrowed to `kind` when the pack keys that kind. `None` when the pack does not cover
    /// the family.
    pub fn resolve(&self, family: Family, kind: Option<GuiTextureKind>) -> Option<Resolved> {
        let e = self.families.get(&family)?;
        let k = kind.and_then(|k| e.kinds.get(&k).map(|x| (k, x)));
        let pick = |kind_lines: &[String], family_lines: &[String]| if kind_lines.is_empty() { family_lines.to_vec() } else { kind_lines.to_vec() };
        Some(match k {
            Some((kind, x)) => Resolved {
                entry: format!("{family}.{kind}"),
                look: x.look.clone().or_else(|| e.look.clone()),
                form: pick(&x.form, &e.form),
                negative: pick(&x.negative, &e.negative),
                references: if x.references.is_empty() { e.references.clone() } else { x.references.clone() },
            },
            None => Resolved {
                entry: family.as_str().to_string(),
                look: e.look.clone(),
                form: e.form.clone(),
                negative: e.negative.clone(),
                references: e.references.clone(),
            },
        })
    }

    /// Every pinned reference of the pack, with where it sits (`decision`, `gui_texture.panel`).
    pub fn all_references(&self) -> Vec<(String, &StyleRef)> {
        let mut out = Vec::new();
        for (family, e) in &self.families {
            out.extend(e.references.iter().map(|r| (family.as_str().to_string(), r)));
            for (kind, k) in &e.kinds {
                out.extend(k.references.iter().map(|r| (format!("{family}.{kind}"), r)));
            }
        }
        out
    }
}

/// Check that each reference still matches its recorded hash. A changed or missing file is refused, as a pinned
/// style's is.
pub fn verify_references(fs: &ProjectFs, pack: &str, refs: &[StyleRef]) -> Result<()> {
    for r in refs {
        let bytes = fs.read(&r.path, crate::refs::MAX_REFERENCE_BYTES).map_err(|e| match e {
            Error::NotFound(_) => Error::refused(
                "pack_reference_missing",
                format!("reference {} of the style pack '{pack}' is missing; restore it or save the pack again", r.path),
                RetryClass::RefreshPreconditions,
            ),
            other => other,
        })?;
        if sha256_hex(&bytes) != r.sha256 {
            return Err(Error::refused(
                "pack_reference_changed",
                format!(
                    "reference {} no longer matches the hash recorded when the style pack '{pack}' was saved; save the pack again to pin the new file",
                    r.path
                ),
                RetryClass::RefreshPreconditions,
            ));
        }
    }
    Ok(())
}

/// The family's profile with the pack applied: its canvas, its processing switches and the checks those switches
/// contradict. The prompt is changed separately ([`prompt_rules`]), because a texture kind may change it.
pub fn apply_profile(base: &Profile, pack: &str, entry: &PackEntry) -> Profile {
    let mut p = base.clone();
    let sw = entry.processing;
    if let Some(c) = entry.canvas {
        if p.canvas != c {
            p.overridden.push("canvas".into());
        }
        p.canvas = c;
        if !p.accepted.contains(&c) {
            p.accepted.insert(0, c);
        }
    }
    let mut drop: Vec<&str> = Vec::new();
    if sw.keep_colours {
        drop.push("unit_material");
    }
    if sw.upright_print {
        drop.push("card_corners");
    }
    if sw.fill_canvas {
        // a panel fills the canvas: an opaque source is right, and the transparent-icon checks do not apply
        p.imagegen.background = Background::Opaque;
        drop.extend(["opaque_canvas", "margin", "fill", "side_view"]);
    }
    if sw.crisp {
        drop.push("fringe");
    }
    if !entry.form.is_empty() {
        // the side-view check measures vanilla's technology form, which the pack's form replaces
        drop.push("side_view");
    }
    p.imagegen.checks.retain(|c| !drop.contains(&c.as_str()));
    p.overridden.push("style_pack".into());
    p.pack = Some(PackProfile { name: pack.to_string(), processing: sw });
    p
}

/// The prompt rules and negatives a plan uses: the pack's form and negatives replace the family's vanilla ones where
/// the pack gives them.
pub fn prompt_rules(profile: &Profile, r: &Resolved) -> (Vec<String>, Vec<String>) {
    let rules = if r.form.is_empty() { profile.imagegen.prompt.rules.clone() } else { r.form.clone() };
    let negative = if r.negative.is_empty() { profile.imagegen.prompt.negative.clone() } else { r.negative.clone() };
    (rules, negative)
}

/// Features measured and reported but never failing a candidate of `family` against its pack.
///
/// Unit art (`counter_large`, `counter_small`, `division_emblem`): `mid_tone_share`. A kept-colours glyph is flat, one
/// colour, so the share of its pixels with a value from 0.25 to 0.75 is a step function of that one colour: 0 or 1.
/// TNO's 8 battalion counters are all 0.00 to 0.02 because their teal's value is 0.780, 0.03 above the band, and one
/// shade darker makes all 8 read 1.0. The 0.3.4 gallery's 8 TNO counters, flat and in the right medium, read 0.93
/// to 1.0 on 6 and 0 on 2. The feature tells shaded fill from line art on painted families; on flat unit art it
/// measures only which side of 0.75 the colour's value lies, which `saturation_mean` and the palette snap already
/// cover. The vanilla check (without a pack) keeps it.
pub fn unchecked(family: Family) -> &'static [&'static str] {
    match family {
        Family::CounterLarge | Family::CounterSmall | Family::DivisionEmblem => &["mid_tone_share"],
        _ => &[],
    }
}

/// One pack reference, measured: its features, and how far each feature moves when the reference is drawn one
/// [`SHADE_STEP`] lighter or darker (the larger of the two).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReferenceRow {
    /// The features.
    pub features: Features,
    /// The change one shade step makes to each feature.
    pub shade: Features,
}

fn shaded(img: &crate::pixels::Rgba, step: i16) -> crate::pixels::Rgba {
    let mut out = img.clone();
    for p in out.data.chunks_exact_mut(4) {
        for c in &mut p[..3] {
            *c = (i16::from(*c) + step).clamp(0, 255) as u8;
        }
    }
    out
}

/// Measure a pack reference ([`ReferenceRow`]). `None` when it has nothing to measure.
pub fn measure_reference(img: &crate::pixels::Rgba) -> Option<ReferenceRow> {
    let features = crate::vanillamatch::measure(img)?;
    let mut shade = [0f64; crate::vanillamatch::N];
    for step in [-i16::from(SHADE_STEP), i16::from(SHADE_STEP)] {
        if let Some(f) = crate::vanillamatch::measure(&shaded(img, step)) {
            for d in 0..crate::vanillamatch::N {
                shade[d] = shade[d].max((f[d] - features[d]).abs());
            }
        }
    }
    Some(ReferenceRow { features, shade })
}

/// Fit the pack-match ranges from a family's measured pack references: [`MATCH_WIDENING`], the floor, the shade
/// floor and the features [`unchecked`] for the family.
pub fn calibrate(family: Family, rows: &[ReferenceRow]) -> Option<Calibration> {
    let features: Vec<Features> = rows.iter().map(|r| r.features).collect();
    let mut shade = [0f64; crate::vanillamatch::N];
    for r in rows {
        for (s, v) in shade.iter_mut().zip(r.shade) {
            *s = s.max(SHADE_MARGIN * v);
        }
    }
    crate::vanillamatch::calibrate_min_max(family, &features, MATCH_WIDENING, MATCH_FLOOR_FACTOR, MATCH_VERSION, Some(&shade), unchecked(family))
}

/// Leave-one-out self-check of a pack set: how many of its own references pass ranges fitted on the others.
/// `None` with fewer than four measurable references.
pub fn leave_one_out(family: Family, rows: &[ReferenceRow]) -> Option<(usize, usize)> {
    if rows.len() < crate::vanillamatch::MIN_FILES + 1 {
        return None;
    }
    let mut pass = 0;
    for i in 0..rows.len() {
        let rest: Vec<ReferenceRow> = rows.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, r)| *r).collect();
        if calibrate(family, &rest).is_some_and(|c| c.check(&rows[i].features).pass) {
            pass += 1;
        }
    }
    Some((pass, rows.len()))
}

/// Folder of a pack.
pub fn pack_dir(name: &str) -> Result<String> {
    validate_snake("pack name", name)?;
    Ok(format!("{PACK_DIR}/{name}"))
}

/// Path of a pack file.
pub fn pack_path(name: &str) -> Result<String> {
    Ok(format!("{}/{PACK_FILE}", pack_dir(name)?))
}

/// Parse pack text: TOML, or JSON when it starts with `{`.
pub fn parse(text: &str) -> Result<StylePack> {
    if text.trim_start().starts_with('{') {
        serde_json::from_str(text).map_err(|e| Error::Config(e.to_string()))
    } else {
        toml::from_str(text).map_err(|e| Error::Config(e.to_string()))
    }
}

/// Load a pack by name.
pub fn load(fs: &ProjectFs, name: &str) -> Result<StylePack> {
    let rel = pack_path(name)?;
    let bytes = fs.read(&rel, MAX_PACK_BYTES).map_err(|e| match e {
        Error::NotFound(_) => Error::NotFound(format!("style pack '{name}' (expected {rel})")),
        other => other,
    })?;
    let text = String::from_utf8(bytes).map_err(|_| Error::Config(format!("{rel} is not UTF-8")))?;
    let pack = parse(&text).map_err(|e| Error::Config(format!("{rel}: {e}")))?;
    if pack.name != name {
        return Err(Error::Config(format!("{rel} declares the name '{}'", pack.name)));
    }
    pack.validate()?;
    Ok(pack)
}

/// SHA-256 of a saved pack file.
pub fn file_sha256(fs: &ProjectFs, name: &str) -> Result<String> {
    Ok(sha256_hex(&fs.read(&pack_path(name)?, MAX_PACK_BYTES)?))
}

/// Names of the saved packs.
pub fn list(fs: &ProjectFs) -> Result<Vec<String>> {
    let mut names: Vec<String> =
        fs.list_names(PACK_DIR)?.into_iter().filter(|n| n != "backup" && fs.exists(&format!("{PACK_DIR}/{n}/{PACK_FILE}"))).collect();
    names.sort();
    Ok(names)
}

/// The pack as TOML text.
pub fn to_toml(pack: &StylePack) -> Result<String> {
    toml::to_string_pretty(pack).map_err(|e| Error::Config(e.to_string()))
}

/// Save a pack. An existing pack is replaced only when `replace` is true; the old file is kept as a versioned backup.
pub fn save(fs: &ProjectFs, pack: &StylePack, replace: bool) -> Result<String> {
    pack.validate()?;
    let rel = pack_path(&pack.name)?;
    let body = to_toml(pack)?;
    if fs.exists(&rel) {
        if !replace {
            return Err(Error::Exists(format!(
                "style pack '{}' already exists; pass replace to overwrite it (the old file is backed up)",
                pack.name
            )));
        }
        let old = fs.read(&rel, MAX_PACK_BYTES)?;
        fs.write_next_version(&format!("{PACK_DIR}/backup"), &pack.name, "toml", 0, &old)?;
        fs.replace(&rel, body.as_bytes())?;
    } else {
        fs.write_new(&rel, body.as_bytes())?;
    }
    Ok(rel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::ProfileSet;

    fn pack() -> StylePack {
        let mut families = BTreeMap::new();
        families.insert(
            Family::Decision,
            PackEntry {
                look: Some("flat teal pictogram".into()),
                form: vec!["A flat pictogram, {w}x{h}.".into()],
                processing: ProcessingSwitches { crisp: true, ..Default::default() },
                references: vec![StyleRef { path: "art/a.png".into(), sha256: sha256_hex(b"a"), note: "mine".into() }],
                ..Default::default()
            },
        );
        let mut kinds = BTreeMap::new();
        kinds.insert(
            GuiTextureKind::Panel,
            KindEntry {
                look: Some("navy panel".into()),
                references: vec![StyleRef { path: "art/p.png".into(), sha256: sha256_hex(b"p"), note: String::new() }],
                ..Default::default()
            },
        );
        families.insert(Family::GuiTexture, PackEntry { look: Some("terminal look".into()), kinds, ..Default::default() });
        StylePack { schema: 1, name: "house".into(), description: "a house style".into(), approved_by: None, approved_on: None, families }
    }

    #[test]
    fn toml_round_trip_save_load_list_and_backup() {
        let dir = tempfile::tempdir().unwrap();
        let fs = ProjectFs::new(dir.path()).unwrap();
        let rel = save(&fs, &pack(), false).unwrap();
        assert_eq!(rel, ".hoi4-imagegen/packs/house/pack.toml");
        let text = String::from_utf8(fs.read(&rel, MAX_PACK_BYTES).unwrap()).unwrap();
        assert!(text.contains("[families.decision]") && text.contains("[families.gui_texture.kinds.panel]"), "{text}");
        assert_eq!(load(&fs, "house").unwrap(), pack());
        assert_eq!(list(&fs).unwrap(), vec!["house"]);
        assert!(matches!(save(&fs, &pack(), false).unwrap_err(), Error::Exists(_)));
        save(&fs, &pack(), true).unwrap();
        assert_eq!(fs.list_names(".hoi4-imagegen/packs/backup").unwrap(), vec!["house-v1.toml"]);
        assert_eq!(list(&fs).unwrap(), vec!["house"], "the backup folder is not a pack");
        let json = serde_json::to_string(&pack()).unwrap();
        assert_eq!(parse(&json).unwrap(), pack(), "a pack may also be written as JSON");
    }

    #[test]
    fn validation_refuses_bad_names_switches_and_kinds() {
        let mut p = pack();
        p.name = "none".into();
        assert!(p.validate().is_err(), "the reserved name");
        let mut p = pack();
        p.families.get_mut(&Family::Decision).unwrap().processing.keep_colours = true;
        assert!(p.validate().unwrap_err().to_string().contains("keep_colours does not apply to decision"));
        let mut p = pack();
        p.families.insert(Family::Focus, PackEntry { kinds: pack().families[&Family::GuiTexture].kinds.clone(), ..Default::default() });
        assert!(p.validate().unwrap_err().to_string().contains("only gui_texture"));
        let mut p = pack();
        p.families.get_mut(&Family::Decision).unwrap().references[0].path = "../outside.png".into();
        assert!(p.validate().is_err());
        let mut p = pack();
        p.families.insert(
            Family::ReportEvent,
            PackEntry { processing: ProcessingSwitches { upright_print: true, ..Default::default() }, ..Default::default() },
        );
        p.families.insert(
            Family::CounterLarge,
            PackEntry { processing: ProcessingSwitches { keep_colours: true, crisp: true, ..Default::default() }, ..Default::default() },
        );
        p.families.insert(
            Family::Technology,
            PackEntry { processing: ProcessingSwitches { fill_canvas: true, crisp: true, ..Default::default() }, ..Default::default() },
        );
        p.validate().unwrap();
        assert!(parse("name = \"x\"\n[families.decision]\nlook = \"a\"\nunknown = 1\n").is_err(), "unknown keys are refused");
    }

    #[test]
    fn snap_palette_and_edge_glow_default_on_with_keep_colours_and_need_it() {
        let keep = ProcessingSwitches { keep_colours: true, ..Default::default() };
        assert!(keep.snaps_palette() && keep.draws_edge_glow());
        assert_eq!(keep.names(), vec!["keep_colours", "snap_palette", "edge_glow"]);
        let off = ProcessingSwitches { keep_colours: true, snap_palette: Some(false), edge_glow: Some(false), ..Default::default() };
        assert!(!off.snaps_palette() && !off.draws_edge_glow());
        assert!(!ProcessingSwitches { snap_palette: Some(true), ..Default::default() }.snaps_palette(), "never without keep_colours");
        // the default is not written, so a pack saved before 0.3.6 keeps its profile hash
        assert_eq!(toml::to_string(&keep).unwrap(), "keep_colours = true\n");
        let mut p = pack();
        p.families.insert(
            Family::CounterLarge,
            PackEntry { processing: ProcessingSwitches { snap_palette: Some(true), ..Default::default() }, ..Default::default() },
        );
        assert!(p.validate().unwrap_err().to_string().contains("needs keep_colours"));
        p.families.get_mut(&Family::CounterLarge).unwrap().processing.keep_colours = true;
        p.validate().unwrap();
    }

    #[test]
    fn resolve_narrows_to_a_kind_and_falls_back_to_the_family() {
        let p = pack();
        let r = p.resolve(Family::GuiTexture, Some(GuiTextureKind::Panel)).unwrap();
        assert_eq!(r.entry, "gui_texture.panel");
        assert_eq!(r.look.as_deref(), Some("navy panel"));
        assert_eq!(r.references.len(), 1);
        let r = p.resolve(Family::GuiTexture, Some(GuiTextureKind::Button)).unwrap();
        assert_eq!(r.entry, "gui_texture");
        assert_eq!(r.look.as_deref(), Some("terminal look"));
        assert!(r.references.is_empty());
        assert!(p.resolve(Family::Focus, None).is_none(), "a family the pack does not cover");
        assert_eq!(p.all_references().len(), 2);
    }

    #[test]
    fn changed_and_missing_references_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let fs = ProjectFs::new(dir.path()).unwrap();
        fs.write_new("art/a.png", b"a").unwrap();
        let refs = pack().families[&Family::Decision].references.clone();
        verify_references(&fs, "house", &refs).unwrap();
        fs.replace("art/a.png", b"changed").unwrap();
        assert_eq!(verify_references(&fs, "house", &refs).unwrap_err().code(), "pack_reference_changed");
        let missing = pack().families[&Family::GuiTexture].kinds[&GuiTextureKind::Panel].references.clone();
        assert_eq!(verify_references(&fs, "house", &missing).unwrap_err().code(), "pack_reference_missing");
    }

    #[test]
    fn applying_a_pack_changes_canvas_checks_and_the_profile_hash() {
        let set = ProfileSet::builtin();
        let report = set.for_family(Family::ReportEvent).unwrap();
        let entry = PackEntry {
            canvas: Some([175, 204]),
            processing: ProcessingSwitches { upright_print: true, ..Default::default() },
            ..Default::default()
        };
        let p = apply_profile(report, "house", &entry);
        assert_eq!(p.canvas, [175, 204]);
        assert_eq!(p.accepted[0], [175, 204]);
        assert!(!p.imagegen.checks.contains(&"card_corners".to_string()));
        assert_eq!(p.pack.as_ref().unwrap().name, "house");
        assert_ne!(ProfileSet::profile_hash(&p), ProfileSet::profile_hash(report));
        // a family without a pack keeps its hash: the new field is not written when unset
        assert!(!serde_json::to_string(report).unwrap().contains("\"pack\""));

        let tech = set.for_family(Family::Technology).unwrap();
        let entry = PackEntry {
            form: vec!["A blueprint panel.".into()],
            processing: ProcessingSwitches { fill_canvas: true, ..Default::default() },
            ..Default::default()
        };
        let p = apply_profile(tech, "house", &entry);
        assert_eq!(p.imagegen.background, Background::Opaque);
        for gone in ["opaque_canvas", "margin", "fill", "side_view"] {
            assert!(!p.imagegen.checks.contains(&gone.to_string()), "{gone}");
        }
        let r = Resolved { entry: "technology".into(), look: None, form: entry.form.clone(), negative: vec![], references: vec![] };
        let (rules, negative) = prompt_rules(&p, &r);
        assert_eq!(rules, vec!["A blueprint panel."]);
        assert_eq!(negative, tech.imagegen.prompt.negative, "negatives stand unless the pack replaces them");
    }
}
