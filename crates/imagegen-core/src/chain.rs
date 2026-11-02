//! Each family's processing chain: what a caller may add, what always runs, and how a record proves it ran.
//!
//! A chain has two parts. **Pre-steps** are optional and come from the caller: only the four that prepare a
//! source for the family's own processing (`chroma_key`, `crop_to_aspect`, `defringe`, `trim_to_subject`) are
//! allowed. **Finishing** is the family's required processing. The tool appends it itself, so a caller cannot ask
//! for less, and it is recorded in the processed record together with the recipe id and version. Validation and
//! the install gate read the record back and refuse an artifact whose chain lacks a required stage.
//!
//! Finishing is made of core [`Op`]s where one image goes in and one comes out, and of *finishers* where the
//! engine supplies something the core does not have (a template, a face detector) or more than one image comes
//! out (the achievement triplet, the three flag sizes).
//!
//! | Recipe | Families | Required stages, in order |
//! |---|---|---|
//! | `icon` v1 | transparent icons, technology, modifier icon, GUI texture | `fit_subject` (`conform` when finished art was imported at an accepted size), `grayscale` for grayscale families |
//! | `picture` v1 | opaque pictures, news, super event | `flatten_on`, `cover_resize` or `conform`, `grayscale` for news |
//! | `report_card` v2 | report event | `report_card` or `conform` (an imported finished card) |
//! | `selected_strip` v1 | faction logo, agency logo | `fit_subject`, `selected_frame` (or one `conform` for an imported finished strip) |
//! | `landmark_strip` v1 | building | `fit_subject`, `second_frame`: the ruined state, a second candidate of the same asset (or one `conform`) |
//! | `unit_counter` v1 | battalion and on-map counters | `unit_glyph` (silhouette to vanilla's material), `nato_frame` (the NATO box as frame 2), or one `conform` for an imported finished strip |
//! | `division_symbol` v1 | division emblem | `unit_glyph`, or `conform` for an imported finished 76x42 symbol |
//! | `portrait` v1 | the four portraits | `flatten_on`, `extract_portrait` (or `conform` for a finished 156x210) |
//! | `advisor_card` v1 | advisor card | `advisor_card` (or `conform` for a finished 65x67 card) |
//! | `achievement` v1 | achievement | `fit_subject`, `achievement_triplet` |
//! | `flag` v1 | flag | `flatten_on`, `flag_ladder` |
//!
//! Families whose sprite the game looks up together with a companion (the division emblem's small symbol, the
//! on-map counter's text icon, the faction logo's miniature, a landmark's small strip, a medal's small strip) end
//! with the `derived_companions` finisher, whatever the recipe; the record must then carry each companion
//! ([`crate::lookup::derived_companions`]).

use crate::error::{Error, Result, RetryClass};
use crate::family::Family;
use crate::hashing::sha256_hex;
use crate::pixels::Rgba;
use crate::process::{edge_stats, Op};
use crate::profile::{Background, Profile};
use crate::records::{CompanionRecord, ProcessedRecord};

/// The pre-steps a caller may pass. Each prepares the source and none finishes the asset.
pub const PRE_STEP_OPS: [&str; 4] = ["chroma_key", "crop_to_aspect", "defringe", "trim_to_subject"];

/// What flat colour provider alpha is composited over in an opaque family.
pub const FLATTEN_BACKGROUND: [u8; 3] = [0, 0, 0];

/// Achievement subject layers keep this margin so the template shows around them.
pub const ACHIEVEMENT_MARGIN: u32 = 2;

/// Check a caller's pre-steps. An empty list is refused too: it is how processing used to be skipped, and
/// omitting the field means the same without looking like a request to skip.
pub fn check_pre_steps(steps: &[Op]) -> Result<()> {
    if steps.is_empty() {
        return Err(Error::refused(
            "chain_steps_empty",
            "an empty step list asks for no processing. The family's processing always runs and cannot be skipped; omit steps, or pass pre-steps (chroma_key, crop_to_aspect, defringe, trim_to_subject) that prepare the source for it",
            RetryClass::Terminal,
        ));
    }
    for s in steps {
        if !PRE_STEP_OPS.contains(&s.name()) {
            return Err(Error::refused(
                "chain_step_not_allowed",
                format!(
                    "'{}' is not a pre-step. Callers may only pass {} to prepare the source. The family's finishing steps are added and run by the tool itself and cannot be replaced or left out",
                    s.name(),
                    PRE_STEP_OPS.join(", ")
                ),
                RetryClass::Terminal,
            ));
        }
    }
    Ok(())
}

/// A processing recipe: one id and version per way of finishing an asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recipe {
    /// Transparent single-frame icons.
    Icon,
    /// Opaque pictures.
    Picture,
    /// Report event card.
    ReportCard,
    /// Two frames, the second selected.
    SelectedStrip,
    /// Two frames: the landmark, then its ruined state (a second candidate of the same asset), as vanilla does.
    LandmarkStrip,
    /// A counter strip: the silhouette in vanilla's material, then the NATO box.
    UnitCounter,
    /// A division template symbol: the silhouette in vanilla's material.
    DivisionSymbol,
    /// Portraits.
    Portrait,
    /// Advisor card.
    AdvisorCard,
    /// Achievement triplet.
    Achievement,
    /// Flag ladder.
    Flag,
}

impl Recipe {
    /// Stable id recorded with every processed artifact.
    pub fn id(self) -> &'static str {
        match self {
            Recipe::Icon => "icon",
            Recipe::Picture => "picture",
            Recipe::ReportCard => "report_card",
            Recipe::SelectedStrip => "selected_strip",
            Recipe::LandmarkStrip => "landmark_strip",
            Recipe::UnitCounter => "unit_counter",
            Recipe::DivisionSymbol => "division_symbol",
            Recipe::Portrait => "portrait",
            Recipe::AdvisorCard => "advisor_card",
            Recipe::Achievement => "achievement",
            Recipe::Flag => "flag",
        }
    }

    /// Version, bumped whenever a recipe's output could change. The report card is at 2 (the shadow fix).
    pub fn version(self) -> u32 {
        match self {
            Recipe::ReportCard => crate::recipes::report_card::RECIPE_VERSION,
            Recipe::UnitCounter | Recipe::DivisionSymbol => crate::recipes::unit_art::RECIPE_VERSION,
            _ => 1,
        }
    }

    /// Keys of the companion images the recipe produces next to the main one.
    pub fn companion_keys(self) -> &'static [&'static str] {
        match self {
            Recipe::Achievement => &["_grey", "_not_eligible"],
            Recipe::Flag => &["medium", "small"],
            _ => &[],
        }
    }
}

/// The recipe of a family.
pub fn recipe_of(profile: &Profile) -> Recipe {
    match profile.family {
        Family::ReportEvent => Recipe::ReportCard,
        Family::FactionLogo | Family::AgencyLogo => Recipe::SelectedStrip,
        Family::Building => Recipe::LandmarkStrip,
        Family::CounterLarge | Family::CounterSmall => Recipe::UnitCounter,
        Family::DivisionEmblem => Recipe::DivisionSymbol,
        Family::LeaderPortrait | Family::CommanderPortrait | Family::OperativePortrait | Family::ScientistPortrait => Recipe::Portrait,
        Family::AdvisorCard => Recipe::AdvisorCard,
        Family::Achievement => Recipe::Achievement,
        Family::Flag => Recipe::Flag,
        _ if profile.imagegen.background == Background::Opaque => Recipe::Picture,
        _ => Recipe::Icon,
    }
}

/// An engine-level finishing stage: needs something the core does not have, or yields several images.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finisher {
    /// Advisor card from a portrait and the card template.
    AdvisorCard,
    /// Achievement triplet from the subject layer and the templates.
    AchievementTriplet,
    /// 156x210 portrait from a face-detected crop.
    ExtractPortrait,
    /// The three flag sizes.
    FlagLadder,
    /// The second frame of a landmark strip from a second candidate (the ruined state), fitted the same way.
    SecondFrame,
    /// The companions the game looks up next to the main sprite, derived from the finished main art.
    DerivedCompanions,
}

impl Finisher {
    /// Name recorded in the processed record and matched by the audit.
    pub fn name(self) -> &'static str {
        match self {
            Finisher::AdvisorCard => "advisor_card",
            Finisher::AchievementTriplet => "achievement_triplet",
            Finisher::ExtractPortrait => "extract_portrait",
            Finisher::FlagLadder => "flag_ladder",
            Finisher::SecondFrame => "second_frame",
            Finisher::DerivedCompanions => "derived_companions",
        }
    }
}

/// The finishing of one run: ops, then finishers.
#[derive(Debug, Clone, PartialEq)]
pub struct ChainPlan {
    /// Steps to run after the pre-steps.
    pub ops: Vec<Op>,
    /// Engine-level stages to run after the ops.
    pub finishers: Vec<Finisher>,
}

/// Grain seed of a report card: the first eight hex digits of the candidate's SHA-256.
pub fn seed_from_sha(sha256_hex: &str) -> i64 {
    i64::from(u32::from_str_radix(sha256_hex.get(..8).unwrap_or("0"), 16).unwrap_or(0))
}

/// True when `img` already is a finished report card: 210x176, the four corners fully transparent, an opaque
/// centre and a card-shaped alpha (a fifth to none of the canvas is transparent).
pub fn is_report_card(img: &Rgba) -> bool {
    if (img.width, img.height) != (210, 176) {
        return false;
    }
    let (w, h) = (img.width - 1, img.height - 1);
    let corners = [img.alpha(0, 0), img.alpha(w, 0), img.alpha(0, h), img.alpha(w, h)];
    if corners.iter().any(|a| *a != 0) || img.alpha(105, 88) != 255 {
        return false;
    }
    let clear = img.data.chunks_exact(4).filter(|p| p[3] == 0).count();
    let share = clear as f64 / (img.width * img.height) as f64;
    (0.005..=0.5).contains(&share)
}

fn transparent_icon_ops(img: &Rgba, canvas: [u32; 2], keyed: bool, margin: u32) -> Result<Vec<Op>> {
    if !keyed && !img.has_transparency() {
        return Err(Error::refused(
            "no_transparency_in_source",
            "the source is fully opaque, so it carries no transparency. A transparent asset needs real alpha: generate with a model that supports it, run a matte workflow, or pass a chroma_key pre-step if you know the background is one flat colour",
            RetryClass::Terminal,
        ));
    }
    let mut ops = Vec::new();
    if edge_stats(img).1 > 0 {
        ops.push(Op::Defringe { erosion: 1 });
    }
    ops.push(Op::TrimToSubject { min_alpha: 16 });
    ops.push(Op::FitSubject { width: canvas[0], height: canvas[1], margin, min_alpha: 16 });
    Ok(ops)
}

/// What the caller may choose about a family's finishing beyond the image itself.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FinishOptions {
    /// Counters: the NATO symbol of frame 2 (`recipes::unit_art::parse_symbol`). When absent it is inferred from the
    /// asset name, which for a counter is the sub-unit key.
    pub counter_symbol: Option<String>,
    /// The asset name, used to infer the NATO symbol.
    pub asset: Option<String>,
}

/// Which unit art a family makes.
pub fn unit_art_kind(family: Family) -> Option<crate::recipes::unit_art::UnitArtKind> {
    use crate::recipes::unit_art::UnitArtKind;
    match family {
        Family::CounterLarge => Some(UnitArtKind::CounterLarge),
        Family::CounterSmall => Some(UnitArtKind::CounterSmall),
        Family::DivisionEmblem => Some(UnitArtKind::DivisionEmblem),
        _ => None,
    }
}

/// The NATO symbol a counter's frame 2 gets, and where it came from (`given` or `inferred from the asset name`).
pub fn counter_symbol(opts: &FinishOptions) -> Result<(String, &'static str)> {
    use crate::recipes::unit_art::{infer_symbol, parse_symbol, symbol_text};
    match &opts.counter_symbol {
        Some(s) => Ok((symbol_text(&parse_symbol(s)?), "given")),
        None => Ok((symbol_text(&infer_symbol(opts.asset.as_deref().unwrap_or(""))), "inferred from the asset name")),
    }
}

/// What the family's processing is for `img`, the image after the caller's pre-steps, with default options.
pub fn plan_finishing(profile: &Profile, canvas: [u32; 2], img: &Rgba, imported: bool, keyed: bool, candidate_sha256: &str) -> Result<ChainPlan> {
    plan_finishing_with(profile, canvas, img, imported, keyed, candidate_sha256, &FinishOptions::default())
}

/// What the family's processing is for `img`, the image after the caller's pre-steps.
///
/// * `canvas` is the effective canvas of one frame.
/// * `imported` is true for art brought in with `hoi4.image_process`'s `import` (never for generated art): finished
///   art that already has an accepted size is taken at that size, which the chain records as `conform`.
/// * `keyed` is true when the caller's pre-steps include a `chroma_key`, which is what creates transparency.
/// * `candidate_sha256` seeds the report card's grain.
/// * `opts` carries the counter's NATO symbol and the asset name it may be inferred from.
pub fn plan_finishing_with(
    profile: &Profile,
    canvas: [u32; 2],
    img: &Rgba,
    imported: bool,
    keyed: bool,
    candidate_sha256: &str,
    opts: &FinishOptions,
) -> Result<ChainPlan> {
    let recipe = recipe_of(profile);
    let frames = profile.frames.max(1);
    let size = [img.width, img.height];
    let accepted_single = |s: [u32; 2]| profile.accepted.contains(&s) || s == canvas;
    let mut ops: Vec<Op> = Vec::new();
    let mut finishers = Vec::new();
    match recipe {
        Recipe::Icon => {
            if imported && accepted_single(size) {
                ops.push(Op::Conform { width: size[0], height: size[1] });
            } else {
                ops = transparent_icon_ops(img, canvas, keyed, profile.margin)?;
            }
            if profile.imagegen.grayscale {
                ops.push(Op::Grayscale);
            }
        }
        Recipe::Picture => {
            ops.push(Op::FlattenOn { background: FLATTEN_BACKGROUND });
            if imported && accepted_single(size) {
                ops.push(Op::Conform { width: size[0], height: size[1] });
            } else {
                ops.push(Op::CoverResize { width: canvas[0], height: canvas[1] });
            }
            if profile.imagegen.grayscale {
                ops.push(Op::Grayscale);
            }
        }
        Recipe::ReportCard => {
            if imported && is_report_card(img) {
                ops.push(Op::Conform { width: 210, height: 176 });
            } else {
                ops.push(Op::FlattenOn { background: FLATTEN_BACKGROUND });
                ops.push(Op::ReportCard { seed: seed_from_sha(candidate_sha256) });
            }
        }
        Recipe::SelectedStrip | Recipe::LandmarkStrip => {
            let accepted_strip = size[0].is_multiple_of(frames) && accepted_single([size[0] / frames, size[1]]);
            if imported && accepted_strip {
                ops.push(Op::Conform { width: size[0], height: size[1] });
            } else {
                let margin = if recipe == Recipe::SelectedStrip {
                    profile.margin.max(crate::recipes::frames::selected_fit_margin(profile.family))
                } else {
                    profile.margin
                };
                ops = transparent_icon_ops(img, canvas, keyed, margin)?;
                if recipe == Recipe::SelectedStrip {
                    ops.push(Op::SelectedFrame { frames });
                } else {
                    // vanilla landmark strips show the ruined building in frame 2: different art, so it comes from a
                    // second candidate of the same asset (usually hoi4.image_edit of the first)
                    finishers.push(Finisher::SecondFrame);
                }
            }
        }
        Recipe::UnitCounter | Recipe::DivisionSymbol => {
            let kind = unit_art_kind(profile.family).expect("unit families have a unit art kind");
            let finished = if recipe == Recipe::UnitCounter {
                size[0].is_multiple_of(frames) && accepted_single([size[0] / frames, size[1]])
            } else {
                accepted_single(size)
            };
            if imported && finished {
                ops.push(Op::Conform { width: size[0], height: size[1] });
            } else {
                let style = crate::recipes::unit_art::GlyphStyle::of(kind);
                if canvas != style.frame {
                    return Err(Error::Invalid(format!(
                        "{} art is drawn on vanilla's {}x{} frame, and this profile's canvas is {}x{}. Import finished art at the profile's size instead",
                        profile.family, style.frame[0], style.frame[1], canvas[0], canvas[1]
                    )));
                }
                ops.push(Op::UnitGlyph { kind });
                if recipe == Recipe::UnitCounter {
                    let (symbol, _) = counter_symbol(opts)?;
                    ops.push(Op::NatoFrame { kind, symbol });
                }
            }
        }
        Recipe::Portrait => {
            ops.push(Op::FlattenOn { background: FLATTEN_BACKGROUND });
            if imported && accepted_single(size) {
                ops.push(Op::Conform { width: size[0], height: size[1] });
            } else {
                finishers.push(Finisher::ExtractPortrait);
            }
        }
        Recipe::AdvisorCard => {
            if imported && accepted_single(size) && img.has_transparency() {
                ops.push(Op::Conform { width: size[0], height: size[1] });
            } else {
                ops.push(Op::FlattenOn { background: FLATTEN_BACKGROUND });
                finishers.push(Finisher::AdvisorCard);
            }
        }
        Recipe::Achievement => {
            if imported && size == [crate::recipes::achievement::IMAGE_SIZE; 2] && img.has_transparency() {
                ops.push(Op::Conform { width: size[0], height: size[1] });
            } else {
                ops = transparent_icon_ops(img, canvas, keyed, ACHIEVEMENT_MARGIN)?;
            }
            finishers.push(Finisher::AchievementTriplet);
        }
        Recipe::Flag => {
            ops.push(Op::FlattenOn { background: FLATTEN_BACKGROUND });
            finishers.push(Finisher::FlagLadder);
        }
    }
    if !crate::lookup::derived_companions(profile.family, canvas).is_empty() {
        finishers.push(Finisher::DerivedCompanions);
    }
    Ok(ChainPlan { ops, finishers })
}

/// Keys of every companion image a processed record of this family and canvas must carry: the recipe's, then the
/// derived companions the game looks up.
pub fn companion_keys_of(profile: &Profile, canvas: [u32; 2]) -> Vec<&'static str> {
    let mut keys: Vec<&'static str> = recipe_of(profile).companion_keys().to_vec();
    keys.extend(crate::lookup::derived_companions(profile.family, canvas).iter().map(|d| d.key));
    keys
}

/// Stages a processed record must show, in order. Each stage lists the names that satisfy it. A single `conform`
/// may satisfy several consecutive stages: it stands for finished art accepted whole.
pub fn required_stages(profile: &Profile) -> Vec<Vec<&'static str>> {
    let mut stages: Vec<Vec<&'static str>> = match recipe_of(profile) {
        Recipe::Icon => vec![vec!["fit_subject", "conform"]],
        Recipe::Picture => vec![vec!["flatten_on"], vec!["cover_resize", "conform"]],
        Recipe::ReportCard => vec![vec!["report_card", "conform"]],
        Recipe::SelectedStrip => vec![vec!["fit_subject", "conform"], vec!["selected_frame", "conform"]],
        Recipe::LandmarkStrip => vec![vec!["fit_subject", "conform"], vec!["second_frame", "conform"]],
        Recipe::UnitCounter => vec![vec!["unit_glyph", "conform"], vec!["nato_frame", "conform"]],
        Recipe::DivisionSymbol => vec![vec!["unit_glyph", "conform"]],
        Recipe::Portrait => vec![vec!["flatten_on"], vec!["extract_portrait", "conform"]],
        Recipe::AdvisorCard => vec![vec!["advisor_card", "conform"]],
        Recipe::Achievement => vec![vec!["fit_subject", "conform"], vec!["achievement_triplet"]],
        Recipe::Flag => vec![vec!["flatten_on"], vec!["flag_ladder"]],
    };
    if profile.imagegen.grayscale {
        stages.push(vec!["grayscale"]);
    }
    stages
}

/// Everything wrong with a processed record's chain, empty when it is complete. This is a check of the record
/// (recipe, version, stages, hash continuity), not of the pixels, which validation checks separately.
pub fn audit(rec: &ProcessedRecord, profile: &Profile) -> Vec<String> {
    let recipe = recipe_of(profile);
    let mut problems = Vec::new();
    let Some(r) = &rec.recipe else {
        return vec![format!(
            "the processed record carries no recipe: it was made before recipes were recorded or without the family's processing. Process the source candidate again (recipe {} v{})",
            recipe.id(),
            recipe.version()
        )];
    };
    if r.id != recipe.id() || r.version != recipe.version() {
        problems.push(format!("the record says recipe {} v{}, but {} uses {} v{}", r.id, r.version, profile.family, recipe.id(), recipe.version()));
    }
    let mut names: Vec<&str> = rec.steps.iter().map(|s| s.op.name()).collect();
    names.extend(r.finishers.iter().map(|f| f.name.as_str()));
    let mut at = 0usize;
    for stage in required_stages(profile) {
        match names.iter().enumerate().skip(at).find(|(_, n)| stage.contains(n)) {
            Some((i, _)) => at = i,
            None => problems.push(format!("the chain lacks the required stage {} (it ran: {})", stage.join(" or "), names.join(", "))),
        }
    }
    let companions: Vec<&str> = rec.companions.iter().map(|c| c.key.as_str()).collect();
    for key in recipe.companion_keys() {
        if !companions.contains(key) {
            problems.push(format!("the record lacks the companion image {key} that {} needs", recipe.id()));
        }
    }
    for d in crate::lookup::derived_companions(profile.family, rec.canvas) {
        match rec.companions.iter().find(|c| c.key == d.key) {
            None => problems.push(format!(
                "the record lacks the {} companion the game looks up next to {}'s sprite; process the source candidate again",
                d.key, profile.family
            )),
            Some(c) if c.size != d.strip() => {
                problems.push(format!("the {} companion is {}x{}, the game needs {}x{}", d.key, c.size[0], c.size[1], d.strip()[0], d.strip()[1]))
            }
            Some(_) => {}
        }
    }
    let last_out = r.finishers.last().map(|f| f.output_sha256.as_str()).or_else(|| rec.steps.last().map(|s| s.output_sha256.as_str()));
    if last_out != Some(rec.pixels_sha256.as_str()) {
        problems.push("the last stage of the chain does not produce the recorded pixels".into());
    }
    problems
}

/// One hash for an artifact and its companions, so approval of "the file" covers every file an install writes.
/// With no companions it is the main file's hash.
pub fn bundle_sha256(main_sha256: &str, companions: &[CompanionRecord]) -> String {
    if companions.is_empty() {
        return main_sha256.to_string();
    }
    let mut sorted: Vec<&CompanionRecord> = companions.iter().collect();
    sorted.sort_by(|a, b| a.key.cmp(&b.key));
    let mut text = format!("hoi4-imagegen-bundle-v1\nmain {main_sha256}\n");
    for c in sorted {
        text.push_str(&format!("{} {}\n", c.key, c.sha256));
    }
    sha256_hex(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::ProfileSet;
    use crate::records::{FinisherRecord, ProfileRef, RecipeRecord};

    fn prof(f: Family) -> Profile {
        ProfileSet::builtin().for_family(f).unwrap().clone()
    }

    fn blob() -> Rgba {
        let mut img = Rgba::new(200, 180);
        for y in 30..150 {
            for x in 40..170 {
                img.set_px(x, y, [120, 100, 80, 255]);
            }
        }
        img
    }

    #[test]
    fn only_preparing_steps_are_allowed_and_an_empty_list_is_refused() {
        assert!(check_pre_steps(&[Op::ChromaKey { key: [255, 0, 255], tolerance: 10 }, Op::TrimToSubject { min_alpha: 16 }]).is_ok());
        assert!(check_pre_steps(&[Op::CropToAspect { width: 1, height: 1 }, Op::Defringe { erosion: 1 }]).is_ok());
        assert_eq!(check_pre_steps(&[]).unwrap_err().code(), "chain_steps_empty");
        for bad in [
            Op::Grayscale,
            Op::Resize { width: 4, height: 4 },
            Op::FitSubject { width: 4, height: 4, margin: 0, min_alpha: 16 },
            Op::FlattenOn { background: [0, 0, 0] },
            Op::Conform { width: 1, height: 1 },
            Op::ReportCard { seed: 1 },
            Op::SelectedFrame { frames: 2 },
            Op::CoverResize { width: 4, height: 4 },
        ] {
            let err = check_pre_steps(&[Op::TrimToSubject { min_alpha: 16 }, bad.clone()]).unwrap_err();
            assert_eq!(err.code(), "chain_step_not_allowed", "{bad:?}");
            assert!(err.to_string().contains(bad.name()));
        }
    }

    #[test]
    fn every_family_has_one_recipe_and_its_required_stages() {
        for p in ProfileSet::builtin().all() {
            let stages = required_stages(p);
            assert!(!stages.is_empty(), "{}", p.family);
            let r = recipe_of(p);
            assert!(r.version() >= 1);
        }
        assert_eq!(recipe_of(&prof(Family::ReportEvent)).version(), 2);
        assert_eq!(recipe_of(&prof(Family::Focus)), Recipe::Icon);
        assert_eq!(recipe_of(&prof(Family::NewsEvent)), Recipe::Picture);
        assert_eq!(recipe_of(&prof(Family::Technology)), Recipe::Icon);
        assert_eq!(required_stages(&prof(Family::NewsEvent)).last().unwrap(), &vec!["grayscale"]);
        assert_eq!(recipe_of(&prof(Family::Achievement)).companion_keys(), &["_grey", "_not_eligible"]);
        assert_eq!(recipe_of(&prof(Family::Flag)).companion_keys(), &["medium", "small"]);
    }

    #[test]
    fn transparent_icons_finish_with_trim_and_fit_and_opaque_sources_are_refused() {
        let p = prof(Family::Focus);
        let plan = plan_finishing(&p, p.canvas, &blob(), false, false, &"0".repeat(64)).unwrap();
        assert_eq!(plan.ops.len(), 2);
        assert!(matches!(plan.ops[1], Op::FitSubject { width: 100, height: 88, margin: 2, .. }));
        assert!(plan.finishers.is_empty());
        let err = plan_finishing(&p, p.canvas, &Rgba::filled(64, 64, [1, 2, 3, 255]), false, false, "").unwrap_err();
        assert_eq!(err.code(), "no_transparency_in_source");
        // a keyed source is allowed through (the key step creates the transparency)
        assert!(plan_finishing(&p, p.canvas, &Rgba::filled(64, 64, [1, 2, 3, 255]), false, true, "").is_ok());
    }

    #[test]
    fn imported_finished_art_is_conformed_and_generated_art_is_always_finished() {
        let idea = prof(Family::IdeaSpirit);
        let finished = blob().crop(0, 0, 60, 68).unwrap();
        let plan = plan_finishing(&idea, idea.canvas, &finished, true, false, "").unwrap();
        assert_eq!(plan.ops, vec![Op::Conform { width: 60, height: 68 }]);
        let plan = plan_finishing(&idea, idea.canvas, &finished, false, false, "").unwrap();
        assert!(plan.ops.iter().any(|o| matches!(o, Op::FitSubject { .. })), "generated art is always fitted");
        let spec = prof(Family::SpecialProjectIcon);
        let grey = plan_finishing(&spec, spec.canvas, &Rgba::new(161, 98), true, false, "").unwrap();
        assert_eq!(grey.ops, vec![Op::Conform { width: 161, height: 98 }, Op::Grayscale]);
    }

    #[test]
    fn pictures_flatten_cover_and_news_goes_grayscale() {
        let news = prof(Family::NewsEvent);
        let plan = plan_finishing(&news, news.canvas, &Rgba::filled(800, 600, [9, 9, 9, 255]), false, false, "").unwrap();
        assert_eq!(plan.ops, vec![Op::FlattenOn { background: [0, 0, 0] }, Op::CoverResize { width: 397, height: 153 }, Op::Grayscale]);
    }

    #[test]
    fn report_events_always_become_cards_and_finished_cards_are_kept() {
        let p = prof(Family::ReportEvent);
        let sha = "ab".repeat(32);
        let plan = plan_finishing(&p, p.canvas, &Rgba::filled(300, 200, [9, 9, 9, 255]), true, false, &sha).unwrap();
        assert_eq!(plan.ops, vec![Op::FlattenOn { background: [0, 0, 0] }, Op::ReportCard { seed: 0xabababab }]);
        // an opaque 210x176 photo is not a card
        let opaque = Rgba::filled(210, 176, [9, 9, 9, 255]);
        assert!(!is_report_card(&opaque));
        assert!(plan_finishing(&p, p.canvas, &opaque, true, false, &sha).unwrap().ops.iter().any(|o| matches!(o, Op::ReportCard { .. })));
        // a finished card is accepted unchanged, but only when imported
        let card = crate::recipes::report_card::report_card(&Rgba::filled(300, 200, [90, 80, 70, 255]), &Default::default()).unwrap();
        assert!(is_report_card(&card));
        assert_eq!(plan_finishing(&p, p.canvas, &card, true, false, &sha).unwrap().ops, vec![Op::Conform { width: 210, height: 176 }]);
        assert!(plan_finishing(&p, p.canvas, &card, false, false, &sha).unwrap().ops.iter().any(|o| matches!(o, Op::ReportCard { .. })));
        assert_eq!(seed_from_sha("zz"), 0);
    }

    #[test]
    fn strips_are_built_by_the_chain_and_counters_only_accept_finished_art() {
        let logo = prof(Family::FactionLogo);
        let plan = plan_finishing(&logo, logo.canvas, &blob(), false, false, "").unwrap();
        assert!(matches!(plan.ops.last().unwrap(), Op::SelectedFrame { frames: 2 }));
        assert!(matches!(plan.ops[plan.ops.len() - 2], Op::FitSubject { width: 100, height: 100, .. }));
        let strip = plan_finishing(&logo, logo.canvas, &Rgba::new(200, 100), true, false, "").unwrap();
        assert_eq!(strip.ops, vec![Op::Conform { width: 200, height: 100 }]);
        let building = prof(Family::Building);
        let plan = plan_finishing(&building, building.canvas, &blob(), false, false, "").unwrap();
        assert!(matches!(plan.ops.last().unwrap(), Op::FitSubject { .. }), "frame 1 is fitted by the chain");
        assert_eq!(
            plan.finishers,
            vec![Finisher::SecondFrame, Finisher::DerivedCompanions],
            "frame 2 (the ruined landmark) comes from a second candidate, then the small strip is derived"
        );
    }

    #[test]
    fn unit_art_is_generated_from_a_silhouette_and_finished_strips_are_still_imported() {
        use crate::recipes::unit_art::UnitArtKind;
        let counter = prof(Family::CounterLarge);
        assert_eq!(recipe_of(&counter), Recipe::UnitCounter);
        assert_eq!(
            plan_finishing(&counter, counter.canvas, &Rgba::new(152, 42), true, false, "").unwrap().ops,
            vec![Op::Conform { width: 152, height: 42 }]
        );
        // generated art (or an imported drawing of another size) gets the glyph and the NATO box
        let opts = FinishOptions { counter_symbol: None, asset: Some("medium_armor".into()) };
        let plan = plan_finishing_with(&counter, counter.canvas, &blob(), false, false, "", &opts).unwrap();
        assert_eq!(
            plan.ops,
            vec![
                Op::UnitGlyph { kind: UnitArtKind::CounterLarge },
                Op::NatoFrame { kind: UnitArtKind::CounterLarge, symbol: "oval+bar_medium".into() }
            ]
        );
        let imported_drawing = plan_finishing(&counter, counter.canvas, &Rgba::new(100, 42), true, false, "").unwrap();
        assert!(matches!(imported_drawing.ops[0], Op::UnitGlyph { .. }));
        // a given symbol wins over the asset name, and a bad one is refused with the list
        let given = FinishOptions { counter_symbol: Some("infantry+support".into()), asset: Some("medium_armor".into()) };
        let plan = plan_finishing_with(&counter, counter.canvas, &blob(), false, false, "", &given).unwrap();
        assert_eq!(plan.ops[1], Op::NatoFrame { kind: UnitArtKind::CounterLarge, symbol: "cross+plus".into() });
        let bad = FinishOptions { counter_symbol: Some("laser".into()), asset: None };
        assert!(plan_finishing_with(&counter, counter.canvas, &blob(), false, false, "", &bad)
            .unwrap_err()
            .to_string()
            .contains("not a NATO symbol"));
        // the on-map counter also derives its text icon
        let small = prof(Family::CounterSmall);
        let plan = plan_finishing(&small, small.canvas, &blob(), false, false, "").unwrap();
        assert_eq!(plan.ops[0], Op::UnitGlyph { kind: UnitArtKind::CounterSmall });
        assert_eq!(plan.finishers, vec![Finisher::DerivedCompanions]);
        // the division symbol: glyph only, its small symbol derived
        let emblem = prof(Family::DivisionEmblem);
        assert_eq!(recipe_of(&emblem), Recipe::DivisionSymbol);
        let plan = plan_finishing(&emblem, emblem.canvas, &blob(), false, false, "").unwrap();
        assert_eq!(plan.ops, vec![Op::UnitGlyph { kind: UnitArtKind::DivisionEmblem }]);
        assert_eq!(plan.finishers, vec![Finisher::DerivedCompanions]);
        assert_eq!(
            plan_finishing(&emblem, emblem.canvas, &Rgba::new(76, 42), true, false, "").unwrap().ops,
            vec![Op::Conform { width: 76, height: 42 }]
        );
        // the processed strip passes the family's own checks
        let (strip, _) =
            crate::process::run_chain(&blob(), &plan_finishing(&counter, counter.canvas, &blob(), false, false, "").unwrap().ops).unwrap();
        let report = crate::validate::validate_image(&strip, &counter, &counter.imagegen.checks, &Default::default());
        assert!(report.findings.iter().all(|f| f.level != crate::validate::Level::Error), "{:?}", report.findings);
        // a profile whose canvas is not vanilla's frame cannot be drawn by the recipe
        let mut odd = counter.clone();
        odd.canvas = [80, 40];
        assert!(plan_finishing(&odd, odd.canvas, &blob(), false, false, "").is_err());
    }

    /// Selected-strip emblems sit as far inside frame 1 as vanilla's, whatever minimum margin the profile sets (the
    /// kit row says 2), and the finished strip passes its own margin and fill checks.
    #[test]
    fn selected_strips_fit_the_emblem_at_vanilla_size_and_pass_their_checks() {
        for (family, fit) in [(Family::AgencyLogo, 14), (Family::FactionLogo, 10)] {
            let mut p = prof(family);
            p.margin = 2;
            let plan = plan_finishing(&p, p.canvas, &blob(), false, false, "").unwrap();
            assert!(plan.ops.iter().any(|op| matches!(op, Op::FitSubject { margin, .. } if *margin == fit)), "{family:?}: {:?}", plan.ops);
            let (strip, _) = crate::process::run_chain(&blob(), &plan.ops).unwrap();
            let checks = ["margin".to_string(), "fill".to_string()];
            let report = crate::validate::validate_image(&strip, &p, &checks, &Default::default());
            assert!(report.findings.is_empty(), "{family:?}: {:?}", report.findings);
        }
    }

    #[test]
    fn finishers_are_named_for_the_families_that_need_them() {
        let c = |f: Family, img: &Rgba, imported: bool| plan_finishing(&prof(f), prof(f).canvas, img, imported, false, "").unwrap();
        assert_eq!(c(Family::Flag, &Rgba::filled(100, 60, [1, 1, 1, 255]), false).finishers, vec![Finisher::FlagLadder]);
        assert_eq!(c(Family::LeaderPortrait, &Rgba::filled(400, 500, [1, 1, 1, 255]), false).finishers, vec![Finisher::ExtractPortrait]);
        assert!(c(Family::LeaderPortrait, &Rgba::filled(156, 210, [1, 1, 1, 255]), true).finishers.is_empty());
        assert_eq!(c(Family::AdvisorCard, &Rgba::filled(300, 400, [1, 1, 1, 255]), false).finishers, vec![Finisher::AdvisorCard]);
        let subject = blob();
        let plan = c(Family::Achievement, &subject, false);
        assert_eq!(plan.finishers, vec![Finisher::AchievementTriplet]);
        assert!(matches!(plan.ops.last().unwrap(), Op::FitSubject { width: 64, height: 64, margin: 2, .. }));
    }

    fn record(profile: &Profile, steps: &[Op], finishers: &[&str], companions: &[&str], pixels: &str) -> ProcessedRecord {
        let recipe = recipe_of(profile);
        let mut prev = "src".to_string();
        let mut recs = Vec::new();
        for (i, op) in steps.iter().enumerate() {
            let out = format!("s{i}");
            recs.push(crate::process::StepRecord {
                op: op.clone(),
                input_sha256: prev.clone(),
                output_sha256: out.clone(),
                output_size: [1, 1],
                engine: "t".into(),
            });
            prev = out;
        }
        let fins: Vec<FinisherRecord> = finishers
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let out = format!("f{i}");
                let r = FinisherRecord { name: n.to_string(), input_sha256: prev.clone(), output_sha256: out.clone(), detail: serde_json::json!({}) };
                prev = out;
                r
            })
            .collect();
        let last = prev;
        ProcessedRecord {
            artifact_id: "f.a-v1-p1".into(),
            source_id: "f.a-v1".into(),
            asset_id: "f.a".into(),
            family: profile.family,
            run: 1,
            canvas: profile.canvas,
            path: "p.png".into(),
            sha256: "x".into(),
            pixels_sha256: if pixels.is_empty() { last } else { pixels.to_string() },
            size: [1, 1],
            steps: recs,
            profile: ProfileRef { id: profile.id.clone(), hash: "h".into(), kit_sha256: None },
            recipe: Some(RecipeRecord { id: recipe.id().into(), version: recipe.version(), finishers: fins }),
            companions: companions
                .iter()
                .map(|k| CompanionRecord {
                    key: k.to_string(),
                    path: format!("{k}.png"),
                    sha256: "c".into(),
                    pixels_sha256: "c".into(),
                    size: crate::lookup::derived_companion(profile.family, k).map_or([1, 1], |d| d.strip()),
                })
                .collect(),
        }
    }

    #[test]
    fn audit_accepts_complete_chains_and_names_what_is_missing() {
        let focus = prof(Family::Focus);
        let fit = Op::FitSubject { width: 100, height: 88, margin: 2, min_alpha: 16 };
        assert!(audit(&record(&focus, &[Op::TrimToSubject { min_alpha: 16 }, fit.clone()], &[], &[], ""), &focus).is_empty());
        assert!(audit(&record(&focus, &[Op::Conform { width: 100, height: 88 }], &[], &[], ""), &focus).is_empty());
        let missing = audit(&record(&focus, &[Op::TrimToSubject { min_alpha: 16 }], &[], &[], ""), &focus);
        assert!(missing.iter().any(|m| m.contains("fit_subject")), "{missing:?}");
        // no recipe at all
        let mut bare = record(&focus, std::slice::from_ref(&fit), &[], &[], "");
        bare.recipe = None;
        assert!(audit(&bare, &focus)[0].contains("no recipe"));
        // wrong recipe or version
        let mut wrong = record(&focus, &[fit], &[], &[], "");
        wrong.recipe.as_mut().unwrap().version = 9;
        assert!(audit(&wrong, &focus).iter().any(|m| m.contains("v9")));
        // grayscale families need grayscale
        let news = prof(Family::NewsEvent);
        let ops = [Op::FlattenOn { background: [0, 0, 0] }, Op::CoverResize { width: 397, height: 153 }];
        assert!(audit(&record(&news, &ops, &[], &[], ""), &news).iter().any(|m| m.contains("grayscale")));
        let mut full = ops.to_vec();
        full.push(Op::Grayscale);
        assert!(audit(&record(&news, &full, &[], &[], ""), &news).is_empty());
        // the report card needs its card step, a bare flatten is not enough
        let rep = prof(Family::ReportEvent);
        assert!(!audit(&record(&rep, &[Op::FlattenOn { background: [0, 0, 0] }], &[], &[], ""), &rep).is_empty());
        assert!(audit(&record(&rep, &[Op::FlattenOn { background: [0, 0, 0] }, Op::ReportCard { seed: 1 }], &[], &[], ""), &rep).is_empty());
    }

    #[test]
    fn audit_wants_every_stage_of_a_strip_one_conform_covers_both_and_companions_are_required() {
        let logo = prof(Family::FactionLogo);
        let fit = Op::FitSubject { width: 100, height: 100, margin: 10, min_alpha: 16 };
        let mini: &[&str] = &["miniature"];
        let derive: &[&str] = &["derived_companions"];
        assert!(audit(&record(&logo, &[fit.clone(), Op::SelectedFrame { frames: 2 }], derive, mini, ""), &logo).is_empty());
        assert!(!audit(&record(&logo, std::slice::from_ref(&fit), derive, mini, ""), &logo).is_empty(), "the frame step cannot be skipped");
        assert!(!audit(&record(&logo, &[Op::SelectedFrame { frames: 2 }], derive, mini, ""), &logo).is_empty(), "the fit cannot be skipped");
        assert!(audit(&record(&logo, &[Op::Conform { width: 200, height: 100 }], derive, mini, ""), &logo).is_empty());
        // the miniature the game looks up next to the logo is required, at 32x32
        let missing = audit(&record(&logo, &[fit, Op::SelectedFrame { frames: 2 }], &[], &[], ""), &logo);
        assert!(missing.iter().any(|m| m.contains("miniature")), "{missing:?}");
        let mut wrong = record(&logo, &[Op::Conform { width: 200, height: 100 }], derive, mini, "");
        wrong.companions[0].size = [16, 16];
        assert!(audit(&wrong, &logo).iter().any(|m| m.contains("32x32")));
        // the division emblem's small symbol and the on-map counter's text icon likewise
        let emblem = prof(Family::DivisionEmblem);
        let conform = [Op::Conform { width: 76, height: 42 }];
        assert!(audit(&record(&emblem, &conform, derive, &["small"], ""), &emblem).is_empty());
        let glyph = [Op::UnitGlyph { kind: crate::recipes::unit_art::UnitArtKind::DivisionEmblem }];
        assert!(audit(&record(&emblem, &glyph, derive, &["small"], ""), &emblem).is_empty());
        assert!(!audit(&record(&emblem, &conform, &[], &[], ""), &emblem).is_empty());
        let counter = prof(Family::CounterSmall);
        let conform = [Op::Conform { width: 60, height: 12 }];
        assert!(audit(&record(&counter, &conform, derive, &["text_icon"], ""), &counter).is_empty());
        let kind = crate::recipes::unit_art::UnitArtKind::CounterSmall;
        let generated = [Op::UnitGlyph { kind }, Op::NatoFrame { kind, symbol: "cross".into() }];
        assert!(audit(&record(&counter, &generated, derive, &["text_icon"], ""), &counter).is_empty());
        let no_box = audit(&record(&counter, &generated[..1], derive, &["text_icon"], ""), &counter);
        assert!(no_box.iter().any(|m| m.contains("nato_frame")), "the NATO box cannot be skipped: {no_box:?}");
        assert!(!audit(&record(&counter, &conform, &[], &[], ""), &counter).is_empty());
        assert_eq!(companion_keys_of(&counter, counter.canvas), vec!["text_icon"]);
        assert_eq!(companion_keys_of(&prof(Family::Achievement), [64, 64]), vec!["_grey", "_not_eligible"]);
        let ach = prof(Family::Achievement);
        let fit = Op::FitSubject { width: 64, height: 64, margin: 2, min_alpha: 16 };
        assert!(audit(&record(&ach, std::slice::from_ref(&fit), &["achievement_triplet"], &["_grey", "_not_eligible"], ""), &ach).is_empty());
        let no_comp = audit(&record(&ach, std::slice::from_ref(&fit), &["achievement_triplet"], &["_grey"], ""), &ach);
        assert!(no_comp.iter().any(|m| m.contains("_not_eligible")), "{no_comp:?}");
        assert!(!audit(&record(&ach, std::slice::from_ref(&fit), &[], &["_grey", "_not_eligible"], ""), &ach).is_empty());
        // a pixel hash that no stage produced
        let mismatch = audit(&record(&ach, &[fit], &["achievement_triplet"], &["_grey", "_not_eligible"], "other"), &ach);
        assert!(mismatch.iter().any(|m| m.contains("recorded pixels")), "{mismatch:?}");
    }

    #[test]
    fn bundle_hash_covers_companions_in_a_stable_order() {
        let c = |k: &str, s: &str| CompanionRecord { key: k.into(), path: "p".into(), sha256: s.into(), pixels_sha256: s.into(), size: [1, 1] };
        assert_eq!(bundle_sha256("m", &[]), "m");
        let a = bundle_sha256("m", &[c("_grey", "1"), c("_not_eligible", "2")]);
        assert_eq!(a, bundle_sha256("m", &[c("_not_eligible", "2"), c("_grey", "1")]));
        assert_ne!(a, bundle_sha256("m", &[c("_grey", "1"), c("_not_eligible", "3")]));
        assert_ne!(a, "m");
    }
}
