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

use crate::animation::{self, AnimationSpec, Motion};
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
    /// Size the model is asked for when it is not the canvas.
    source_canvas: Option<[u32; 2]>,
}

const ICON_CHECKS: &[&str] = &["size", "alpha", "opaque_canvas", "fringe", "margin", "fill", "checker", "style", "dds"];
const FOCUS_CHECKS: &[&str] = &["size", "alpha", "opaque_canvas", "fringe", "margin", "fill", "checker", "ghost", "style", "dds"];
const PICTURE_CHECKS: &[&str] = &["size", "checker", "opaque", "style", "dds"];
const CARD_CHECKS: &[&str] = &["size", "checker", "card_corners", "style", "dds"];
const ACHIEVEMENT_CHECKS: &[&str] = &["size", "checker", "near_opaque", "style", "dds"];
const ADVISOR_CHECKS: &[&str] = &["size", "alpha", "checker", "card_corners", "style", "dds"];
const PORTRAIT_CHECKS: &[&str] = &["size", "checker", "opaque", "style", "dds"];
const FLAG_CHECKS: &[&str] = &["size", "opaque", "style", "tga"];
const UNIT_CHECKS: &[&str] = &["size", "alpha", "frames", "style", "dds"];
const INTERFACE_CHECKS: &[&str] = &["size", "alpha", "style", "dds"];
const SUPER_CHECKS: &[&str] = &["size", "checker", "opaque", "style", "dds"];
const ANIM_CHECKS: &[&str] = &["size", "alpha", "opaque_canvas", "margin", "frames", "animation", "checker", "style", "dds"];
const SCROLL_CHECKS: &[&str] = &["size", "opaque", "frames", "animation", "checker", "style", "dds"];

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
    "The report-card treatment (sepia, grain, tilt, shadow) is applied afterwards by the tool, so do not draw a border, a shadow or a tilt.",
];
const ACHIEVEMENT_RULES: &[&str] = &[
    "One original subject, centred, compact and readable at {w}x{h} pixels, with strong contrast and a simple silhouette.",
    "Real transparent background: no achievement frame, no square background, no red cross, no plate behind the subject.",
    "Painterly, slightly aged texture in the style of Hearts of Iron IV interface art.",
];
const ACHIEVEMENT_NEGATIVE: &[&str] =
    &["text", "letters or numbers", "watermark or signature", "a drawn checkerboard pattern", "a frame or border", "a red cross"];
const ADVISOR_RULES: &[&str] = &[
    "A head-and-shoulders portrait of one person in the painterly style of Hearts of Iron IV advisor portraits, face and shoulders centred, composed for a tall crop.",
    "Period-appropriate clothing and a plain, slightly textured backdrop. The card frame is added afterwards by the tool, so do not draw a frame or border.",
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
const FRAME_STRIP_RULES: &[&str] = &[
    "One centred emblem with a strong silhouette that reads at {w}x{h} pixels, drawn in the style of Hearts of Iron IV faction and interface art.",
    "Real transparent background: no backdrop, no frame, no plate. Leave clear space around the emblem: the tool adds the second frame (its highlight or its disabled look) itself.",
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
const GLOW_RULES: &[&str] = &[
    "A soft luminous glow, light only, drawn on a pure black background: the black becomes transparent, so everything that is not glow must be pure black.",
    "The glow is shaped for an interface element of {w}x{h} pixels (a ring, an outline or a halo around where the element sits), in the warm gold, red or green light of Hearts of Iron IV highlight glows.",
    "Draw the glow at full strength: the tool makes the dim frame of the pulse itself.",
];
const GLOW_NEGATIVE: &[&str] = &["text", "letters or numbers", "watermark or signature", "a grey or coloured background", "solid objects"];
const KEYFRAME_RULES: &[&str] = &[
    "One centred subject with a strong silhouette that reads at {w}x{h} pixels, in the painterly style of Hearts of Iron IV interface art.",
    "Real transparent background: no backdrop, no frame, no plate. Leave clear space around the subject: the next key frames are edits of this one and must fit the same frame.",
];
const SCROLL_RULES: &[&str] = &[
    "A seamless pattern that repeats from left to right and fills the whole {w}x{h} frame edge to edge, such as diagonal stripes, chevrons or a moving texture, in the muted palette of Hearts of Iron IV interface art.",
    "No single subject, no border and no frame: the pattern scrolls, so its left and right edges must continue each other.",
];
const SPIN_RULES: &[&str] = &[
    "One round subject that is meant to turn, such as a cog, a wheel, a propeller or a radar dish, centred and seen straight on, readable at {w}x{h} pixels.",
    "Real transparent background, nothing outside the round subject: the whole picture turns a full circle, so anything at the corners would swing in and out.",
];

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
        source_canvas: None,
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
        Group::Animation => {
            row.checks = ANIM_CHECKS;
            row.margin = 1;
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
            note: "Only the 32x32 raid icons are generated; the wide banner strips in the same folder are interface pieces. The 32x32 icon is the raid's map icon, which the game looks up as GFX_raid_type_icon_<raid type> (24 of the 25 vanilla ones are 32x32), so the asset name is the raid type key, or the raid sets custom_map_icon to the sprite.",
            ..base(Raid, [32, 32], &[[32, 32]], "gfx/interface/military_raids/{feature_slug}", "raid_type_icon_", "interface/{feature_slug}_raids.gfx")
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
            sprite: Some(("interface/{feature_slug}_medals.gfx", "GFX_medal_icon_{asset}_large", &[])),
            note: "A unit medal: script names GFX_medal_icon_<x> with frame = N, and every vanilla GFX_medal_icon_* has a GFX_medal_icon_*_large twin at twice the size (28x42 and 56x84 frames). This tool makes one 56x84 frame registered as the _large sprite and derives the 28x42 GFX_medal_icon_<asset> from it, the way vanilla's small strips are its large ones downscaled; script uses frame = 1. The measured dominant 16x28 files are older medal sprites that no vanilla script or GUI names.",
            ..base(Medal, [56, 84], &[[56, 84]], "gfx/interface/medals/{feature_slug}", "medal_icon_", "interface/{feature_slug}_medals.gfx")
        },
        Row {
            note: "Vanilla names these GFX_agency_*.",
            ..base(AgencyUpgrade, [68, 68], &[[68, 68]], "gfx/interface/operatives/icons/{feature_slug}", "agency_", "interface/{feature_slug}_agency_upgrades.gfx")
        },
        Row {
            sprite: Some(("interface/{feature_slug}_operations.gfx", "GFX_phase_{asset}_small", &[])),
            note: "The 59x58 operation phase icon: an operation phase's icon names it in full, and all 73 vanilla ones are GFX_phase_<x>_small at 59x58, beside the phase's 210x176 picture GFX_phase_<x> (operation_picture). The folder also holds 48x48 and 85x85 operation icons.",
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
            note: "Vanilla report cards have transparent corners and a soft shadow (532 of 600 use one of four alpha masks). The model draws an opaque picture; the report_card step (tone, grain, tilt, shadow) always turns it into a card with transparent corners, so the installed art has card alpha and is checked for transparent corners, never for opacity.",
            ..base(ReportEvent, [210, 176], &[[210, 176], [209, 176]], "gfx/event_pictures/{feature_slug}", "report_event_", "interface/{feature_slug}_eventpictures.gfx")
        },
        Row {
            rules: NEWS_RULES,
            grayscale: true,
            note: "News pictures are opaque and black and white.",
            ..base(NewsEvent, [397, 153], &[[397, 153], [396, 153]], "gfx/event_pictures/{feature_slug}", "news_event_", "interface/{feature_slug}_eventpictures.gfx")
        },
        Row {
            note: "Two vanilla animated strips use noOfFrames = 21 and are not generated here. A category's picture = x resolves to GFX_decision_cat_picture_x (106 of the 130 vanilla pictures use that name), so the sprite carries that prefix and script may write the short key.",
            ..base(DecisionCategoryPicture, [114, 101], &[[114, 101]], "gfx/interface/decisions/{feature_slug}/category_pictures", "decision_cat_picture_", "interface/{feature_slug}_decisions.gfx")
        },
        Row {
            note: "LOW CONFIDENCE. The 210x176 operation phase picture: a phase's picture names it in full, and all 71 vanilla ones are GFX_phase_<x> (GFX_operations_* are the 85x85 and 48x48 operation icons). The pixel class of this size alone was not measured, so opacity follows the picture convention.",
            ..base(OperationPicture, [210, 176], &[[210, 176]], "gfx/interface/operations/{feature_slug}/pictures", "phase_", "interface/{feature_slug}_operations.gfx")
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
            background: Background::Transparent,
            margin: 2,
            sprite: None,
            companion_files: &["_grey", "_not_eligible"],
            checks: ACHIEVEMENT_CHECKS,
            rules: ACHIEVEMENT_RULES,
            negative: ACHIEVEMENT_NEGATIVE,
            note: "Achievements are not registered in any .gfx: the engine loads gfx/achievements/<key>.dds plus the _grey and _not_eligible variants by name, so there is no feature folder. The model draws one transparent subject layer; the tool fits it to 64x64, derives the grey state (grayscale), the not-eligible state (grey with the red cross) and puts the kit's template backgrounds beneath all three. Installed art has alpha 254 or 255 because the templates do.",
            ..base(Achievement, [64, 64], &[[64, 64]], "gfx/achievements", "", "")
        },
        // ---- portrait group
        Row {
            companion_files: &["_small"],
            note: "Real people are the user's responsibility: the plan warns about likeness. Vanilla has a _small companion for some portraits, which this tool does not make.",
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
            background: Background::Opaque,
            margin: 0,
            checks: ADVISOR_CHECKS,
            rules: ADVISOR_RULES,
            source_canvas: Some([156, 210]),
            frames: 1,
            sprite: Some(("interface/{feature_slug}_ideas.gfx", "GFX_idea_{asset}", &[])),
            note: "The model draws a head-and-shoulders portrait; the tool composites it into the kit's advisor frame template (65x67 with transparent corners). The card has transparent corners although the source is opaque. Named GFX_idea_<token> like ideas.",
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
            sprite: Some(("interface/{feature_slug}_counters.gfx", "GFX_unit_{asset}_icon_medium", &[])),
            note: "The battalion icon: a strip of two 76x42 frames (152x42, noOfFrames = 2). Needs separate art for each frame and the exact vanilla reference, so it is import-only. The game looks it up as GFX_unit_<sub-unit key>_icon_medium, so the asset name is the sub-unit key (not its sprite key).",
            ..base(CounterLarge, [76, 42], &[[76, 42]], "gfx/interface/counters/divisions_large/{feature_slug}", "unit_", "")
        },
        Row {
            sprite: Some(("interface/{feature_slug}_counters.gfx", "GFX_unit_{asset}_icon_medium_white", &[])),
            note: "The on-map counter: a strip of two 30x12 frames (60x12, noOfFrames = 2), light grey because the game tints it. Import-only like the large counter. The game looks it up as GFX_unit_<sub-unit key>_icon_medium_white; the tooltip text icon GFX_unit_<sub-unit key>_icon_small (60x12, 2 frames, under gfx/texticons) is derived from it: frame 1 tinted green as vanilla's are, frame 2 kept.",
            ..base(CounterSmall, [30, 12], &[[30, 12]], "gfx/interface/counters/divisions_small/{feature_slug}", "onmap_unit_", "")
        },
        Row {
            frames: 1,
            margin: 1,
            sprite: Some(("interface/{feature_slug}_counters.gfx", "GFX_div_templ_{n}_large", &[])),
            note: "Single frame, 76x42 (the large division template symbol). The game offers the symbols as GFX_div_templ_<n>_large with GFX_div_templ_<n>_small (30x12); vanilla uses 0-43 and 65-121. Install picks n, the next number after vanilla's 121 and every number the mod or an earlier install already uses, records it, and derives the grey 30x12 small symbol. Import-only: it needs an exact vanilla reference.",
            ..base(DivisionEmblem, [76, 42], &[[76, 42]], "gfx/interface/counters/division_templates_large/{feature_slug}", "custom_", "")
        },
        Row {
            automation: Automation::Generate,
            rules: FRAME_STRIP_RULES,
            sprite: Some(("interface/buildings/{feature_slug}_landmarks.gfx", "GFX_{asset}_icon", &[])),
            note: "Landmark buildings are two frames: a 64x64 frame (128x64 strip, the raid window's GFX_<x>_icon) or a 32x32 frame (64x32 strip, the province window's GFX_<x>_icon_small, which the building's special_icon names). A 64x64 landmark also gets the small strip, halved frame by frame; one made at 32x32 is the small sprite alone. Frame 2 is the ruined state, a second candidate of the same asset.",
            ..base(Building, [64, 64], &[[64, 64], [32, 32]], "gfx/interface/buildings/{feature_slug}", "building_", "")
        },
        Row {
            automation: Automation::Generate,
            rules: FRAME_STRIP_RULES,
            sprite: Some(("interface/factions/{feature_slug}_factions.gfx", "GFX_{prefix}{asset}", &[])),
            note: "Two 100x100 frames (200x100 strip). Frame 2 is the selected state (cream stroke and dark-red glow, fitted to 31 vanilla strips) which needs room around the emblem: frame 1 is fitted 10 px inside its frame, the vanilla median, and the glow brings frame 2's margin down to about 2, the minimum checked. The game also loads <sprite>_miniature (32x32, all 31 vanilla logos have one), which is derived from frame 1. Vanilla logos are GFX_faction_logo_<x>.",
            ..base(FactionLogo, [100, 100], &[[100, 100]], "gfx/interface/factions/faction_logos/{feature_slug}", "faction_logo_", "")
        },
        Row {
            automation: Automation::Generate,
            rules: FRAME_STRIP_RULES,
            sprite: Some(("interface/{feature_slug}_agencies.gfx", "GFX_intelligence_agency_logo_{asset}", &[])),
            note: "Two 117x119 frames (234x119 strip). Frame 2 is the selected state, measured on the 37 vanilla 234x119 strips with the same glow profile as the faction logos. Frame 1 is fitted 14 px inside its frame (the vanilla median, an emblem of about 89 px), and the glow brings frame 2's margin down to about 2, the minimum checked. 34 files are 233x119, which does not divide into two whole frames and is not accepted.",
            ..base(AgencyLogo, [117, 119], &[[117, 119]], "gfx/interface/operatives/agencies/{feature_slug}", "agency_logo_", "")
        },
        // ---- interface group
        Row {
            margin: 0,
            sprite: Some(("interface/{feature_slug}_modifiers.gfx", "GFX_MODIFIER_{ASSET}", &[])),
            note: "Modifier icons that GUI elements name in full (spriteType = GFX_MODIFIER_<KEY>); the engine builds no such name. The sprite keeps vanilla's exact case, GFX_MODIFIER_ and the asset name in upper case, like all 159 vanilla ones: nothing in vanilla proves the lookup ignores case.",
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
        // ---- animation group: the canvas is a default; a plan may name the size of the element the strip sits on
        Row {
            frames: 2,
            rules: GLOW_RULES,
            negative: GLOW_NEGATIVE,
            note: "Two frames that the engine cross-fades (effectFile buttonstate_blendframes.lua, animation_rate_fps 1, looping, play_on_show, alwaystransparent), the way 70 vanilla glows pulse. Vanilla glow sizes follow the element they sit on (47 sizes over 66 textures), so a plan may pass its own canvas. The model draws the bright glow on black, which becomes alpha; frame 1 is the same glow at 0.17 of its alpha, the vanilla median.",
            ..base(AnimBlendGlow, [37, 49], &[[37, 49]], "gfx/interface/animated/{feature_slug}/glows", "glow_", "interface/{feature_slug}_animated.gfx")
        },
        Row {
            frames: 3,
            margin: 2,
            rules: KEYFRAME_RULES,
            note: "Three key frames that the engine cross-fades (buttonstate_blendframes.lua at 1 fps, like the ongoing-focus and spirit glows). Frame 1 is generated; frames 2 and 3 are hoi4.image_edit candidates of it, passed to processing as frame_candidates, and all three are fitted with one shared crop and scale.",
            ..base(AnimBlendKeyframes, [141, 115], &[[141, 115]], "gfx/interface/animated/{feature_slug}/keyframes", "anim_", "interface/{feature_slug}_animated.gfx")
        },
        Row {
            frames: 9,
            margin: 0,
            background: Background::Opaque,
            checks: SCROLL_CHECKS,
            rules: SCROLL_RULES,
            negative: PICTURE_NEGATIVE,
            note: "Nine frames at 15 fps with loadType INGAME and transparencecheck, the currently-researching highlight of 38 vanilla sprites (72x72 is the frame 16 of them use). The picture is made seamless across its width and moves right by a ninth of it per frame, so a tenth frame would be the first again. Vanilla panels keep a static border with rounded transparent corners around the pattern; this family scrolls the whole frame, so it is opaque.",
            ..base(AnimScroll, [72, 72], &[[72, 72]], "gfx/interface/animated/{feature_slug}/scroll", "scroll_", "interface/{feature_slug}_animated.gfx")
        },
        Row {
            frames: 20,
            rules: SPIN_RULES,
            note: "Twenty frames at 7 fps with loadType INGAME and transparencecheck, like the turning cogs of the leader-group and navy views (26x26 and 25x25). The subject is fitted by its radius, so it stays inside the frame at every angle, and turned clockwise a full circle per loop.",
            ..base(AnimSpin, [26, 26], &[[26, 26], [25, 25]], "gfx/interface/animated/{feature_slug}/spin", "spin_", "interface/{feature_slug}_animated.gfx")
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

/// The animation fields of an animated family, as vanilla writes them for the sprites it is measured from
/// (`data/vanilla-measurement.json`: every one of those sprites loops and plays on show with pause_on_loop 0.0).
pub fn animation_of(family: Family) -> Option<AnimationSpec> {
    let blend = |motion, fps: &str| AnimationSpec {
        motion,
        fps: fps.into(),
        looping: true,
        play_on_show: true,
        pause_on_loop: "0.0".into(),
        effect_file: Some(animation::BLEND_EFFECT.into()),
        always_transparent: true,
        load_type: None,
        transparence_check: false,
    };
    let strip = |motion, fps: &str| AnimationSpec {
        motion,
        fps: fps.into(),
        looping: true,
        play_on_show: true,
        pause_on_loop: "0.0".into(),
        effect_file: None,
        always_transparent: false,
        load_type: Some("INGAME".into()),
        transparence_check: true,
    };
    Some(match family {
        Family::AnimBlendGlow => blend(Motion::BlendGlow, "1"),
        Family::AnimBlendKeyframes => blend(Motion::BlendKeyframes, "1"),
        Family::AnimScroll => strip(Motion::Scroll, "15"),
        Family::AnimSpin => strip(Motion::Spin, "7"),
        _ => return None,
    })
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
                    source_canvas: r.source_canvas,
                    animation: animation_of(r.family),
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
    /// * `medal`: the dominant 16x28 files are older sprites no vanilla script or GUI names. Unit medals are
    ///   GFX_medal_icon_* strips of 28x42 frames with a GFX_medal_icon_*_large twin of 56x84 frames; the family makes
    ///   one 56x84 frame, which must be the frame of the measured 224x84 four-frame large strips (checked below).
    const NOT_DOMINANT: &[Family] =
        &[Family::IdeaConcern, Family::MioPolicy, Family::OperationIcon, Family::CounterLarge, Family::CounterSmall, Family::Medal];

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
                if f == Family::Medal {
                    let large = meas.count_at(id, [s[0] * 4, s[1]]).unwrap_or(0);
                    assert!(large >= 10, "medal: only {large} four-frame large strips of {}x{} frames", s[0], s[1]);
                    continue;
                }
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
            // documented exceptions: `background` says what the model is asked to draw. The report card and the
            // advisor card are made from an opaque picture and end with transparent corners; the achievement is
            // drawn as a transparent subject and ends on an opaque template.
            if matches!(f, Family::ReportEvent | Family::OperationPicture | Family::AdvisorCard) {
                assert_eq!(p.imagegen.background, Background::Opaque, "{f}");
                continue;
            }
            if f == Family::Achievement {
                assert_eq!(p.imagegen.background, Background::Transparent, "{f}");
                continue;
            }
            // vanilla's scrolling panels have rounded transparent corners around a static border; this family
            // scrolls the whole frame, which is opaque
            if f == Family::AnimScroll {
                assert_eq!(p.imagegen.background, Background::Opaque, "{f}");
                continue;
            }
            let class = meas.alpha_class(id).unwrap();
            let want = if class.starts_with("transparent") { Background::Transparent } else { Background::Opaque };
            assert_eq!(p.imagegen.background, want, "{f}: measured '{class}'");
        }
    }

    /// The animation fields of every animated family are the most common ones of the vanilla sprites it is measured
    /// from, and the derivation constants are the measured ones.
    #[test]
    fn animation_fields_are_the_measured_vanilla_ones() {
        let meas = data();
        for (f, p) in by_family() {
            let anim = p.imagegen.animation.as_ref();
            assert_eq!(anim.is_some(), f.group() == Group::Animation, "{f}");
            let Some(anim) = anim else { continue };
            let id = m::entries_of(f)[0];
            let mode = |key: &str| meas.registration_mode(id, key).unwrap_or_else(|| panic!("{f}: no {key}")).0;
            assert_eq!(anim.fps, mode("animation_rate_fps_values"), "{f}");
            assert_eq!(anim.looping, mode("looping_values") == "yes", "{f}");
            assert_eq!(anim.play_on_show, mode("play_on_show_values") == "yes", "{f}");
            assert_eq!(anim.pause_on_loop, mode("pause_on_loop_values"), "{f}");
            assert_eq!(anim.always_transparent, mode("alwaystransparent_values") == "yes", "{f}");
            let effect = mode("effect_file_values");
            let ours = anim.effect_file.as_deref().map(str::to_ascii_lowercase).unwrap_or_else(|| "(none)".into());
            assert_eq!(ours, effect, "{f}");
            assert!(p.imagegen.checks.iter().any(|c| c == "animation"), "{f}");
            assert!(p.frames >= 2, "{f}");
            assert!(p.free_canvas(), "{f}: the element the strip sits on sets its size");
        }
        let dim = meas.animation_number("anim_blend_glow", "median_dim_to_bright_alpha_ratio").unwrap();
        assert!((dim - crate::animation::GLOW_DIM_RATIO).abs() < 0.005, "measured {dim}");
        let set = by_family();
        assert!(!set[&Family::AnimBlendGlow].source_transparent(), "a glow is drawn as light on black");
        assert!(set[&Family::AnimSpin].source_transparent());
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
        assert_eq!(ach.imagegen.background, Background::Transparent, "the model draws a transparent subject layer; the templates go beneath it");
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
        // every family is generated end to end except the three whose second frame is different art
        for (f, p) in by_family() {
            let want = if matches!(f, Family::CounterLarge | Family::CounterSmall | Family::DivisionEmblem) {
                Automation::ImportOnly
            } else {
                Automation::Generate
            };
            assert_eq!(p.imagegen.automation, want, "{f}");
        }
        let set = by_family();
        for f in [
            Family::AdvisorCard,
            Family::Achievement,
            Family::LeaderPortrait,
            Family::CommanderPortrait,
            Family::OperativePortrait,
            Family::ScientistPortrait,
        ] {
            assert_eq!(set[&f].imagegen.automation, Automation::Generate, "{f}");
        }
        for f in [Family::FactionLogo, Family::AgencyLogo, Family::Building] {
            assert_eq!(set[&f].imagegen.automation, Automation::Generate, "{f}: the chain builds the strip");
            assert_eq!(set[&f].frames, 2);
        }
        assert_eq!(set[&Family::AdvisorCard].imagegen.source_canvas, Some([156, 210]), "the card is made from a tall portrait");
        assert_eq!(set[&Family::AdvisorCard].imagegen.background, Background::Opaque);
        // the profile margin is the minimum every frame keeps, the same as the kit row; the room for frame 2's glow
        // is the recipe's fit margin
        assert_eq!(set[&Family::FactionLogo].margin, 2);
        assert_eq!(set[&Family::AgencyLogo].margin, 2);
        assert_eq!(crate::recipes::frames::selected_fit_margin(Family::FactionLogo), 10);
        assert_eq!(crate::recipes::frames::selected_fit_margin(Family::AgencyLogo), 14);
    }

    #[test]
    fn opaque_families_check_for_opacity_and_every_family_has_its_group_rubric() {
        for (f, p) in by_family() {
            assert_eq!(p.imagegen.rubric, format!("{}_v1", f.group().as_str()), "{f}");
            if p.imagegen.background == Background::Opaque {
                // cards keep transparent corners on purpose: they are checked for the corners instead of for opacity
                let card = matches!(f, Family::ReportEvent | Family::AdvisorCard);
                let want = if card { "card_corners" } else { "opaque" };
                assert!(p.imagegen.checks.iter().any(|c| c == want), "{f} must check {want}");
                assert_eq!(p.imagegen.checks.iter().any(|c| c == "opaque"), !card, "{f}");
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
