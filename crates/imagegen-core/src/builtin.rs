//! Built-in family table, measured from the installed game.
//!
//! Source: `data/vanilla-measurement.json` (HOI4 1.19.4.1, every texture of the surveyed folders, pixels
//! sampled; produced by `scripts/measure_families.py`). Canvas, accepted sizes, frames and the pixel class come
//! from that file, and a unit test fails when a family's row stops matching it, with explicit commented
//! exceptions where a row deliberately differs. These are defaults, not authority: the mod's own
//! `.tools/gfx/profiles.json` rows (and project overrides) win, so a mod keeps the sizes it chose.
//! Style cannot be measured from headers and stays a visual-review matter.
//!
//! Folder and file-name conventions of a mod are this tool's own: vanilla keeps several families in one
//! folder and names sprites freely, but an installed asset must be attributable to exactly one family, so every
//! family writes to its own leaf folder under `{feature_slug}`.

use crate::family::{Automation, Family, Group};
use crate::measurement;
use crate::profile::{Background, Container, ImagegenSpec, Profile, ProfileOrigin, PromptSpec, SpriteSpec, VanillaDefault, Variant};

struct Row {
    family: Family,
    /// Canvas of one frame.
    canvas: [u32; 2],
    accepted: &'static [[u32; 2]],
    margin: u32,
    frames: u32,
    /// What the measurement does not say: the reason for a deliberate difference, confidence, conventions.
    note: &'static str,
    automation: Automation,
    background: Background,
    container: Container,
    folder: &'static str,
    prefix: &'static str,
    alt_prefixes: &'static [&'static str],
    /// `.gfx` file, sprite name pattern and sprite companions. `None` for assets the engine loads by file name.
    sprite: Option<(&'static str, &'static str, &'static [&'static str])>,
    companion_files: &'static [&'static str],
    checks: &'static [&'static str],
    rules: &'static [&'static str],
    negative: &'static [&'static str],
    grayscale: bool,
}

const ICON_CHECKS: &[&str] = &["size", "alpha", "opaque_canvas", "fringe", "margin", "fill", "checker", "style", "dds"];
const FOCUS_CHECKS: &[&str] = &["size", "alpha", "opaque_canvas", "fringe", "margin", "fill", "checker", "ghost", "style", "dds"];
const PICTURE_CHECKS: &[&str] = &["size", "checker", "opaque", "style", "dds"];
const CARD_CHECKS: &[&str] = &["size", "checker", "style", "dds"];
const PORTRAIT_CHECKS: &[&str] = &["size", "checker", "opaque", "style", "dds"];
const FLAG_CHECKS: &[&str] = &["size", "opaque", "style", "tga"];
const UNIT_CHECKS: &[&str] = &["size", "alpha", "frames", "style", "dds"];
const INTERFACE_CHECKS: &[&str] = &["size", "alpha", "style", "dds"];
const SUPER_CHECKS: &[&str] = &["size", "checker", "opaque", "style", "dds"];

const ICON_RULES: &[&str] = &[
    "One centred subject with a strong, simple silhouette that stays readable at {w}x{h} pixels.",
    "Painterly, slightly aged texture in the style of Hearts of Iron IV interface art, with readable contrast.",
    "Real transparent background: no backdrop, no frame, no border, no plate behind the subject.",
    "Leave a small empty margin around the subject.",
];
const FOCUS_RULES: &[&str] = &[
    "A strong central symbol that supports the focus's story, with a clear silhouette and aged painterly detail.",
    "Readable contrast at {w}x{h} pixels.",
    "Real transparent background: no backdrop, no frame, no border.",
    "Leave a small empty margin around the subject.",
];
const GRAY_ICON_RULES: &[&str] = &[
    "One centred subject drawn in shades of grey only, readable at {w}x{h} pixels.",
    "Painterly, slightly aged texture in the style of Hearts of Iron IV interface art.",
    "Real transparent background: no backdrop, no frame, no border.",
    "Leave a small empty margin around the subject.",
];
const ICON_NEGATIVE: &[&str] = &["text", "letters or numbers", "watermark or signature", "a drawn checkerboard pattern", "a frame or border"];

const PICTURE_RULES: &[&str] =
    &["A complete scene that fills the whole frame, composed for a {w}x{h} crop.", "Period illustration look, painterly, with natural film grain."];
const REPORT_RULES: &[&str] = &[
    "A documentary-style scene that fills the whole frame, composed for a {w}x{h} crop.",
    "The report-card treatment (sepia, grain, paper border, tilt, shadow) is applied afterwards, so do not draw a border.",
];
const NEWS_RULES: &[&str] = &[
    "A black and white press photograph that fills the whole frame, composed for a wide {w}x{h} crop.",
    "Period newspaper look with natural grain.",
];
const PICTURE_NEGATIVE: &[&str] = &["text, captions or headlines", "watermark or signature", "a frame or border"];

const PORTRAIT_RULES: &[&str] = &[
    "A head-and-shoulders portrait in the painterly style of Hearts of Iron IV portraits, composed for a tall {w}x{h} crop.",
    "Period-appropriate clothing and a plain, slightly textured backdrop.",
];
const FLAG_RULES: &[&str] = &[
    "A flat national flag design with simple shapes that stay readable at 82x52 pixels and at 10x7 pixels.",
    "The flag fills the whole frame edge to edge.",
];
const FLAG_NEGATIVE: &[&str] = &["text", "a pole or cloth folds", "perspective", "a border"];
const UNIT_RULES: &[&str] = &[
    "Match the style of Hearts of Iron IV unit and faction art; one frame is {w}x{h} pixels and every frame shares that size.",
    "Real transparent background and a clear silhouette at native size.",
];
const INTERFACE_RULES: &[&str] = &["Match the style of Hearts of Iron IV interface art; the exact size is {w}x{h} pixels."];

/// Defaults of a group. A row overrides what differs with struct update syntax.
fn base(family: Family, canvas: [u32; 2], accepted: &'static [[u32; 2]], folder: &'static str, prefix: &'static str, gfx: &'static str) -> Row {
    let sprite = Some((gfx, "GFX_{prefix}{asset}", &[] as &'static [&'static str]));
    let mut row = Row {
        family,
        canvas,
        accepted,
        margin: if canvas[0] < 40 || canvas[1] < 40 { 1 } else { 2 },
        frames: 1,
        note: "",
        automation: Automation::Generate,
        background: Background::Transparent,
        container: Container::Dds,
        folder,
        prefix,
        alt_prefixes: &[],
        sprite,
        companion_files: &[],
        checks: ICON_CHECKS,
        rules: ICON_RULES,
        negative: ICON_NEGATIVE,
        grayscale: false,
    };
    match family.group() {
        Group::Icon => {}
        Group::Picture => {
            row.background = Background::Opaque;
            row.margin = 0;
            row.checks = PICTURE_CHECKS;
            row.rules = PICTURE_RULES;
            row.negative = PICTURE_NEGATIVE;
        }
        Group::Portrait => {
            row.background = Background::Opaque;
            row.margin = 0;
            row.automation = Automation::UserRunOnly;
            row.checks = PORTRAIT_CHECKS;
            row.rules = PORTRAIT_RULES;
            row.negative = PICTURE_NEGATIVE;
        }
        Group::Flag => {
            row.background = Background::Opaque;
            row.margin = 0;
            row.container = Container::Tga;
            row.checks = FLAG_CHECKS;
            row.rules = FLAG_RULES;
            row.negative = FLAG_NEGATIVE;
        }
        Group::Unit => {
            row.frames = 2;
            row.automation = Automation::ImportOnly;
            row.checks = UNIT_CHECKS;
            row.rules = UNIT_RULES;
        }
        Group::Interface => {
            row.checks = INTERFACE_CHECKS;
            row.rules = INTERFACE_RULES;
        }
    }
    row
}

fn rows() -> Vec<Row> {
    use Family::*;
    vec![
        // ---- icon group
        Row {
            margin: 2,
            alt_prefixes: &["goal_"],
            sprite: Some(("interface/{feature_slug}_goals.gfx", "GFX_{prefix}{asset}", &["shine"])),
            checks: FOCUS_CHECKS,
            rules: FOCUS_RULES,
            note: "GFX_focus_ is the majority sprite prefix and GFX_goal_ the original (goal_ stays acceptable). Every vanilla focus has a _shine companion sprite.",
            ..base(Focus, [100, 88], &[[100, 88], [102, 90], [99, 88], [98, 88]], "gfx/interface/goals/{feature_slug}", "focus_", "interface/{feature_slug}_goals.gfx")
        },
        Row {
            margin: 1,
            note: "Ideas share the GFX_idea_ namespace across four families, so the prefix cannot tell them apart: the consuming script slot and this folder do. Spirits are 60x68.",
            ..base(IdeaSpirit, [60, 68], &[[60, 68], [62, 70]], "gfx/interface/ideas/{feature_slug}", "idea_", "interface/{feature_slug}_ideas.gfx")
        },
        Row {
            margin: 1,
            note: "Industrial concerns and manufacturers are 64x64 (about 80 percent of their ideas); the dominant size of the shared idea folder is the spirit size, which is why this row is a documented exception in the drift test.",
            ..base(IdeaConcern, [64, 64], &[[64, 64]], "gfx/interface/ideas/{feature_slug}/concerns", "idea_", "interface/{feature_slug}_ideas.gfx")
        },
        Row {
            margin: 1,
            sprite: Some(("interface/officer_corp/{feature_slug}_spirits.gfx", "GFX_idea_{asset}", &[])),
            note: "Officer corps spirits are 45x45 and are registered as GFX_idea_* sprites, like ideas, but are a different surface.",
            ..base(OfficerCorpsSpirit, [45, 45], &[[45, 45]], "gfx/interface/officer_corp/spirits/{feature_slug}", "spirit_", "")
        },
        Row {
            margin: 1,
            note: "Missions use this family too. A bare `icon = x` in decision script resolves to GFX_decision_x, so the sprite must be named GFX_decision_<x>.",
            ..base(Decision, [33, 32], &[[33, 32], [38, 35], [38, 37], [38, 38], [33, 31]], "gfx/interface/decisions/{feature_slug}", "decision_", "interface/{feature_slug}_decisions.gfx")
        },
        Row {
            margin: 3,
            note: "Two vanilla category icons are 21-frame animations and are not generated here.",
            ..base(DecisionCategoryIcon, [52, 40], &[[52, 40]], "gfx/interface/decisions/{feature_slug}/category_icons", "decision_category_", "interface/{feature_slug}_decisions.gfx")
        },
        Row {
            margin: 1,
            note: "Dynamic modifiers use this family (GFX_modifiers_*), sometimes aliased to GFX_idea_*.",
            ..base(StateModifier, [32, 32], &[[32, 32]], "gfx/interface/state_modifiers/{feature_slug}", "modifiers_", "interface/{feature_slug}_state_modifiers.gfx")
        },
        Row {
            margin: 3,
            grayscale: true,
            rules: GRAY_ICON_RULES,
            note: "Special project icons are grayscale on transparent.",
            ..base(SpecialProjectIcon, [161, 98], &[[161, 98]], "gfx/interface/special_project/project_icons/{feature_slug}", "sp_", "interface/special_projects/{feature_slug}_projects.gfx")
        },
        Row {
            note: "Only the 32x32 raid icons are generated; the wide banner strips in the same folder are interface pieces.",
            ..base(Raid, [32, 32], &[[32, 32]], "gfx/interface/military_raids/{feature_slug}", "raid_", "interface/{feature_slug}_raids.gfx")
        },
        Row {
            note: "Vanilla sprite names are free-form (GFX_military_industrial_organization_*, GFX_mio_*); this tool names them GFX_mio_department_<asset>.",
            ..base(MioDepartment, [48, 48], &[[48, 48]], "gfx/interface/military_industrial_organization/department_icons/{feature_slug}", "mio_department_", "interface/military_industrial_organization/{feature_slug}_mio.gfx")
        },
        Row {
            note: "Trait icons are 48x48. The trait and policy folders are measured together; policies are the 60x60 files.",
            ..base(MioTrait, [48, 48], &[[48, 48]], "gfx/interface/military_industrial_organization/trait_icons/{feature_slug}", "mio_trait_", "interface/military_industrial_organization/{feature_slug}_mio.gfx")
        },
        Row {
            note: "Policy icons are the 60x60 files of the combined trait and policy measurement, which is why this row is a documented exception in the drift test.",
            ..base(MioPolicy, [60, 60], &[[60, 60]], "gfx/interface/military_industrial_organization/policy_icons/{feature_slug}", "mio_policy_", "interface/military_industrial_organization/{feature_slug}_mio.gfx")
        },
        Row {
            note: "Only the 72x88 side icons are generated; the multi-frame strips in the folder need separate frame art.",
            ..base(BopSide, [72, 88], &[[72, 88]], "gfx/interface/bop/{feature_slug}", "bop_", "interface/{feature_slug}_bop.gfx")
        },
        Row {
            note: "Single-frame 16x28 medal. Vanilla also ships 4, 7 and 2-frame medal strips selected by `frame = N`; those are not generated here. Script references are GFX_medal_icon_*.",
            ..base(Medal, [16, 28], &[[16, 28]], "gfx/interface/medals/{feature_slug}", "medal_icon_", "interface/{feature_slug}_medals.gfx")
        },
        Row {
            note: "Vanilla names these GFX_agency_*.",
            ..base(AgencyUpgrade, [68, 68], &[[68, 68]], "gfx/interface/operatives/icons/{feature_slug}", "agency_", "interface/{feature_slug}_agency_upgrades.gfx")
        },
        Row {
            note: "LOW CONFIDENCE. The operation folder mixes 210x176 pictures (operation_picture), 59x58 phase icons, 48x48 and 85x85 art. The icon family takes 59x58, which ties 210x176 for most files; confirm the intended surface in game before relying on it.",
            ..base(OperationIcon, [59, 58], &[[59, 58], [48, 48], [85, 85]], "gfx/interface/operations/{feature_slug}/icons", "phase_", "interface/{feature_slug}_operations.gfx")
        },
        Row {
            note: "Commander and personal traits are both 23x33 and both named GFX_trait_*; the script slot (trait group) tells them apart.",
            ..base(CommanderTrait, [23, 33], &[[23, 33]], "gfx/interface/traits/{feature_slug}", "trait_", "interface/{feature_slug}_traits.gfx")
        },
        Row {
            note: "See commander_trait: same size and naming, different script slot. Vanilla keeps them in traits/personal.",
            ..base(PersonalTrait, [23, 33], &[[23, 33]], "gfx/interface/traits/{feature_slug}/personal", "trait_", "interface/{feature_slug}_traits.gfx")
        },
        Row {
            note: "Only the 64x64 doctrine icons are generated; the wide 212x83 and banner art in the folder are interface pieces.",
            ..base(Doctrine, [64, 64], &[[64, 64]], "gfx/interface/doctrines/{feature_slug}", "doctrine_", "interface/{feature_slug}_doctrines.gfx")
        },
        // ---- picture group
        Row {
            accepted: &[[210, 176], [209, 176]],
            rules: REPORT_RULES,
            checks: CARD_CHECKS,
            note: "Vanilla report cards have transparent corners and a soft shadow (532 of 600 use one of four alpha masks). The pixels measured as transparent, but the card treatment (rotation, tone, shadow) is applied to a full picture by a later recipe, so this row stays opaque until that recipe runs and does not check for opacity.",
            ..base(ReportEvent, [210, 176], &[[210, 176], [209, 176]], "gfx/event_pictures/{feature_slug}", "report_event_", "interface/{feature_slug}_eventpictures.gfx")
        },
        Row {
            rules: NEWS_RULES,
            grayscale: true,
            note: "News pictures are opaque and black and white.",
            ..base(NewsEvent, [397, 153], &[[397, 153], [396, 153]], "gfx/event_pictures/{feature_slug}", "news_event_", "interface/{feature_slug}_eventpictures.gfx")
        },
        Row {
            note: "Two vanilla animated strips use noOfFrames = 21 and are not generated here. Vanilla names the files decision_cat_*.",
            ..base(DecisionCategoryPicture, [114, 101], &[[114, 101]], "gfx/interface/decisions/{feature_slug}/category_pictures", "decision_cat_", "interface/{feature_slug}_decisions.gfx")
        },
        Row {
            note: "LOW CONFIDENCE. 210x176 ties 59x58 as the most common size of the operation folder; the pixel class of this size alone was not measured, so opacity follows the picture convention. Vanilla names these GFX_operations_*.",
            ..base(OperationPicture, [210, 176], &[[210, 176]], "gfx/interface/operations/{feature_slug}/pictures", "operations_", "interface/{feature_slug}_operations.gfx")
        },
        Row {
            background: Background::Transparent,
            margin: 2,
            checks: ICON_CHECKS,
            rules: ICON_RULES,
            negative: ICON_NEGATIVE,
            sprite: Some(("interface/{feature_slug}_technologies.gfx", "GFX_{asset}_medium", &[])),
            note: "The engine binds technology art by name: GFX_<technology key>_medium, so the asset name is the technology key. 131x52 (341 files) is the other common size. Measured transparent, so it follows the icon chain.",
            ..base(Technology, [146, 54], &[[146, 54], [131, 52]], "gfx/interface/technologies/{feature_slug}", "", "interface/{feature_slug}_technologies.gfx")
        },
        Row {
            automation: Automation::ImportOnly,
            sprite: None,
            companion_files: &["_grey", "_not_eligible"],
            note: "Achievements are not registered in any .gfx: the engine loads gfx/achievements/<key>.dds plus the _grey and _not_eligible variants by name, so there is no feature folder. The triplet recipe is a separate step, so this family is import-only until it exists.",
            ..base(Achievement, [64, 64], &[[64, 64]], "gfx/achievements", "", "")
        },
        // ---- portrait group
        Row {
            companion_files: &["_small"],
            note: "Real people: only a ComfyUI endpoint the maintainer configured may generate (decision D3). Vanilla has a _small companion for some portraits.",
            ..base(LeaderPortrait, [156, 210], &[[156, 210]], "gfx/leaders/{feature_slug}", "portrait_", "interface/{feature_slug}_leader_portraits.gfx")
        },
        Row {
            companion_files: &["_small"],
            note: "Commanders share the leader measurement (one gfx/leaders folder) and the 156x210 portrait; the script slot is the difference.",
            ..base(CommanderPortrait, [156, 210], &[[156, 210]], "gfx/leaders/{feature_slug}/commanders", "portrait_", "interface/{feature_slug}_commander_portraits.gfx")
        },
        Row {
            companion_files: &["_small"],
            note: "Operative portraits (La Resistance) are 156x210 with a _small companion for 28 files.",
            ..base(OperativePortrait, [156, 210], &[[156, 210]], "gfx/leaders/{feature_slug}/operatives", "portrait_", "interface/{feature_slug}_operative_portraits.gfx")
        },
        Row {
            companion_files: &["_small"],
            note: "Scientist portraits are 156x210.",
            ..base(ScientistPortrait, [156, 210], &[[156, 210]], "gfx/leaders/{feature_slug}/scientists", "portrait_", "interface/{feature_slug}_scientist_portraits.gfx")
        },
        Row {
            background: Background::Transparent,
            automation: Automation::ImportOnly,
            margin: 0,
            checks: UNIT_CHECKS,
            frames: 1,
            sprite: Some(("interface/{feature_slug}_ideas.gfx", "GFX_idea_{asset}", &[])),
            note: "The card is built from an approved portrait by a recipe (template composite), not generated, so it is import-only. 65x67 with rounded transparent corners; named GFX_idea_<token> like ideas.",
            ..base(AdvisorCard, [65, 67], &[[65, 67]], "gfx/interface/ideas/{feature_slug}/advisors", "idea_", "")
        },
        // ---- flag
        Row {
            sprite: None,
            note: "One call writes the three sizes of one tag and ideology: 82x52, 41x26 and 10x7, as 32-bit TGA with a bottom-left origin. The file name is the lookup: gfx/flags/<TAG>[_ideology].tga.",
            ..base(Flag, [82, 52], &[[82, 52]], "gfx/flags", "", "")
        },
        // ---- unit group
        Row {
            sprite: Some(("interface/{feature_slug}_counters.gfx", "GFX_unit_{asset}_icon", &[])),
            note: "A strip of two 76x42 frames (152x42). Needs separate art for each frame and the exact vanilla reference, so it is import-only. Sprite pattern follows the measured GFX_unit_*_icon family.",
            ..base(CounterLarge, [76, 42], &[[76, 42]], "gfx/interface/counters/divisions_large/{feature_slug}", "unit_", "")
        },
        Row {
            sprite: Some(("interface/{feature_slug}_counters.gfx", "GFX_unit_{asset}_icon_small", &[])),
            note: "A strip of two 30x12 frames (60x12). Import-only like the large counter. The _icon_small sprite suffix is the measured companion pattern.",
            ..base(CounterSmall, [30, 12], &[[30, 12]], "gfx/interface/counters/divisions_small/{feature_slug}", "unit_", "")
        },
        Row {
            frames: 1,
            margin: 1,
            sprite: Some(("interface/{feature_slug}_counters.gfx", "GFX_div_{prefix}{asset}", &[])),
            note: "Single frame, 76x42 (the large template emblem). The small 30x12 emblems of the sibling folder are separate assets and are not generated. Vanilla names the files custom_* and the sprites GFX_div_*. Import-only: it needs an exact vanilla reference.",
            ..base(DivisionEmblem, [76, 42], &[[76, 42]], "gfx/interface/counters/division_templates_large/{feature_slug}", "custom_", "")
        },
        Row {
            sprite: Some(("interface/buildings/{feature_slug}_landmarks.gfx", "GFX_{prefix}{asset}", &[])),
            note: "Landmark buildings are two frames: a 64x64 frame (128x64 strip) or a 32x32 frame (64x32 strip).",
            ..base(Building, [64, 64], &[[64, 64], [32, 32]], "gfx/interface/buildings/{feature_slug}", "building_", "")
        },
        Row {
            sprite: Some(("interface/factions/{feature_slug}_factions.gfx", "GFX_{prefix}{asset}", &[])),
            note: "Two 100x100 frames (200x100 strip). The 32x32 single-frame small logo of the same folder is a separate sprite and is not accepted here.",
            ..base(FactionLogo, [100, 100], &[[100, 100]], "gfx/interface/factions/faction_logos/{feature_slug}", "faction_", "")
        },
        Row {
            margin: 2,
            sprite: Some(("interface/{feature_slug}_agencies.gfx", "GFX_intelligence_agency_logo_{asset}", &[])),
            note: "Two 117x119 frames (234x119 strip). 34 files are 233x119, which does not divide into two whole frames and is not accepted.",
            ..base(AgencyLogo, [117, 119], &[[117, 119]], "gfx/interface/operatives/agencies/{feature_slug}", "agency_logo_", "")
        },
        // ---- interface group
        Row {
            margin: 0,
            note: "Tooltip icons for modifiers (GFX_MODIFIER_* in vanilla; this tool writes the lowercase GFX_modifier_<asset>, which the game resolves case-insensitively).",
            ..base(ModifierStatIcon, [30, 20], &[[30, 20]], "gfx/interface/modifiers/{feature_slug}", "modifier_", "interface/{feature_slug}_modifiers.gfx")
        },
        Row {
            note: "General GUI textures have 856 distinct sizes in vanilla: the size comes from the consuming GUI element and the mod must define it (a canvas in the plan, or a gui_texture row in profiles.json).",
            ..base(GuiTexture, [0, 0], &[], "gfx/interface/{feature_slug}", "gui_", "interface/{feature_slug}_gui.gfx")
        },
        Row {
            background: Background::Opaque,
            margin: 0,
            checks: SUPER_CHECKS,
            rules: PICTURE_RULES,
            negative: PICTURE_NEGATIVE,
            note: "Super events have no vanilla texture: they are a mod pattern (GUI windows), so the mod defines the canvas.",
            ..base(SuperEvent, [0, 0], &[], "gfx/super_events/{feature_slug}", "super_event_", "interface/{feature_slug}_super_events.gfx")
        },
    ]
}

fn flag_variants() -> Vec<Variant> {
    vec![
        Variant { name: "normal".into(), canvas: [82, 52], folder: "gfx/flags".into() },
        Variant { name: "medium".into(), canvas: [41, 26], folder: "gfx/flags/medium".into() },
        Variant { name: "small".into(), canvas: [10, 7], folder: "gfx/flags/small".into() },
    ]
}

/// The rubric id of a family: one per group (officer spirits and every other icon get the icon rubric).
pub fn rubric_of(family: Family) -> String {
    format!("{}_v1", family.group().as_str())
}

/// The evidence text of a family: the measurement sentence, then what the measurement does not say.
fn source_of(r: &Row) -> String {
    let entries = measurement::entries_of(r.family);
    let (low, note) = match r.note.strip_prefix("LOW CONFIDENCE. ") {
        Some(n) => (true, n),
        None => (false, r.note),
    };
    let mut s = String::new();
    if low {
        s.push_str("LOW CONFIDENCE. ");
    }
    let data = measurement::data();
    match entries.first().and_then(|id| data.summary(id)) {
        Some(m) => {
            s.push_str(&m);
            for id in entries.iter().skip(1) {
                if let Some(more) = data.summary(id) {
                    s.push(' ');
                    s.push_str(&more);
                }
            }
        }
        None => s.push_str("Not measured in vanilla."),
    }
    if !note.is_empty() {
        s.push(' ');
        s.push_str(note);
    }
    s
}

/// The built-in profile list, one per family.
pub fn profiles() -> Vec<Profile> {
    rows()
        .into_iter()
        .map(|r| {
            let canvas = r.canvas;
            let source = source_of(&r);
            Profile {
                id: r.family.as_str().to_string(),
                family: r.family,
                canvas,
                accepted: r.accepted.to_vec(),
                margin: r.margin,
                frames: r.frames,
                source: source.clone(),
                origin: ProfileOrigin::Builtin,
                overridden: Vec::new(),
                vanilla: Some(VanillaDefault { canvas, source }),
                imagegen: ImagegenSpec {
                    automation: r.automation,
                    background: r.background,
                    container: r.container,
                    folder: r.folder.to_string(),
                    prefix: r.prefix.to_string(),
                    alt_prefixes: r.alt_prefixes.iter().map(|s| s.to_string()).collect(),
                    sprite: r.sprite.map(|(gfx_file, name, companions)| SpriteSpec {
                        gfx_file: gfx_file.to_string(),
                        name: name.to_string(),
                        companions: companions.iter().map(|s| s.to_string()).collect(),
                    }),
                    companion_files: r.companion_files.iter().map(|s| s.to_string()).collect(),
                    variants: if r.family == Family::Flag { flag_variants() } else { Vec::new() },
                    checks: r.checks.iter().map(|s| s.to_string()).collect(),
                    reference_dirs: vec![format!("references/{}", r.family.as_str())],
                    rubric: rubric_of(r.family),
                    prompt: PromptSpec {
                        rules: r.rules.iter().map(|s| s.to_string()).collect(),
                        negative: r.negative.iter().map(|s| s.to_string()).collect(),
                    },
                    grayscale: r.grayscale,
                },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measurement::{self as m, data};

    fn strip(p: &Profile) -> [u32; 2] {
        [p.canvas[0] * p.frames, p.canvas[1]]
    }

    fn by_family() -> std::collections::BTreeMap<Family, Profile> {
        profiles().into_iter().map(|p| (p.family, p)).collect()
    }

    /// Families whose whole-image size is NOT the dominant size of their measured entry, with the reason and
    /// the entry size they must instead appear at.
    ///
    /// * `idea_concern`: the shared idea folder is dominated by spirits (60x68); concerns are 64x64, which is the
    ///   dominant size of `idea_categories.industrial_concern` (checked below).
    /// * `mio_policy`: the combined trait and policy measurement is dominated by 48x48 traits; policies are the
    ///   60x60 files.
    /// * `operation_icon`: the operation folder ties 210x176 (pictures) with 59x58 (phase icons, 71 files each).
    /// * `counter_large`, `counter_small`: the strips (152x42 and 60x12) are not the most common image of the
    ///   shared counters folder, which also holds single-frame sprites (30x12 and 76x42).
    const NOT_DOMINANT: &[Family] = &[Family::IdeaConcern, Family::MioPolicy, Family::OperationIcon, Family::CounterLarge, Family::CounterSmall];

    #[test]
    fn every_family_has_a_row_and_a_source_that_cites_the_measurement() {
        let set = by_family();
        assert_eq!(set.len(), Family::ALL.len());
        for (f, p) in &set {
            assert!(p.source.len() > 40, "{f}");
            if m::entries_of(*f).is_empty() {
                assert!(p.source.starts_with("Not measured in vanilla"), "{f}: {}", p.source);
                assert!(p.canvas == [0, 0], "{f}: a family nobody measured has a mod-defined canvas");
            } else {
                assert!(p.source.contains("1.19.4.1"), "{f}: {}", p.source);
                assert!(p.source.contains("Measured on HOI4"), "{f}");
            }
        }
    }

    #[test]
    fn canvas_and_frames_equal_the_measured_dominant_values() {
        let meas = data();
        for (f, p) in by_family() {
            let Some(id) = m::entries_of(f).first() else { continue };
            let s = strip(&p);
            let (dom, dom_n) = meas.dominant(id).unwrap();
            if NOT_DOMINANT.contains(&f) {
                let n = meas.count_at(id, s).unwrap_or_else(|| panic!("{f}: {}x{} is not among the measured sizes of {id}", s[0], s[1]));
                assert!(n >= 20, "{f}: only {n} files at {}x{}", s[0], s[1]);
                if f == Family::OperationIcon {
                    assert_eq!(n, dom_n, "operation_icon must tie the dominant size (59x58 vs 210x176)");
                }
            } else {
                assert_eq!(s, dom, "{f}: strip {}x{} differs from the measured dominant {}x{} of {id}", s[0], s[1], dom[0], dom[1]);
            }
            // frames come from the registrations of the entry
            let frames = meas.frame_values(id);
            if !frames.is_empty() {
                let want = if p.frames == 1 { vec!["1(none)".to_string(), "1".to_string()] } else { vec![p.frames.to_string()] };
                assert!(frames.iter().any(|v| want.contains(v)), "{f}: frames {} not among registered {frames:?}", p.frames);
            }
        }
        // the two exceptions that rest on other measurements
        assert_eq!(meas.idea_category_dominant("industrial_concern"), Some([64, 64]));
        assert_eq!(by_family()[&Family::IdeaConcern].canvas, [64, 64]);
        assert_eq!(by_family()[&Family::AdvisorCard].canvas, meas.dominant("advisor_character_small").unwrap().0);
    }

    #[test]
    fn accepted_sizes_are_measured_sizes() {
        let meas = data();
        for (f, p) in by_family() {
            let Some(id) = m::entries_of(f).first() else { continue };
            for s in p.accepted_strips() {
                assert!(meas.count_at(id, s).is_some(), "{f}: accepted size {}x{} is not among the measured sizes of {id}", s[0], s[1]);
            }
        }
    }

    #[test]
    fn flag_variants_are_the_measured_flag_sizes() {
        let meas = data();
        let v = flag_variants();
        assert_eq!(v[0].canvas, meas.dominant("flag_large").unwrap().0);
        assert_eq!(v[1].canvas, meas.dominant("flag_medium").unwrap().0);
        assert_eq!(v[2].canvas, meas.dominant("flag_small").unwrap().0);
    }

    /// Families whose background differs from the measured pixel class, with the reason.
    /// * `report_event`: measured transparent (card corners and shadow); the card recipe adds that alpha to an
    ///   opaque picture later, so the family is opaque until it runs.
    /// * `operation_picture`: the operation entry mixes sizes and the pixel class of 210x176 alone was not measured.
    /// * `technology`: measured transparent, kept transparent (listed here only because it sits in the picture group).
    #[test]
    fn background_follows_the_measured_pixel_class() {
        let meas = data();
        for (f, p) in by_family() {
            let Some(id) = m::entries_of(f).first() else { continue };
            if matches!(f, Family::ReportEvent | Family::OperationPicture) {
                assert_eq!(p.imagegen.background, Background::Opaque, "{f}");
                continue;
            }
            let class = meas.alpha_class(id).unwrap();
            let want = if class.starts_with("transparent") { Background::Transparent } else { Background::Opaque };
            assert_eq!(p.imagegen.background, want, "{f}: measured '{class}'");
        }
    }

    #[test]
    fn grayscale_only_for_news_and_special_projects() {
        for (f, p) in by_family() {
            assert_eq!(p.imagegen.grayscale, matches!(f, Family::NewsEvent | Family::SpecialProjectIcon), "{f}");
        }
    }

    #[test]
    fn the_notes_the_maintainer_named_hold() {
        let set = by_family();
        let sprite = |f: Family| set[&f].imagegen.sprite.as_ref().map(|s| s.name.clone());
        assert_eq!(set[&Family::IdeaSpirit].canvas, [60, 68]);
        assert_eq!(set[&Family::AdvisorCard].canvas, [65, 67]);
        assert_eq!(set[&Family::IdeaConcern].canvas, [64, 64]);
        assert_eq!(set[&Family::OfficerCorpsSpirit].canvas, [45, 45]);
        assert_eq!(sprite(Family::OfficerCorpsSpirit).unwrap(), "GFX_idea_{asset}");
        assert_eq!(set[&Family::Decision].canvas, [33, 32]);
        assert_eq!(sprite(Family::Decision).unwrap(), "GFX_{prefix}{asset}");
        assert_eq!(set[&Family::Decision].imagegen.prefix, "decision_");
        assert_eq!(set[&Family::StateModifier].canvas, [32, 32]);
        assert!(set[&Family::StateModifier].imagegen.folder.starts_with("gfx/interface/state_modifiers"));
        assert_eq!(set[&Family::Focus].canvas, [100, 88]);
        assert_eq!(set[&Family::Focus].imagegen.sprite.as_ref().unwrap().companions, vec!["shine".to_string()]);
        assert_eq!(set[&Family::Focus].imagegen.alt_prefixes, vec!["goal_".to_string()]);
        let ach = &set[&Family::Achievement];
        assert_eq!(ach.imagegen.background, Background::Opaque);
        assert!(ach.imagegen.sprite.is_none());
        assert_eq!(ach.imagegen.folder, "gfx/achievements");
        assert_eq!(ach.imagegen.companion_files, vec!["_grey".to_string(), "_not_eligible".to_string()]);
        assert_eq!(sprite(Family::Technology).unwrap(), "GFX_{asset}_medium");
        assert_eq!(set[&Family::Technology].canvas, [146, 54]);
        assert!(set[&Family::Technology].accepted.contains(&[131, 52]));
        assert_eq!(set[&Family::Doctrine].canvas, [64, 64]);
        assert_eq!(set[&Family::SpecialProjectIcon].canvas, [161, 98]);
        assert_eq!(set[&Family::AgencyLogo].canvas, [117, 119]);
        assert_eq!(set[&Family::AgencyLogo].frames, 2);
        assert_eq!(strip(&set[&Family::FactionLogo]), [200, 100]);
        assert_eq!(strip(&set[&Family::Building]), [128, 64]);
        assert_eq!(set[&Family::Building].accepted_strips(), vec![[128, 64], [64, 32]]);
        assert_eq!(strip(&set[&Family::CounterLarge]), [152, 42]);
        assert_eq!(strip(&set[&Family::CounterSmall]), [60, 12]);
        assert_eq!(set[&Family::DivisionEmblem].canvas, [76, 42]);
        assert_eq!(set[&Family::DivisionEmblem].frames, 1);
        assert_eq!(set[&Family::Flag].imagegen.variants.len(), 3);
        assert_eq!(set[&Family::Flag].imagegen.container, Container::Tga);
        assert_eq!(set[&Family::LeaderPortrait].canvas, [156, 210]);
        assert_eq!(set[&Family::ReportEvent].canvas, [210, 176]);
        assert_eq!(set[&Family::NewsEvent].canvas, [397, 153]);
        assert!(set[&Family::GuiTexture].needs_mod_canvas() && set[&Family::SuperEvent].needs_mod_canvas());
    }

    #[test]
    fn automation_follows_what_the_chain_can_do() {
        for (f, p) in by_family() {
            let a = p.imagegen.automation;
            if p.frames > 1 {
                assert_eq!(a, Automation::ImportOnly, "{f}: the chain cannot assemble frame strips");
            }
            if f.group() == Group::Portrait && f != Family::AdvisorCard {
                assert_eq!(a, Automation::UserRunOnly, "{f}");
            }
        }
        let set = by_family();
        assert_eq!(set[&Family::AdvisorCard].imagegen.automation, Automation::ImportOnly);
        assert_eq!(set[&Family::Flag].imagegen.automation, Automation::Generate);
        assert_eq!(set[&Family::Focus].imagegen.automation, Automation::Generate);
        assert_eq!(set[&Family::DivisionEmblem].imagegen.automation, Automation::ImportOnly);
    }

    #[test]
    fn opaque_families_check_for_opacity_and_every_family_has_its_group_rubric() {
        for (f, p) in by_family() {
            assert_eq!(p.imagegen.rubric, format!("{}_v1", f.group().as_str()), "{f}");
            if p.imagegen.background == Background::Opaque && f != Family::ReportEvent {
                assert!(p.imagegen.checks.iter().any(|c| c == "opaque"), "{f} must check that no pixel is translucent");
            }
            assert!(p.imagegen.checks.iter().any(|c| c == "size"), "{f}");
        }
    }

    #[test]
    fn every_family_names_its_own_final_path() {
        // two families never write the same relative file for the same asset
        let mut seen = std::collections::BTreeMap::new();
        for (f, p) in by_family() {
            let folder = p.imagegen.folder.replace("{feature_slug}", "feat");
            let key = format!("{folder}/{}", p.imagegen.prefix);
            if let Some(other) = seen.insert(key.clone(), f) {
                panic!("{f} and {other} share the path pattern {key}");
            }
        }
    }
}
