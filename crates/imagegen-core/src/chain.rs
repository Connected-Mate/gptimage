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
//! | `icon` v1 | transparent icons, technology, modifier icon, GUI texture | `fit_subject` (`conform` when finished art was imported at an accepted size), `grayscale` for grayscale families; technology adds `tech_overlay` when the caller asks for a tier numeral |
//! | `bop_side` v1 | balance of power side | `fit_subject` (the emblem in colour), `bop_stages`: the grey standard (the main image), the `_improved` companion and, when asked for, the `_bad` and `_bad_improved` companions |
//! | `raid` v1 | raid | `fit_subject`, `gold_gradient` (vanilla's gold ramp and black outline), or one `conform` for imported finished art |
//! | `doctrine` v2 | doctrine | `fit_subject`, `doctrine_look` (black units, pale outline and glow), or one `conform` for imported finished art |
//! | `vignette` v1 | agency upgrade | `round_vignette` (the disc in vanilla's muted palette with its darkened edge), `fit_subject`, or one `conform` for imported finished art |
//! | `picture` v1 | opaque pictures, news, super event | `flatten_on`, `cover_resize` or `conform`, `grayscale` for news |
//! | `report_card` v2 | report event | `report_card` or `conform` (an imported finished card) |
//! | `phase_card` v1 | operation picture | `phase_card` (the report card on vanilla's phase picture mask) or `conform` (an imported finished card) |
//! | `selected_strip` v1 | faction logo, agency logo | `fit_subject`, `selected_frame` (or one `conform` for an imported finished strip) |
//! | `landmark_strip` v1 | building | `fit_subject`, `second_frame`: the ruined state, a second candidate of the same asset (or one `conform`) |
//! | `unit_counter` v1 | battalion and on-map counters | `unit_glyph` (silhouette to vanilla's material), `nato_frame` (the NATO box as frame 2), or one `conform` for an imported finished strip that is already in vanilla's material; any other strip is drawn again from its frame 1 (`frame_of`, `unit_glyph`, `nato_frame`) |
//! | `division_symbol` v1 | division emblem | `unit_glyph`, or `conform` for an imported finished 76x42 symbol already in vanilla's material |
//! | `line_art` v1 | special project icon | `line_art` (any image to vanilla's pale line drawing) or `conform` for imported finished art, then `grayscale` |
//! | `portrait` v1 | the four portraits | `flatten_on`, `extract_portrait` (or `conform` for a finished 156x210) |
//! | `advisor_card` v1 | advisor card | `advisor_card` (or `conform` for a finished 65x67 card) |
//! | `achievement` v1 | achievement | `fit_subject`, `achievement_triplet` |
//! | `flag` v1 | flag | `flatten_on`, `flag_ladder` |
//! | `anim_glow` v1 | blend glow | `flatten_on`, `black_to_alpha`, `fit_subject`, `glow_frames` (or one `conform` for an imported finished strip) |
//! | `anim_keyframes` v1 | blend key frames | `key_frames`: the base and its `frame_candidates` fitted with one crop and scale (or one `conform`) |
//! | `anim_scroll` v1 | scroll | `flatten_on`, `cover_resize`, `scroll_frames` (or `flatten_on`, `conform`) |
//! | `anim_spin` v1 | spin | `spin_frames` (or one `conform`) |
//! | `upright_print` v1 | report event, operation picture with a style pack's `upright_print` | `fit_subject` (art with alpha) or `cover_resize` (opaque art), or `conform` for imported finished art; no tilt, mask, tone or shadow |
//! | `kept_unit_art` v2 | counters and the division emblem with a style pack's `keep_colours` | `fit_in_box` (or `area_fit`) into vanilla's glyph box in the art's own colours, `palette_snap` to the pack's measured palette (`snap_palette`), `nato_outline` for a counter, `edge_glow` (the pack's measured glow, `edge_glow`), or `conform` for imported finished art |
//! | `filled_icon` v1 | transparent icon families with a style pack's `fill_canvas` | `cover_resize` (or `area_cover`) to the whole canvas, or `conform` |
//!
//! A style pack's `crisp` switch keeps the recipe and swaps every Lanczos3 fit for its area-filter twin (`area_fit`,
//! `area_cover`) and drops the defringe erosion (see `pack::ProcessingSwitches`).
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

/// The box a medal is fitted into on its 56x84 frame: vanilla's medal is 33 px wide and 56 px tall in the median of
/// the 85 frames of its large strips (HOI4 1.19.4.1; margins 12 px at the sides, 13 at the top, 15 at the bottom), so
/// it spans 0.667 of the frame. Fitted edge to edge with the default 2 px margin it spanned 0.94 to 0.95.
pub const MEDAL_BOX: [u32; 2] = [33, 56];

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
    /// Operation phase picture: the report card on vanilla's phase mask.
    PhaseCard,
    /// Two frames, the second selected.
    SelectedStrip,
    /// Two frames: the landmark, then its ruined state (a second candidate of the same asset), as vanilla does.
    LandmarkStrip,
    /// A counter strip: the silhouette in vanilla's material, then the NATO box.
    UnitCounter,
    /// A division template symbol: the silhouette in vanilla's material.
    DivisionSymbol,
    /// A special project icon: any image redrawn as vanilla's thin pale lines on transparency.
    LineArt,
    /// A raid icon: the subject in vanilla's gold gradient with its black outline.
    Raid,
    /// A doctrine icon: black units with vanilla's pale outline and glow, warm accents kept.
    Doctrine,
    /// An agency upgrade: the subject cut to vanilla's round, muted vignette with its darkened edge.
    Vignette,
    /// Portraits.
    Portrait,
    /// Advisor card.
    AdvisorCard,
    /// Achievement triplet.
    Achievement,
    /// Flag ladder.
    Flag,
    /// Blend glow: light on black to alpha, then the dim frame derived.
    AnimGlow,
    /// Blend key frames: the base and its edits fitted together.
    AnimKeyframes,
    /// Scrolling strip.
    AnimScroll,
    /// Turning strip.
    AnimSpin,
    /// A style pack's upright print (`upright_print`): report or phase art fitted as it is, with no tilt, mask, tone
    /// or shadow.
    UprightPrint,
    /// A style pack's unit art in its own colours (`keep_colours`): fitted into vanilla's glyph box, and for a counter
    /// frame 2 is the NATO box as an outline in the glyph's colour.
    KeptUnitArt,
    /// A style pack's panel (`fill_canvas`): a transparent icon family whose art fills the whole canvas.
    FilledIcon,
    /// A balance of power side: the fitted emblem in colour, then its stages (standard grey, improved, optionally the cracked bad pair).
    BopSide,
}

impl Recipe {
    /// Stable id recorded with every processed artifact.
    pub fn id(self) -> &'static str {
        match self {
            Recipe::Icon => "icon",
            Recipe::Picture => "picture",
            Recipe::ReportCard => "report_card",
            Recipe::PhaseCard => "phase_card",
            Recipe::SelectedStrip => "selected_strip",
            Recipe::LandmarkStrip => "landmark_strip",
            Recipe::UnitCounter => "unit_counter",
            Recipe::DivisionSymbol => "division_symbol",
            Recipe::LineArt => "line_art",
            Recipe::Raid => "raid",
            Recipe::Doctrine => "doctrine",
            Recipe::Vignette => "vignette",
            Recipe::Portrait => "portrait",
            Recipe::AdvisorCard => "advisor_card",
            Recipe::Achievement => "achievement",
            Recipe::Flag => "flag",
            Recipe::AnimGlow => "anim_glow",
            Recipe::AnimKeyframes => "anim_keyframes",
            Recipe::AnimScroll => "anim_scroll",
            Recipe::AnimSpin => "anim_spin",
            Recipe::UprightPrint => "upright_print",
            Recipe::KeptUnitArt => "kept_unit_art",
            Recipe::FilledIcon => "filled_icon",
            Recipe::BopSide => "bop_side",
        }
    }

    /// Version, bumped whenever a recipe's output could change. The report card is at 2 (the shadow fix).
    pub fn version(self) -> u32 {
        match self {
            Recipe::ReportCard => crate::recipes::report_card::RECIPE_VERSION,
            Recipe::UnitCounter | Recipe::DivisionSymbol => crate::recipes::unit_art::RECIPE_VERSION,
            Recipe::LineArt => crate::recipes::line_art::RECIPE_VERSION,
            Recipe::Raid => crate::recipes::raid::RECIPE_VERSION,
            Recipe::Doctrine => crate::recipes::doctrine::RECIPE_VERSION,
            Recipe::Vignette => crate::recipes::vignette::RECIPE_VERSION,
            Recipe::AnimGlow | Recipe::AnimKeyframes | Recipe::AnimScroll | Recipe::AnimSpin => crate::animation::RECIPE_VERSION,
            // v2 (0.3.6): the pack's palette snap and edge glow
            Recipe::KeptUnitArt => 2,
            Recipe::BopSide => crate::recipes::bop::RECIPE_VERSION,
            _ => 1,
        }
    }

    /// Keys of the companion images the recipe produces next to the main one.
    pub fn companion_keys(self) -> &'static [&'static str] {
        match self {
            Recipe::Achievement => &["_grey", "_not_eligible"],
            Recipe::Flag => &["medium", "small"],
            Recipe::BopSide => &["_improved"],
            _ => &[],
        }
    }
}

/// The recipe of a family.
pub fn recipe_of(profile: &Profile) -> Recipe {
    let sw = profile.processing();
    match profile.family {
        Family::ReportEvent | Family::OperationPicture if sw.upright_print => Recipe::UprightPrint,
        Family::CounterLarge | Family::CounterSmall | Family::DivisionEmblem if sw.keep_colours => Recipe::KeptUnitArt,
        Family::ReportEvent => Recipe::ReportCard,
        Family::OperationPicture => Recipe::PhaseCard,
        Family::FactionLogo | Family::AgencyLogo => Recipe::SelectedStrip,
        Family::Building => Recipe::LandmarkStrip,
        Family::CounterLarge | Family::CounterSmall => Recipe::UnitCounter,
        Family::DivisionEmblem => Recipe::DivisionSymbol,
        Family::SpecialProjectIcon => Recipe::LineArt,
        Family::Raid => Recipe::Raid,
        Family::Doctrine => Recipe::Doctrine,
        Family::AgencyUpgrade => Recipe::Vignette,
        Family::LeaderPortrait | Family::CommanderPortrait | Family::OperativePortrait | Family::ScientistPortrait => Recipe::Portrait,
        Family::AdvisorCard => Recipe::AdvisorCard,
        Family::Achievement => Recipe::Achievement,
        Family::Flag => Recipe::Flag,
        Family::AnimBlendGlow => Recipe::AnimGlow,
        Family::AnimBlendKeyframes => Recipe::AnimKeyframes,
        Family::AnimScroll => Recipe::AnimScroll,
        Family::AnimSpin => Recipe::AnimSpin,
        Family::BopSide => Recipe::BopSide,
        _ if sw.fill_canvas => Recipe::FilledIcon,
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
    /// The frames of a blend key-frame strip: the base and its edits, fitted with one shared crop and scale.
    KeyFrames,
    /// The stages of a balance of power side made from the fitted colour emblem: the grey standard (the main image),
    /// the improved companion and, when asked for, the cracked `_bad` and `_bad_improved` companions.
    BopStages,
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
            Finisher::KeyFrames => "key_frames",
            Finisher::BopStages => "bop_stages",
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
    /// Why the plan differs from what the caller may expect, for example imported finished art that is drawn again
    /// because it is not in vanilla's material.
    pub notes: Vec<String>,
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

/// The cleaning and fit of a transparent subject into a `box_size` box centred on `canvas`. With `crisp` (a style
/// pack's switch) no defringe erosion runs and the fit uses the area filter (`area_fit`).
fn subject_fit_ops(img: &Rgba, canvas: [u32; 2], box_size: [u32; 2], keyed: bool, crisp: bool) -> Result<Vec<Op>> {
    let mut ops = transparent_icon_ops(img, canvas, keyed, 0)?;
    ops.pop();
    if crisp {
        ops.retain(|o| !matches!(o, Op::Defringe { .. }));
        ops.push(Op::AreaFit { width: canvas[0], height: canvas[1], box_width: box_size[0], box_height: box_size[1], min_alpha: 16 });
    } else {
        ops.push(Op::FitInBox { width: canvas[0], height: canvas[1], box_width: box_size[0], box_height: box_size[1], min_alpha: 16 });
    }
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
    /// Technology only: the tier numeral (and gem) to stamp where vanilla bakes it into the picture.
    pub overlay: Option<crate::recipes::tech_overlay::TechOverlay>,
    /// Kept-colours unit art with a style pack: the look measured on the pack's references for the family (the glyph
    /// size the fit scales to, and the palette and glow of the `snap_palette` and `edge_glow` switches). `None`
    /// without a pack or without readable references.
    pub pack_look: Option<PackFinish>,
}

/// A style pack's measured look on unit art, and where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct PackFinish {
    /// The pack.
    pub pack: String,
    /// SHA-256 of each reference it was measured from.
    pub references: Vec<String>,
    /// What was measured.
    pub look: crate::recipes::pack_look::PackLook,
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
    let crisp = profile.processing().crisp;
    let frames = profile.frames.max(1);
    let size = [img.width, img.height];
    let accepted_single = |s: [u32; 2]| profile.accepted.contains(&s) || s == canvas;
    let mut ops: Vec<Op> = Vec::new();
    let mut finishers = Vec::new();
    let mut notes = Vec::new();
    if opts.overlay.is_some() && profile.family != Family::Technology {
        return Err(Error::Invalid(format!("overlay (a tier numeral) is for technology pictures, not {}", profile.family)));
    }
    match recipe {
        Recipe::Icon | Recipe::BopSide => {
            if imported && accepted_single(size) {
                ops.push(Op::Conform { width: size[0], height: size[1] });
            } else if crisp {
                let inner = [canvas[0].saturating_sub(2 * profile.margin).max(1), canvas[1].saturating_sub(2 * profile.margin).max(1)];
                let box_size = if profile.family == Family::Medal && canvas == [56, 84] { MEDAL_BOX } else { inner };
                ops = subject_fit_ops(img, canvas, box_size, keyed, true)?;
            } else {
                ops = transparent_icon_ops(img, canvas, keyed, profile.margin)?;
                if profile.family == Family::Medal && canvas == [56, 84] {
                    if let Some(last) = ops.last_mut() {
                        *last = Op::FitInBox { width: 56, height: 84, box_width: MEDAL_BOX[0], box_height: MEDAL_BOX[1], min_alpha: 16 };
                    }
                }
            }
            if profile.imagegen.grayscale {
                ops.push(Op::Grayscale);
            }
            if let Some(overlay) = &opts.overlay {
                crate::recipes::tech_overlay::roman(overlay.tier)?;
                ops.push(Op::TechOverlay { overlay: overlay.clone() });
            }
        }
        Recipe::Raid | Recipe::Doctrine => {
            if imported && accepted_single(size) {
                ops.push(Op::Conform { width: size[0], height: size[1] });
            } else {
                // the doctrine glow reaches two pixels past the subject, so the subject sits that much further in
                let margin = if recipe == Recipe::Doctrine { profile.margin + crate::recipes::doctrine::GLOW_PX } else { profile.margin };
                ops = transparent_icon_ops(img, canvas, keyed, margin)?;
                ops.push(if recipe == Recipe::Raid { Op::GoldGradient } else { Op::DoctrineLook });
            }
        }
        Recipe::Vignette => {
            if imported && accepted_single(size) {
                ops.push(Op::Conform { width: size[0], height: size[1] });
            } else {
                // the vignette cuts its own disc and lays clear pixels over vanilla's backdrop, so an opaque picture is
                // as good a source as a disc on transparency
                if img.has_transparency() && edge_stats(img).1 > 0 {
                    ops.push(Op::Defringe { erosion: 1 });
                }
                ops.push(Op::TrimToSubject { min_alpha: 16 });
                ops.push(Op::RoundVignette);
                ops.push(Op::FitSubject { width: canvas[0], height: canvas[1], margin: profile.margin, min_alpha: 16 });
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
        Recipe::ReportCard | Recipe::PhaseCard => {
            if imported && is_report_card(img) {
                ops.push(Op::Conform { width: 210, height: 176 });
            } else {
                ops.push(Op::FlattenOn { background: FLATTEN_BACKGROUND });
                let seed = seed_from_sha(candidate_sha256);
                ops.push(if recipe == Recipe::ReportCard { Op::ReportCard { seed } } else { Op::PhaseCard { seed } });
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
            // finished art is kept only when it already is vanilla's material; anything else is drawn again from its
            // frame 1, so every route (generate, edit, import) ends in the unit art recipe
            let problems = if imported && finished { crate::recipes::unit_art::material_problems(img, kind) } else { Vec::new() };
            if imported && finished && problems.is_empty() {
                ops.push(Op::Conform { width: size[0], height: size[1] });
            } else {
                if imported && finished {
                    notes.push(format!(
                        "the imported {}x{} art is not in vanilla's {} material ({}), so it is drawn again from its frame 1 as a silhouette",
                        size[0],
                        size[1],
                        profile.family,
                        problems.join("; ")
                    ));
                    if recipe == Recipe::UnitCounter && frames > 1 {
                        ops.push(Op::FrameOf { frames, index: 0 });
                    }
                }
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
        Recipe::LineArt => {
            if imported && accepted_single(size) {
                ops.push(Op::Conform { width: size[0], height: size[1] });
            } else {
                // the recipe reads the subject from alpha or from a flat background itself, so an opaque source is
                // not refused here, and it redraws every line in white, so a key-coloured fringe needs no cleaning
                ops.push(Op::LineArt { width: canvas[0], height: canvas[1] });
            }
            if profile.imagegen.grayscale {
                ops.push(Op::Grayscale);
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
        Recipe::AnimGlow | Recipe::AnimKeyframes | Recipe::AnimScroll | Recipe::AnimSpin => {
            // imported art of a whole accepted strip is finished art; anything else is the base frame
            let finished = imported && size[0].is_multiple_of(frames) && accepted_single([size[0] / frames, size[1]]);
            match recipe {
                Recipe::AnimScroll => {
                    ops.push(Op::FlattenOn { background: FLATTEN_BACKGROUND });
                    if finished {
                        ops.push(Op::Conform { width: size[0], height: size[1] });
                    } else {
                        ops.push(Op::CoverResize { width: canvas[0], height: canvas[1] });
                        ops.push(Op::ScrollFrames { frames });
                    }
                }
                _ if finished => ops.push(Op::Conform { width: size[0], height: size[1] }),
                Recipe::AnimGlow => {
                    ops.push(Op::FlattenOn { background: FLATTEN_BACKGROUND });
                    ops.push(Op::BlackToAlpha);
                    ops.push(Op::FitSubject {
                        width: canvas[0],
                        height: canvas[1],
                        margin: profile.margin,
                        min_alpha: crate::animation::GLOW_MIN_ALPHA,
                    });
                    ops.push(Op::GlowFrames { frames, dim: crate::animation::GLOW_DIM_RATIO });
                }
                Recipe::AnimKeyframes => {
                    // the same refusal of an opaque source as every transparent family; the finisher fits the frames
                    transparent_icon_ops(img, canvas, keyed, profile.margin)?;
                    finishers.push(Finisher::KeyFrames);
                }
                _ => {
                    let mut clean = transparent_icon_ops(img, canvas, keyed, profile.margin)?;
                    // the turn fits the subject by its radius itself: keep only the cleaning steps
                    clean.retain(|o| !matches!(o, Op::FitSubject { .. }));
                    ops.extend(clean);
                    ops.push(Op::SpinFrames { width: canvas[0], height: canvas[1], margin: profile.margin, frames, degrees: 360.0 });
                }
            }
        }
        Recipe::UprightPrint => {
            // an upright print keeps the art as it is: no tilt, mask, tone or shadow; its own alpha stays
            if imported && accepted_single(size) {
                ops.push(Op::Conform { width: size[0], height: size[1] });
            } else if img.has_transparency() || keyed {
                ops.push(Op::TrimToSubject { min_alpha: 16 });
                ops.push(Op::FitSubject { width: canvas[0], height: canvas[1], margin: profile.margin, min_alpha: 16 });
            } else if crisp {
                ops.push(Op::AreaCover { width: canvas[0], height: canvas[1] });
            } else {
                ops.push(Op::CoverResize { width: canvas[0], height: canvas[1] });
            }
        }
        Recipe::KeptUnitArt => {
            let kind = unit_art_kind(profile.family).expect("unit families have a unit art kind");
            let counter = kind != crate::recipes::unit_art::UnitArtKind::DivisionEmblem;
            let finished =
                if counter { size[0].is_multiple_of(frames) && accepted_single([size[0] / frames, size[1]]) } else { accepted_single(size) };
            if imported && finished {
                ops.push(Op::Conform { width: size[0], height: size[1] });
            } else {
                let style = crate::recipes::unit_art::GlyphStyle::of(kind);
                if canvas != style.frame {
                    return Err(Error::Invalid(format!(
                        "{} art is fitted on vanilla's {}x{} frame, and this profile's canvas is {}x{}. Import finished art at the profile's size instead",
                        profile.family, style.frame[0], style.frame[1], canvas[0], canvas[1]
                    )));
                }
                // the glyph keeps its own colours and is fitted into vanilla's largest glyph box, centred on the frame.
                // TNO's 8 battalion glyphs span 26 to 62 by 18 to 29 px, so vanilla's 62x30 box (the 90th percentile
                // of vanilla's) is also the pack's envelope and stays the cap. With the pack's references the glyph is
                // scaled to their median size inside it, so a dense glyph is not drawn heavier than theirs
                let look = opts.pack_look.as_ref();
                let max_box = [style.max_box[0].round() as u32, style.max_box[1].round() as u32];
                let box_size = match look {
                    Some(f) if f.look.glyph_ink > 0.0 => crate::recipes::pack_look::ink_box(img, max_box, f.look.glyph_ink),
                    _ => max_box,
                };
                ops = subject_fit_ops(img, canvas, box_size, keyed, crisp)?;
                if let (Some(f), true) = (look, box_size != max_box) {
                    notes.push(format!(
                        "the glyph is fitted into a {}x{} box, inside the {}x{} glyph box, so it holds about the {:.0} opaque pixels of the style pack's references",
                        box_size[0], box_size[1], max_box[0], max_box[1], f.look.glyph_ink
                    ));
                }
                // the fit shrinks everything the trim keeps into the box, so a drawing that holds more than the unit's
                // glyph (its own NATO box beside it, a second element) ends as a sliver with the real box added to it
                if let Some(b) = img.alpha_bbox(16) {
                    if let Some(shape) = crate::recipes::unit_art::glyph_shape_problem(kind, true, b[2] - b[0], b[3] - b[1]) {
                        return Err(Error::refused(
                            "unit_art_holds_more_than_the_glyph",
                            format!(
                                "the art holds more than the glyph: after trimming, its subject is {shape}, so fitting it into the {}x{} glyph box would shrink the unit to a sliver. It probably draws its own NATO box or a second element beside the unit. Redraw it with only the unit's glyph, one object, and process it again",
                                max_box[0], max_box[1]
                            ),
                            RetryClass::Terminal,
                        ));
                    }
                }
                let sw = profile.processing();
                // the palette goes onto the glyph before the outline, which is drawn in the glyph's colour
                match look {
                    Some(f) if sw.snaps_palette() && !f.look.palette.is_empty() => ops.push(Op::PaletteSnap {
                        palette: f.look.palette.clone(),
                        max_distance: crate::recipes::pack_look::SNAP_DISTANCE,
                        pack: f.pack.clone(),
                        references: f.references.clone(),
                    }),
                    _ if sw.snaps_palette() => notes.push(
                        "snap_palette is on but the pack has no measurable references for the family, so the glyph keeps the model's shades"
                            .to_string(),
                    ),
                    _ => {}
                }
                if counter {
                    let (symbol, _) = counter_symbol(opts)?;
                    ops.push(Op::NatoOutline { kind, symbol });
                }
                match look {
                    Some(f) if sw.draws_edge_glow() && f.look.glow.len() == frames as usize && f.look.glow.iter().any(|g| g[0] >= 16) => {
                        ops.push(Op::EdgeGlow { frames, glow: f.look.glow.clone(), pack: f.pack.clone(), references: f.references.clone() })
                    }
                    _ => {}
                }
            }
        }
        Recipe::FilledIcon => {
            // a panel fills the canvas, so an opaque source is right and its own alpha (rounded corners) stays
            if imported && accepted_single(size) {
                ops.push(Op::Conform { width: size[0], height: size[1] });
            } else if crisp {
                ops.push(Op::AreaCover { width: canvas[0], height: canvas[1] });
            } else {
                ops.push(Op::CoverResize { width: canvas[0], height: canvas[1] });
            }
            if profile.imagegen.grayscale {
                ops.push(Op::Grayscale);
            }
            if let Some(overlay) = &opts.overlay {
                crate::recipes::tech_overlay::roman(overlay.tier)?;
                ops.push(Op::TechOverlay { overlay: overlay.clone() });
            }
        }
    }
    if recipe == Recipe::BopSide {
        finishers.push(Finisher::BopStages);
    }
    if !crate::lookup::derived_companions(profile.family, canvas).is_empty() {
        finishers.push(Finisher::DerivedCompanions);
    }
    Ok(ChainPlan { ops, finishers, notes })
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
    let counter = matches!(profile.family, Family::CounterLarge | Family::CounterSmall);
    let mut stages: Vec<Vec<&'static str>> = match recipe_of(profile) {
        Recipe::Icon if profile.processing().crisp => vec![vec!["area_fit", "conform"]],
        Recipe::Icon => vec![vec!["fit_subject", "fit_in_box", "conform"]],
        Recipe::BopSide if profile.processing().crisp => vec![vec!["area_fit", "conform"], vec!["bop_stages"]],
        Recipe::BopSide => vec![vec!["fit_subject", "fit_in_box", "conform"], vec!["bop_stages"]],
        Recipe::UprightPrint => vec![vec!["fit_subject", "cover_resize", "area_cover", "conform"]],
        Recipe::KeptUnitArt if counter => vec![vec!["fit_in_box", "area_fit", "conform"], vec!["nato_outline", "conform"]],
        Recipe::KeptUnitArt => vec![vec!["fit_in_box", "area_fit", "conform"]],
        Recipe::FilledIcon => vec![vec!["cover_resize", "area_cover", "conform"]],
        Recipe::Picture => vec![vec!["flatten_on"], vec!["cover_resize", "conform"]],
        Recipe::ReportCard => vec![vec!["report_card", "conform"]],
        Recipe::PhaseCard => vec![vec!["phase_card", "conform"]],
        Recipe::SelectedStrip => vec![vec!["fit_subject", "conform"], vec!["selected_frame", "conform"]],
        Recipe::LandmarkStrip => vec![vec!["fit_subject", "conform"], vec!["second_frame", "conform"]],
        Recipe::UnitCounter => vec![vec!["unit_glyph", "conform"], vec!["nato_frame", "conform"]],
        Recipe::DivisionSymbol => vec![vec!["unit_glyph", "conform"]],
        Recipe::LineArt => vec![vec!["line_art", "conform"]],
        Recipe::Raid => vec![vec!["fit_subject", "conform"], vec!["gold_gradient", "conform"]],
        Recipe::Doctrine => vec![vec!["fit_subject", "conform"], vec!["doctrine_look", "conform"]],
        Recipe::Vignette => vec![vec!["round_vignette", "conform"], vec!["fit_subject", "conform"]],
        Recipe::Portrait => vec![vec!["flatten_on"], vec!["extract_portrait", "conform"]],
        Recipe::AdvisorCard => vec![vec!["advisor_card", "conform"]],
        Recipe::Achievement => vec![vec!["fit_subject", "conform"], vec!["achievement_triplet"]],
        Recipe::Flag => vec![vec!["flatten_on"], vec!["flag_ladder"]],
        Recipe::AnimGlow => vec![vec!["black_to_alpha", "conform"], vec!["fit_subject", "conform"], vec!["glow_frames", "conform"]],
        Recipe::AnimKeyframes => vec![vec!["key_frames", "conform"]],
        Recipe::AnimScroll => vec![vec!["flatten_on"], vec!["cover_resize", "conform"], vec!["scroll_frames", "conform"]],
        Recipe::AnimSpin => vec![vec!["spin_frames", "conform"]],
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
    fn operation_pictures_become_phase_cards() {
        let p = prof(Family::OperationPicture);
        assert_eq!((recipe_of(&p).id(), recipe_of(&p).version()), ("phase_card", 1));
        let sha = "ab".repeat(32);
        let plan = plan_finishing(&p, p.canvas, &Rgba::filled(300, 200, [90, 80, 70, 255]), false, false, &sha).unwrap();
        assert_eq!(plan.ops, vec![Op::FlattenOn { background: [0, 0, 0] }, Op::PhaseCard { seed: 0xabababab }]);
        let (card, _) = crate::process::run_chain(&Rgba::filled(300, 200, [90, 80, 70, 255]), &plan.ops).unwrap();
        assert!(is_report_card(&card));
        // the card passes the card checks the family's row needs (size, checker, card_corners)
        let checks: Vec<String> = ["size", "checker", "card_corners"].iter().map(|s| s.to_string()).collect();
        let report = crate::validate::validate_image(&card, &p, &checks, &Default::default());
        assert!(report.findings.is_empty(), "{:?}", report.findings);
        assert_eq!(plan_finishing(&p, p.canvas, &card, true, false, &sha).unwrap().ops, vec![Op::Conform { width: 210, height: 176 }]);
        assert!(audit(&record(&p, &plan.ops, &[], &[], ""), &p).is_empty());
        assert!(!audit(&record(&p, &plan.ops[..1], &[], &[], ""), &p).is_empty());
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
        // a finished strip in vanilla's material is kept as it is
        let (finished, _) =
            crate::process::run_chain(&blob(), &plan_finishing(&counter, counter.canvas, &blob(), false, false, "").unwrap().ops).unwrap();
        let kept = plan_finishing(&counter, counter.canvas, &finished, true, false, "").unwrap();
        assert_eq!(kept.ops, vec![Op::Conform { width: 152, height: 42 }]);
        assert!(kept.notes.is_empty());
        // a blank strip of the right size is not finished art: it is drawn again from frame 1
        let blank = plan_finishing(&counter, counter.canvas, &Rgba::new(152, 42), true, false, "").unwrap();
        assert_eq!(blank.ops[0], Op::FrameOf { frames: 2, index: 0 });
        assert!(blank.notes[0].contains("drawn again"), "{:?}", blank.notes);
        // generated art (or an imported drawing of another size) gets the glyph and the NATO box
        let opts = FinishOptions { counter_symbol: None, asset: Some("medium_armor".into()), ..Default::default() };
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
        let given = FinishOptions { counter_symbol: Some("infantry+support".into()), asset: Some("medium_armor".into()), ..Default::default() };
        let plan = plan_finishing_with(&counter, counter.canvas, &blob(), false, false, "", &given).unwrap();
        assert_eq!(plan.ops[1], Op::NatoFrame { kind: UnitArtKind::CounterLarge, symbol: "cross+plus".into() });
        let bad = FinishOptions { counter_symbol: Some("laser".into()), asset: None, ..Default::default() };
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
        let (symbol, _) = crate::process::run_chain(&blob(), &plan.ops).unwrap();
        assert_eq!(plan_finishing(&emblem, emblem.canvas, &symbol, true, false, "").unwrap().ops, vec![Op::Conform { width: 76, height: 42 }]);
        assert_eq!(
            plan_finishing(&emblem, emblem.canvas, &Rgba::new(76, 42), true, false, "").unwrap().ops,
            vec![Op::UnitGlyph { kind: UnitArtKind::DivisionEmblem }]
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

    #[test]
    fn special_project_icons_are_redrawn_as_line_art_and_the_audit_wants_the_drawing() {
        let p = prof(Family::SpecialProjectIcon);
        assert_eq!(recipe_of(&p), Recipe::LineArt);
        assert_eq!((recipe_of(&p).id(), recipe_of(&p).version()), ("line_art", 1));
        // generated art, cut out or opaque on a flat background, is always redrawn, then kept grayscale
        let plan = plan_finishing(&p, p.canvas, &blob(), false, false, "").unwrap();
        assert_eq!(plan.ops, vec![Op::LineArt { width: 161, height: 98 }, Op::Grayscale]);
        let paper = Rgba::filled(300, 200, [20, 40, 90, 255]);
        assert_eq!(plan_finishing(&p, p.canvas, &paper, false, false, "").unwrap().ops[0], Op::LineArt { width: 161, height: 98 });
        // imported finished art at the accepted size is kept
        assert_eq!(plan_finishing(&p, p.canvas, &Rgba::new(161, 98), true, false, "").unwrap().ops[0], Op::Conform { width: 161, height: 98 });
        // the processed drawing passes the family's own checks
        let (out, _) = crate::process::run_chain(&blob(), &plan.ops).unwrap();
        let report = crate::validate::validate_image(&out, &p, &p.imagegen.checks, &Default::default());
        assert!(report.findings.iter().all(|f| f.level != crate::validate::Level::Error), "{:?}", report.findings);
        assert!(report.findings.iter().all(|f| f.code != "GFX-UNDERSIZED-SUBJECT"), "{:?}", report.findings);
        // the audit wants the drawing: a bare fit, the old icon recipe's stage, is not enough
        let line = [Op::LineArt { width: 161, height: 98 }, Op::Grayscale];
        assert!(audit(&record(&p, &line, &[], &[], ""), &p).is_empty());
        assert!(audit(&record(&p, &[Op::Conform { width: 161, height: 98 }, Op::Grayscale], &[], &[], ""), &p).is_empty());
        let fit = [Op::FitSubject { width: 161, height: 98, margin: 3, min_alpha: 16 }, Op::Grayscale];
        let missing = audit(&record(&p, &fit, &[], &[], ""), &p);
        assert!(missing.iter().any(|m| m.contains("line_art")), "{missing:?}");
        let no_grey = audit(&record(&p, &line[..1], &[], &[], ""), &p);
        assert!(no_grey.iter().any(|m| m.contains("grayscale")), "{no_grey:?}");
        // a record of the old icon recipe is refused by id
        let mut old = record(&p, &line, &[], &[], "");
        old.recipe.as_mut().unwrap().id = "icon".into();
        assert!(audit(&old, &p).iter().any(|m| m.contains("recipe icon v1")));
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

    fn fixture(name: &str) -> Rgba {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gallery").join(name);
        crate::pixels::decode(&std::fs::read(p).unwrap(), Some(name), Default::default()).unwrap().0
    }

    /// The gallery's counters and division emblem (hoi4-imagegen 0.2.0) were imported as finished art and conformed
    /// unchanged, so they never got vanilla's green material. Imported again now, each is drawn again from its frame 1
    /// and the result passes the family's checks, the material check included.
    #[test]
    fn imported_unit_art_that_is_not_vanillas_material_goes_through_the_unit_recipe() {
        for (name, family) in [
            ("combat_engineers-v2-p1.png", Family::CounterLarge),
            ("infantry_rifle-v1-p1.png", Family::CounterSmall),
            ("lightning_shield-v1-p1.png", Family::DivisionEmblem),
        ] {
            let p = prof(family);
            let img = fixture(name);
            let before = crate::validate::validate_image(&img, &p, &p.imagegen.checks, &Default::default());
            assert!(before.findings.iter().any(|f| f.code == crate::validate::UNIT_MATERIAL_CODE), "{name}: {:?}", before.findings);
            let canvas = [img.width / p.frames.max(1), img.height];
            let opts = FinishOptions { asset: Some("engineer".into()), ..Default::default() };
            let plan = plan_finishing_with(&p, canvas, &img, true, false, "", &opts).unwrap();
            assert!(plan.ops.iter().any(|o| matches!(o, Op::UnitGlyph { .. })), "{name}: {:?}", plan.ops);
            assert!(!plan.notes.is_empty());
            let (out, _) = crate::process::run_chain(&img, &plan.ops).unwrap();
            let after = crate::validate::validate_image(&out, &p, &p.imagegen.checks, &Default::default());
            assert!(after.findings.iter().all(|f| f.level != crate::validate::Level::Error), "{name}: {:?}", after.findings);
        }
    }

    #[test]
    fn raid_and_doctrine_icons_get_their_vanilla_look_and_the_audit_wants_it() {
        for (family, op, name) in [(Family::Raid, Op::GoldGradient, "gold_gradient"), (Family::Doctrine, Op::DoctrineLook, "doctrine_look")] {
            let p = prof(family);
            assert_eq!(recipe_of(&p).id(), if family == Family::Raid { "raid" } else { "doctrine" });
            let plan = plan_finishing(&p, p.canvas, &blob(), false, false, "").unwrap();
            assert_eq!(plan.ops.last().unwrap(), &op);
            assert!(matches!(plan.ops[plan.ops.len() - 2], Op::FitSubject { .. }));
            let (out, _) = crate::process::run_chain(&blob(), &plan.ops).unwrap();
            let report = crate::validate::validate_image(&out, &p, &p.imagegen.checks, &Default::default());
            assert!(report.findings.iter().all(|f| f.level != crate::validate::Level::Error), "{family}: {:?}", report.findings);
            let fit = Op::FitSubject { width: p.canvas[0], height: p.canvas[1], margin: p.margin, min_alpha: 16 };
            assert!(audit(&record(&p, &[fit.clone(), op.clone()], &[], &[], ""), &p).is_empty());
            let missing = audit(&record(&p, &[fit], &[], &[], ""), &p);
            assert!(missing.iter().any(|m| m.contains(name)), "{missing:?}");
            assert!(audit(&record(&p, &[Op::Conform { width: p.canvas[0], height: p.canvas[1] }], &[], &[], ""), &p).is_empty());
        }
        // the gallery's raid and doctrine art, as processed by 0.2.0, goes through the look
        let sabotage = fixture("sabotage-v1-p1.png");
        let raid = prof(Family::Raid);
        let (out, _) = crate::process::run_chain(&sabotage, &plan_finishing(&raid, raid.canvas, &sabotage, false, false, "").unwrap().ops).unwrap();
        let (b, a) = (crate::recipes::raid::raid_stats(&[&sabotage]), crate::recipes::raid::raid_stats(&[&out]));
        eprintln!("gallery sabotage: before {b:?}\n  after {a:?}");
        assert!(a.ratios[0] > 0.7 && a.ratios[0] < 0.95, "gold after: {a:?}");
        let report = crate::validate::validate_image(&out, &raid, &raid.imagegen.checks, &Default::default());
        assert!(report.findings.iter().all(|f| f.level != crate::validate::Level::Error), "{:?}", report.findings);
        let warfare = fixture("mobile_warfare-v1-p1.png");
        let doctrine = prof(Family::Doctrine);
        let (out, _) =
            crate::process::run_chain(&warfare, &plan_finishing(&doctrine, doctrine.canvas, &warfare, false, false, "").unwrap().ops).unwrap();
        let (b, a) = (crate::recipes::doctrine::doctrine_stats(&[&warfare]), crate::recipes::doctrine::doctrine_stats(&[&out]));
        eprintln!("gallery mobile_warfare: before {b:?}\n  after {a:?}");
        assert!(a.black_share > b.black_share + 0.2, "units turn black: {b:?} -> {a:?}");
    }

    /// Every transparent family's own processing keeps the margin its `margin` check reads (`profile.margin`), on a
    /// subject that fills the whole source: a dark one, and a warm accent, whose doctrine shadow reaches furthest. The
    /// doctrine fit leaves room for the glow ([`crate::recipes::doctrine::GLOW_PX`], which is the glow's computed
    /// reach). Only finished frames are measured: frame 1, and every frame of a strip the ops make whole, except the
    /// selected frame 2 whose glow runs to the edge as vanilla's does (the check skips it too). Families whose last
    /// stage is an engine finisher that fits the frames itself (blend key frames) have no fitted output here.
    #[test]
    fn every_familys_processing_keeps_the_margin_the_check_reads() {
        assert_eq!(crate::recipes::doctrine::glow_reach(), crate::recipes::doctrine::GLOW_PX);
        let mut checked = Vec::new();
        let set = ProfileSet::builtin();
        for p in set.all() {
            if p.imagegen.background != Background::Transparent || !p.imagegen.checks.iter().any(|c| c == "margin") || p.margin == 0 {
                continue;
            }
            let canvas = if p.canvas == [0, 0] { [64, 64] } else { p.canvas };
            for colour in [[60, 60, 60, 255], [230, 140, 20, 255]] {
                // a solid block in a clear 8 px border, at four times the canvas: trimmed, it fills the whole canvas
                let (w, h) = (canvas[0] * 4, canvas[1] * 4);
                let mut src = Rgba::new(w, h);
                for y in 8..h - 8 {
                    for x in 8..w - 8 {
                        src.set_px(x, y, colour);
                    }
                }
                let plan = plan_finishing(p, canvas, &src, false, false, "").unwrap();
                let (out, _) = crate::process::run_chain(&src, &plan.ops).unwrap_or_else(|e| panic!("{}: {e}", p.family));
                if out.height != canvas[1] || !out.width.is_multiple_of(canvas[0]) {
                    assert!(plan.finishers.contains(&Finisher::KeyFrames), "{}: {}x{} out of the ops", p.family, out.width, out.height);
                    continue;
                }
                let frames = out.split_frames(out.width / canvas[0]).unwrap();
                let selected = crate::recipes::frames::selected_fit_margin(p.family) > 0;
                for (i, f) in frames.iter().enumerate() {
                    if i > 0 && selected {
                        continue;
                    }
                    let Some(b) = f.alpha_bbox(16) else { continue };
                    let m = [b[0], b[1], f.width - b[2], f.height - b[3]];
                    assert!(
                        m.iter().all(|x| *x >= p.margin),
                        "{} frame {}: margins {m:?} under the checked minimum {} (colour {colour:?})",
                        p.family,
                        i + 1,
                        p.margin
                    );
                }
            }
            checked.push(p.family);
        }
        assert!(checked.contains(&Family::Doctrine) && checked.contains(&Family::AgencyUpgrade) && checked.len() > 20, "{checked:?}");
    }

    /// An agency upgrade runs the round vignette before the fit, from a disc on transparency or from an opaque
    /// picture, and the audit wants the vignette. Imported finished art of the canvas size is taken as it is.
    #[test]
    fn agency_upgrades_get_the_round_vignette_and_the_audit_wants_it() {
        let p = prof(Family::AgencyUpgrade);
        assert_eq!(recipe_of(&p).id(), "vignette");
        for src in [blob(), Rgba::filled(200, 160, [150, 90, 40, 255])] {
            let plan = plan_finishing(&p, p.canvas, &src, false, false, "").unwrap();
            let n = plan.ops.len();
            assert_eq!(plan.ops[n - 2], Op::RoundVignette, "{:?}", plan.ops);
            assert_eq!(plan.ops[n - 1], Op::FitSubject { width: 68, height: 68, margin: 8, min_alpha: 16 });
            let (out, _) = crate::process::run_chain(&src, &plan.ops).unwrap();
            let report = crate::validate::validate_image(&out, &p, &p.imagegen.checks, &Default::default());
            assert!(report.findings.iter().all(|f| f.level != crate::validate::Level::Error), "{:?}", report.findings);
        }
        let fit = Op::FitSubject { width: 68, height: 68, margin: 8, min_alpha: 16 };
        assert!(audit(&record(&p, &[Op::RoundVignette, fit.clone()], &[], &[], ""), &p).is_empty());
        let missing = audit(&record(&p, &[fit], &[], &[], ""), &p);
        assert!(missing.iter().any(|m| m.contains("round_vignette")), "{missing:?}");
        assert!(audit(&record(&p, &[Op::Conform { width: 68, height: 68 }], &[], &[], ""), &p).is_empty());
        let finished = Rgba::filled(68, 68, [10, 10, 10, 0]);
        assert_eq!(plan_finishing(&p, p.canvas, &finished, true, false, "").unwrap().ops, vec![Op::Conform { width: 68, height: 68 }]);
    }

    /// A three-quarter view (the 0.3.1 riveted cruiser tank) gets the side-view hint, and side views do not. The hint
    /// is a warning, so the review decides.
    #[test]
    fn a_three_quarter_technology_picture_gets_the_side_view_hint() {
        let tech = prof(Family::Technology);
        let hint = |name: &str| {
            let r = crate::validate::validate_image(&fixture(name), &tech, &tech.imagegen.checks, &Default::default());
            r.findings.into_iter().find(|f| f.code == crate::validate::SIDE_VIEW_CODE)
        };
        let three_quarter = hint("v031_riveted_cruiser_tank.png").expect("a side-view hint");
        assert_eq!(three_quarter.level, crate::validate::Level::Warning);
        assert!(hint("v031_sloped_armour_medium_tank.png").is_none());
        assert!(hint("heavy_tank-v1-p1.png").is_none());
        let rules = tech.imagegen.prompt.rules.join(" ");
        assert!(rules.contains("exactly from the side") && rules.contains("facing right") && rules.contains("no three-quarter view"), "{rules}");
    }

    #[test]
    fn a_technology_overlay_is_stamped_last_and_only_on_technology() {
        use crate::recipes::tech_overlay::TechOverlay;
        let tech = prof(Family::Technology);
        let overlay = TechOverlay { tier: 3, gem: false, position: None };
        let opts = FinishOptions { overlay: Some(overlay.clone()), ..Default::default() };
        let plan = plan_finishing_with(&tech, tech.canvas, &blob(), false, false, "", &opts).unwrap();
        assert_eq!(plan.ops.last().unwrap(), &Op::TechOverlay { overlay: overlay.clone() });
        let (out, _) = crate::process::run_chain(&blob(), &plan.ops).unwrap();
        assert_eq!((out.width, out.height), (146, 54));
        // the audit accepts the extra step
        let plain = plan_finishing(&tech, tech.canvas, &blob(), false, false, "").unwrap();
        assert!(audit(&record(&tech, &plan.ops, &[], &[], ""), &tech).is_empty());
        assert_eq!(plain.ops.len() + 1, plan.ops.len());
        // refused elsewhere, and a tier out of range is refused
        assert!(plan_finishing_with(&prof(Family::Focus), [100, 88], &blob(), false, false, "", &opts)
            .unwrap_err()
            .to_string()
            .contains("technology"));
        let bad = FinishOptions { overlay: Some(TechOverlay { tier: 11, gem: false, position: None }), ..Default::default() };
        assert!(plan_finishing_with(&tech, tech.canvas, &blob(), false, false, "", &bad).is_err());
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
            profile: ProfileRef { id: profile.id.clone(), hash: "h".into(), kit_sha256: None, style_pack: None },
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
    fn kept(f: Family) -> Profile {
        use crate::pack::{PackProfile, ProcessingSwitches};
        let mut p = prof(f);
        p.pack = Some(PackProfile { name: "t".into(), processing: ProcessingSwitches { keep_colours: true, crisp: true, ..Default::default() } });
        p
    }

    /// A transparent drawing of `w` x `h`: a filled block over the left `glyph_w` columns (the "vehicle") and, when
    /// `with_box`, an outlined box (the model's own NATO box) over the rest.
    fn glyph_and_box(w: u32, h: u32, glyph_w: u32, with_box: bool) -> Rgba {
        let mut img = Rgba::new(w, h);
        for y in h / 5..h * 4 / 5 {
            for x in 0..glyph_w {
                img.set_px(x, y, [30, 160, 150, 255]);
            }
        }
        if with_box {
            let x0 = glyph_w + w / 25;
            for y in 0..h {
                for x in x0..w {
                    if y < 6 || y >= h - 6 || x < x0 + 6 || x >= w - 6 {
                        img.set_px(x, y, [30, 160, 150, 255]);
                    }
                }
            }
        }
        img
    }

    #[test]
    fn kept_unit_art_refuses_a_source_with_its_own_nato_box_beside_the_glyph() {
        // the 0.3.4 bug: a 1552x359 strip (a vehicle and a drawn NATO box) was shrunk into the glyph box as a sliver
        for family in [Family::CounterLarge, Family::CounterSmall, Family::DivisionEmblem] {
            let p = kept(family);
            let src = glyph_and_box(1552, 359, 700, true);
            let opts = FinishOptions { asset: Some("tank".into()), ..Default::default() };
            let msg = plan_finishing_with(&p, p.canvas, &src, false, false, "", &opts).unwrap_err().to_string();
            assert!(msg.contains("holds more than the glyph"), "{family}: {msg}");
            assert!(msg.contains("NATO box") && msg.contains("Redraw it with only the unit's glyph"), "{family}: {msg}");
        }
        // the plan sees the image after a chroma key, so a keyed source is measured the same way
        let p = kept(Family::CounterLarge);
        let src = glyph_and_box(1552, 359, 700, true);
        assert!(plan_finishing_with(&p, p.canvas, &src, false, true, "", &Default::default()).is_err());
        // a tall column is refused too
        let mut column = Rgba::new(100, 600);
        for y in 0..600 {
            for x in 40..60 {
                column.set_px(x, y, [30, 160, 150, 255]);
            }
        }
        assert!(plan_finishing_with(&p, p.canvas, &column, false, false, "", &Default::default()).is_err());
    }

    #[test]
    fn kept_unit_art_fits_a_normal_glyph() {
        for (family, w, h) in [
            (Family::CounterLarge, 1000, 480),
            (Family::CounterSmall, 900, 380),
            (Family::DivisionEmblem, 700, 360),
            (Family::DivisionEmblem, 400, 520),
        ] {
            let p = kept(family);
            let src = glyph_and_box(w, h, w, false);
            let opts = FinishOptions { asset: Some("tank".into()), ..Default::default() };
            let plan = plan_finishing_with(&p, p.canvas, &src, false, false, "", &opts).unwrap();
            assert!(plan.ops.iter().any(|o| matches!(o, Op::AreaFit { .. })), "{family}");
        }
    }
    #[test]
    fn a_glyph_the_kept_recipe_accepts_passes_the_shape_check_after_the_fit() {
        // just inside the limit (4.1 against 4.13 for a large counter, the drawing is 0.6 of the height), where rounding could push the result over it
        for (family, w, h) in [
            (Family::CounterLarge, 1230, 500),
            (Family::CounterLarge, 1000, 480),
            (Family::CounterSmall, 1180, 500),
            (Family::DivisionEmblem, 1150, 500),
        ] {
            let p = kept(family);
            let src = glyph_and_box(w, h, w, false);
            let opts = FinishOptions { asset: Some("tank".into()), ..Default::default() };
            let Ok(plan) = plan_finishing_with(&p, p.canvas, &src, false, false, "", &opts) else {
                panic!("{family} {w}x{h} refused");
            };
            let (out, _) = crate::process::run_chain(&src, &plan.ops).unwrap();
            let report = crate::validate::validate_image(&out, &p, &["unit_glyph_shape".to_string()], &Default::default());
            assert!(report.findings.is_empty(), "{family} {w}x{h}: {:?}", report.findings);
        }
    }
}
