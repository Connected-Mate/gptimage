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
/// Portraits: a warning when the backdrop is far from the family's vanilla backdrop (`GFX-PORTRAIT-BACKDROP`).
const PORTRAIT_CHECKS: &[&str] = &["size", "checker", "opaque", "backdrop", "style", "dds"];
const FLAG_CHECKS: &[&str] = &["size", "opaque", "style", "tga"];
const UNIT_CHECKS: &[&str] = &["size", "alpha", "frames", "style", "dds"];
/// Counters and the division emblem: finished art must be vanilla's material (`GFX-UNIT-MATERIAL`) and shaped like one
/// glyph (`GFX-UNIT-GLYPH-SHAPE`).
const COUNTER_CHECKS: &[&str] = &["size", "alpha", "frames", "unit_material", "unit_glyph_shape", "style", "dds"];
/// Special project icons: nothing may be filled (`GFX-FILLED-INTERIOR`).
const LINE_ART_CHECKS: &[&str] = &["size", "alpha", "opaque_canvas", "fringe", "margin", "fill", "line_art", "checker", "style", "dds"];
/// Technology pictures: a hint when the subject is too narrow for vanilla's side view (`GFX-TECH-VIEW`).
const TECH_CHECKS: &[&str] = &["size", "alpha", "opaque_canvas", "fringe", "margin", "fill", "side_view", "checker", "style", "dds"];
const INTERFACE_CHECKS: &[&str] = &["size", "alpha", "style", "dds"];
const SUPER_CHECKS: &[&str] = &["size", "checker", "opaque", "style", "dds"];
/// Model references are opaque PNG input for a 3D tool: a plain backdrop and a whole subject, and no DDS.
const MODEL_CHECKS: &[&str] = &["size", "checker", "opaque", "plain_backdrop", "style"];
const ANIM_CHECKS: &[&str] = &["size", "alpha", "opaque_canvas", "margin", "frames", "animation", "checker", "style", "dds"];
const SCROLL_CHECKS: &[&str] = &["size", "opaque", "frames", "animation", "checker", "style", "dds"];

const ICON_RULES: &[&str] = &[
    "One centred subject with a strong, simple silhouette that stays readable at {w}x{h} pixels.",
    "Painterly, slightly aged texture in the style of Hearts of Iron IV interface art, with readable contrast.",
    "Real transparent background: no backdrop, no frame, no border, no plate behind the subject.",
    "Leave a small empty margin around the subject.",
];
/// An icon family whose canvas has a side of this many pixels or fewer gets [`SMALL_CANVAS_RULE`], its negatives and
/// the `small_detail` check: decision, decision category icon, state modifier, raid and the two trait families.
pub const SMALL_CANVAS_MAX: u32 = 40;

/// Readability on a small canvas: at 32x32 thin lines and small parts do not read, such as a dinghy with figures, a
/// broken arch bridge or twisted rails.
const SMALL_CANVAS_RULE: &str = "Small canvas: at {w}x{h} pixels only one bold object reads. Draw it with a few large, solid shapes and thick parts, nothing smaller than a tenth of the canvas: no thin parallel lines (rails, wires, ropes, cables, railings), no lattices or open frameworks, no groups of small figures, no scene, ground or water line.";
const SMALL_CANVAS_NEGATIVE: &[&str] = &["thin parallel lines, wires or lattices", "several small objects or figures"];

/// True when a family gets the small-canvas rule and check: an icon whose smaller side is at most
/// [`SMALL_CANVAS_MAX`].
pub fn is_small_icon(family: Family, canvas: [u32; 2]) -> bool {
    family.group() == Group::Icon && canvas[0].min(canvas[1]) > 0 && canvas[0].min(canvas[1]) <= SMALL_CANVAS_MAX
}

// Per-family form and medium. The general icon rules ask for a loose subject with no frame, which is wrong for the
// families below: their vanilla art (the kit's 1.19.4.1 reference shelf) has a fixed form. Each rule states that
// form so the model draws it, and those families drop "a frame or border" from the negative list.
/// Vanilla focus icons have no single frame. On a look at 160 vanilla `goal_*` and `focus_*` files (HOI4 1.19.4.1, 80
/// drawn from all of them and 80 from the generic ones) about seven in ten sit on a round plaque or medallion, about
/// one in ten on a diamond, square or rectangular plaque, about one in fifteen on a heraldic shield, and about one in
/// eight have no frame at all. Frames, colours and compositions vary, so the rule names the range.
const FOCUS_RULES: &[&str] = &[
    "The Hearts of Iron IV national focus icon form: one emblem in aged painterly metal and enamel. Vanilla focus icons do not share one frame. Most (about seven in ten) set the subject on a round plaque or medallion, often with a laurel wreath, ribbons, crossed swords or flags around it. Others use a diamond, square or rectangular plaque (about one in ten), a heraldic shield (about one in fifteen), or no frame at all, a free-standing subject such as a flag, a star, a crown, a document, a building or an aircraft (about one in eight). Pick the form, frame colour and composition that suit this brief instead of defaulting to a round plaque.",
    "Flags and ribbons use invented colours, never a real country's flag, unless the brief names that country.",
    "The subject is large: it fills most of the emblem and may overlap its frame, never a small figure inside a wide empty plaque.",
    "A strong central symbol that supports the focus's story, readable contrast at {w}x{h} pixels.",
    "Real transparent background around the emblem: no backdrop behind it.",
    "Leave a small empty margin around the emblem.",
];
/// National spirits may be emblems, figures, objects or small scenes, so the form stays open and only readability at
/// the native size is required.
const IDEA_SPIRIT_RULES: &[&str] = &[
    "One centred subject, an emblem, a figure, an object or a small scene, readable at {w}x{h} pixels: a few large, clear shapes with readable contrast and no fine detail that disappears at that size.",
    "Painterly, slightly aged texture in the style of Hearts of Iron IV interface art.",
    "Real transparent background: no backdrop, no frame, no border, no plate behind the subject.",
    "Leave a small empty margin around the subject.",
];
const CONCERN_RULES: &[&str] = &[
    "The Hearts of Iron IV industrial concern form: a company badge or logo emblem, such as a shield, roundel or winged badge with the company's symbol.",
    "The symbol is large: it fills most of the badge's face, never a small mark inside a wide empty badge.",
    "Clean painted enamel and metal, readable at {w}x{h} pixels.",
    "Real transparent background outside the badge.",
    "Leave a small empty margin around the badge.",
];
const CATEGORY_ICON_RULES: &[&str] = &[
    "The Hearts of Iron IV decision category icon form: one symbol on a small round disc with a dark riveted metal rim. The disc face varies between steel blue, dark red, tan and dark grey, so pick the one that suits the brief.",
    "The symbol is large: it fills most of the disc and may overlap the rim, never a small mark inside a wide empty disc.",
    "Painted, slightly aged, readable at {w}x{h} pixels.",
    "Real transparent background outside the disc.",
    "Leave a small empty margin around the disc.",
];
const BLUEPRINT_RULES: &[&str] = &[
    "The Hearts of Iron IV special project icon form: a clean technical line drawing of the project's subject, a side or plan view with clear outlines and a few inner detail lines, or one plain flat silhouette.",
    "No shading, no painted texture, no scene and nothing around the subject: the tool redraws it as vanilla's thin pale lines, so only the outline and the main inner lines matter. Readable at {w}x{h} pixels.",
    "Real transparent background, or one plain flat colour behind the subject.",
    "Leave a small empty margin around the drawing.",
];
/// Special project negatives: what keeps the redrawn lines clean.
const BLUEPRINT_NEGATIVE: &[&str] = &[
    "text",
    "letters or numbers",
    "watermark or signature",
    "a drawn checkerboard pattern",
    "a frame or border",
    "shading, gradients or painted texture",
    "a scene, ground, sky or launch site around the subject",
    "a paper sheet or a grid",
];
const RAID_RULES: &[&str] = &[
    "The Hearts of Iron IV raid icon form: one simple, bold pictogram of the raid's target or means (a factory, an aircraft, a ship, a dam, a tool) as one solid shape with a few darker detail lines inside it.",
    "Flat tones only, no painted texture and no scene: the tool recolours the shape in vanilla's gold gradient (pale gold at the top, brown at the bottom) and draws the black outline, so only the shape and its light and dark details matter.",
    "ONE object only, large and seen from the side: no flames, smoke, ground, sky or second object around it, so the shape alone reads at {w}x{h} pixels.",
    "Real transparent background.",
    "Leave a small empty margin around the pictogram.",
];
const MIO_DEPARTMENT_RULES: &[&str] = &[
    "The Hearts of Iron IV military industrial organisation department icon form: ONE piece of equipment (an aircraft, a tracked or wheeled vehicle, a ship or a piece of field equipment) as a bold, flat interface icon, seen from the side or three quarters.",
    "The whole piece is tinted one bold, saturated colour: aircraft red or blue, land equipment olive green, ships steel blue. It is never grey, metallic, natural-coloured or photographic. Simple flat shading and a thin dark outline.",
    "Depict the equipment itself, never plans, blueprints, drawings or models of it, even when the brief names a design bureau, a design team or a department.",
    "At most one small accent symbol beside it, such as circular green arrows, a rising yellow zigzag arrow, a wrench, gears, a star, a shield or a gem. Readable at {w}x{h} pixels.",
    "The equipment fills nearly the whole canvas. Real transparent background: no scene, no workshop, no paper, no panel and no plate behind it.",
];
/// MIO department negatives: the scene and paper the department's name invites.
const MIO_DEPARTMENT_NEGATIVE: &[&str] = &[
    "text",
    "letters or numbers",
    "watermark or signature",
    "a drawn checkerboard pattern",
    "a frame or border",
    "a scene, landscape, workshop or background",
    "grey or metallic shading, natural colours or a photographic look",
    "a blueprint, plan, technical drawing or paper sheet",
    "panels, dividers or tiles",
    "drafting tools such as a compass or a ruler",
    "a second main object",
];
/// MIO traits. Measured on the 44 vanilla trait icons (HOI4 1.19.4.1), inside the glyph (alpha 224 and up): mean value
/// 0.50 in the median file (10th percentile 0.39), dark pixels (value under 0.2) 6 percent and light ones (over 0.75)
/// 15 percent, 14 colour buckets for 90 percent of the pixels. Around it, a soft black shadow takes 48 percent of the
/// visible pixels. The 0.3.2 gallery's traits had about the same number of colours (7 to 16) but were dark and
/// murky inside (mean value 0.24 to 0.44, 5 to 43 percent dark pixels, almost no light ones): shaded painted objects,
/// not light, flat glyphs.
const MIO_TRAIT_RULES: &[&str] = &[
    "The Hearts of Iron IV military industrial organisation trait icon form: one small, simple glyph of one object or symbol, drawn flat like an interface pictogram (a shield, a star, a wrench, binoculars, a shell, an aircraft, a turret, an anvil, rising bars), often with one small accent sign beside it, such as a green plus, a green arrow or a red target ring.",
    "Flat and low in detail: a few large areas of clean colour with light shading, one highlight and one shade per area. Light tones overall (light steel, pale brass, white or light grey) with one or two saturated accents (green, red, gold or orange), and little dark area inside the shape. No texture, grime, rivets, bolts, cables or small parts, and no realistic lighting or reflections.",
    "A thin dark outline follows the whole shape, with a soft dark shadow around it. Seen from the front or the side with little depth, bold enough to read at {w}x{h} pixels.",
    "Real transparent background: no badge, disc, plate or square behind the glyph.",
    "Leave a small empty margin around it.",
];
/// MIO trait negatives: the badge the old rule asked for, the painted scene a brief invites, and the dark, detailed
/// object the 0.3.2 rule still produced.
const MIO_TRAIT_NEGATIVE: &[&str] = &[
    "text",
    "letters or numbers",
    "watermark or signature",
    "a drawn checkerboard pattern",
    "a round badge, disc or plate behind the object",
    "a detailed, realistically lit object",
    "dark, murky or heavily shaded painting",
    "fine mechanical detail, texture or grime",
    "a three-quarter view from above with visible depth",
    "a scene, landscape or background",
];
/// Agency upgrades. Measured on the 15 vanilla upgrades (HOI4 1.19.4.1): mean saturation 0.249 and mean value 0.423
/// in the median file, but a wide tonal range (luminance standard deviation 0.23, the 10th to 90th percentile 0.12 to
/// 0.72) and clear accent colours: 13 percent of the disc has a saturation above 0.5 (brass, steel blue, brick red,
/// green), the 90th percentile of saturation is 0.56. The 0.3.2 gallery's upgrades were darker, more saturated and
/// warmer, busy rooms full of small objects. Its 0.3.4 to 0.3.7 replacement, "pale and muted, never saturated", came out
/// as a flat grey-green haze (saturation 0.11, luminance standard deviation 0.16, no accent at all). The vignette
/// recipe brings any subject into vanilla's mean palette and edge and stretches a flat picture, and the rules ask for
/// the clear colour and the real darks and highlights it cannot make up.
const AGENCY_UPGRADE_RULES: &[&str] = &[
    "The Hearts of Iron IV intelligence agency upgrade form: a round painted vignette, a disc whose whole face is one simple close-up (one person at work, hands, one device, documents or one piece of equipment), seen at eye level or slightly from above.",
    "Rich but sober colour, like a hand-painted 1940s illustration: strong contrast between deep shadow and bright clean highlights (cream, white, pale gold) with dusty mid-tones, and one or two clear accent colours on the subject, such as brass or gold, steel blue, brick red or leaf green, against a softer dusty grey-green, steel-blue or slate background. Keep the scene cool or neutral overall (slate, steel, olive grey, with only a small brass or red accent): not a warm lamp-lit or sepia scene. Never a flat grey haze, never neon or garish, and no overall orange or sepia cast.",
    "One subject large in the disc on a plain, softly painted background: a few large shapes, no crowded room, no shelves or desks full of objects and no fine detail that disappears at {w}x{h} pixels.",
    "The scene runs to the edge of the disc, which is softly darkened. There is no ring, rim, frame or border around the disc.",
    "Real transparent background outside the disc.",
    "Leave a small empty margin around the disc.",
];
/// Agency upgrade negatives: the loose object and the ring the old rules produced, the dark, warm, busy painting of
/// 0.3.2 and the flat grey haze of 0.3.4 to 0.3.7.
const AGENCY_UPGRADE_NEGATIVE: &[&str] = &[
    "text",
    "letters or numbers",
    "watermark or signature",
    "a drawn checkerboard pattern",
    "a ring, rim or metal frame around the disc",
    "a square or rectangular picture",
    "a loose object on a transparent background without the round scene",
    "a black and white or sepia photograph",
    "a flat grey or grey-green haze with no real darks, highlights or accent colour",
    "neon, garish or candy colours, or an overall warm brown and orange cast",
    "a warm lamp-lit sepia scene",
    "a busy scene with many small objects",
];
const OPERATION_ICON_RULES: &[&str] = &[
    "The Hearts of Iron IV operation icon form: one square picture, a painted or photographic close-up of the operation's act, such as hands on a document, a bundle of dynamite, a handshake, soldiers in a line, a podium with guards or a tool on a table, filling the whole frame and composed for a {w}x{h} crop.",
    "A warm, muted colour palette (browns, tans, warm greys, with a few colour accents), soft dramatic light and a slightly aged, painterly or photographic look. The subject is cropped tightly and reaches the edges of the picture. Keep the picture light and high-key: pale paper, bright lit hands or faces and a light, softly blurred surround; never a dark, black or night scene.",
    "This family has only this form. It is not an isolated object on a blank background, not a round vignette (the phase icon), and not a tilted card: the card tilt, the outline and the shadow are added afterwards.",
    "Do not draw a border, a frame, a ring, a tilt or a shadow.",
];
/// Operation icon negatives: an isolated object, a ring or frame, text.
const OPERATION_ICON_NEGATIVE: &[&str] = &[
    "text, captions or numbers",
    "watermark or signature",
    "a frame, border or ring",
    "an isolated object on a blank or transparent background",
    "an icon, emblem or symbol",
    "a round vignette",
    "a tilted card or a drop shadow",
];
const OPERATION_PICTURE_RULES: &[&str] = &[
    "The Hearts of Iron IV operation phase picture form: one period photograph of the phase's scene that fills the whole frame, composed for a {w}x{h} crop, in monochrome sepia or warm black and white with natural film grain, a documentary press-photo look.",
    "This family has only this form. It is not a painted illustration and not in colour, and it is not the round vignette of the phase icon (operation_icon).",
    "Do not draw a border, a torn edge, a tilt or a shadow.",
];
/// Operation phase picture negatives: colour, paint and the icon's round form.
const OPERATION_PICTURE_NEGATIVE: &[&str] = &[
    "text, captions or headlines",
    "watermark or signature",
    "a frame or border",
    "colour other than sepia",
    "a painted illustration",
    "a round vignette",
];
const MIO_POLICY_RULES: &[&str] = &[
    "The Hearts of Iron IV military industrial organisation policy icon form: one object, or two that belong together, as a painted interface icon (crossed rifles, a ship, a light bulb in a hand, a signed contract, a spirit level, a chart with shells), seen from the front or three quarters.",
    "Painted semi-realistic metal, wood and paper in muted colours with a few saturated accents, a thin dark outline and a soft dark shadow. No perspective depth and nothing that recedes into the distance. Readable at {w}x{h} pixels.",
    "Real transparent background: no room, no factory hall, no landscape and no backdrop.",
    "Leave a small empty margin around the icon.",
];
/// MIO policy negatives: the busy scene the brief invites.
const MIO_POLICY_NEGATIVE: &[&str] = &[
    "text",
    "letters or numbers",
    "watermark or signature",
    "a drawn checkerboard pattern",
    "a frame or border",
    "a scene, room, factory hall or landscape",
    "a row of many repeated objects or a view into the distance",
];
const BOP_SIDE_RULES: &[&str] = &[
    "The Hearts of Iron IV balance of power side form: one person who leads or stands for the side, as a head-and-shoulders portrait cut out with no backdrop, or as a bust on a small plinth. A side that is not a person is one symbol in a laurel wreath.",
    "Painted in muted natural colours, a photo-like 1930s portrait or polished metal with gold or bronze details, with a soft dark outline. The tool makes the grey standard stage and the colour improved stage from this one picture, so the emblem must read as well in grey as in colour: strong light and dark, no detail that exists only as a colour. The figure faces the viewer or three quarters, readable at {w}x{h} pixels.",
    "The references show the shape and the finish. They are the grey standard stage: keep their form and draw this subject in muted colour anyway.",
    "Real transparent background: no room, no scene and no plate behind the figure.",
    "Leave a small empty margin around it.",
];
/// Balance of power negatives: the colourful objects and banners the old rules allowed.
const BOP_SIDE_NEGATIVE: &[&str] = &[
    "text",
    "letters or numbers",
    "watermark or signature",
    "a drawn checkerboard pattern",
    "a glow, halo or aura around the figure",
    "cracks, damage or a split emblem",
    "a banner, flag or loose object instead of a person",
    "a scene or backdrop",
];
const STATE_MODIFIER_RULES: &[&str] = &[
    "The Hearts of Iron IV state modifier icon form: one bold, simple object or symbol (a factory, a raised fist, a flag, a flame, crossed tools, a helmet, a crown), at most two elements, drawn large with chunky shapes.",
    "High contrast: a strong dark outline, light shading and one or two saturated accents (red, orange, gold or green). It must read at {w}x{h} pixels, so no small details, no thin lines and no scene.",
    "Real transparent background: no landscape, no ground and no backdrop.",
    "Leave a one-pixel empty margin around the icon.",
];
/// State modifier negatives: what is unreadable at 32 px.
const STATE_MODIFIER_NEGATIVE: &[&str] = &[
    "text",
    "letters or numbers",
    "watermark or signature",
    "a drawn checkerboard pattern",
    "a frame or border",
    "a scene, landscape or wide view",
    "small details, thin lines or low contrast",
];
const COMMANDER_TRAIT_RULES: &[&str] = &[
    "The Hearts of Iron IV unit leader trait form: a small military medal seen straight on. At the top, a short vertical ribbon with two or three coloured stripes under a thin gold bar takes about the top third. Below it hangs the trait's symbol as a grey steel or silver metal piece (a vehicle, an aircraft, a ship, an animal head, a helmet or crossed tools), often set on a small bronze laurel wreath or ring.",
    "The symbol is monochrome metal, never painted in its own colours: only the ribbon and the wreath carry colour. Bevelled, softly shaded metal with a thin dark outline, readable at {w}x{h} pixels.",
    "Real transparent background.",
    "Leave a small empty margin around the medal.",
];
/// Commander trait negatives: the painted subject without a medal that the old rules produced.
const COMMANDER_TRAIT_NEGATIVE: &[&str] = &[
    "text",
    "letters or numbers",
    "watermark or signature",
    "a drawn checkerboard pattern",
    "a square backdrop behind the medal",
    "a vehicle, animal or figure painted in natural colours",
    "a scene or ground",
    "a symbol without the ribbon above it",
];
const DOCTRINE_RULES: &[&str] = &[
    "The Hearts of Iron IV doctrine icon form: flat side-view silhouettes of the units (soldiers, tanks, aircraft or ships) grouped into one compact emblem, with bright orange or yellow accents such as arrows, chevrons or bars.",
    "Simple flat colour, as in the game: aircraft in one flat sky blue, ships and submarines in one flat teal or sea blue, the accents (arrows, chevrons, flags, bursts, fire) in bright orange or yellow. Soldiers, tanks and vehicles are solid dark shapes. One flat tone per shape, no painted texture, and no other colours (no green, red or purple). The tool adds vanilla's pale outline and dark shadow, and its soft light-to-dark fall inside the coloured shapes.",
    "Few, large, bold shapes that fill the square: one or two big aircraft or ships (never a row of small ones), or a tank or a few soldiers, with one thick orange arrow or chevron. Each unit is a simple solid silhouette with no panel lines, rivets or window detail.",
    "No painted texture or scene, readable at {w}x{h} pixels.",
    "Real transparent background.",
    "Leave a small empty margin around the pictogram.",
];
/// Technology pictures: one fixed view, the one every inspected vanilla picture uses.
const TECHNOLOGY_RULES: &[&str] = &[
    "The Hearts of Iron IV technology picture form: ONE piece of equipment (a vehicle, an artillery piece, an aircraft or a ship) seen exactly from the side, a flat side profile with no three-quarter view, no front view and no perspective, facing right: its front, nose, barrel or bow points to the right edge.",
    "The equipment is long and low and fills most of the width, readable at {w}x{h} pixels, painted in the slightly aged style of Hearts of Iron IV interface art.",
    "Real transparent background: no backdrop, no ground, no frame, no border, no plate.",
    "No numerals or text on the picture: the tool stamps a tier numeral itself when one is asked for.",
    "Leave a small empty margin around the equipment.",
];
/// Negatives for families whose form is a plaque, disc, badge, medallion or medal: the frame is the form.
const FORM_NEGATIVE: &[&str] =
    &["text", "letters or numbers", "watermark or signature", "a drawn checkerboard pattern", "a square backdrop behind the subject"];
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
    "An invented person: the face, the name and the likeness are fictional and must not resemble any real historical figure.",
];
const NEWS_RULES: &[&str] = &[
    "A black and white press photograph that fills the whole frame, composed for a wide {w}x{h} crop.",
    "Period newspaper look with natural grain.",
];
const PICTURE_NEGATIVE: &[&str] = &["text, captions or headlines", "watermark or signature", "a frame or border"];
/// A model reference is the input picture of an image-to-3D tool (Meshy): the whole subject, nothing cropped, on a
/// plain backdrop the tool can separate, with no grain or noise that would end up in the model's texture.
const MODEL_RULES: &[&str] = &[
    "One subject, shown whole from head to foot (or end to end), with a clear margin on every side and nothing cropped, composed for a {w}x{h} frame.",
    "A three-quarter front view with natural perspective and even, soft studio lighting, so every part of the subject's form reads.",
    "A plain, flat light grey backdrop (about RGB 200, 200, 200) with no ground plane, no cast shadow, no horizon and no setting.",
    "Clean, sharp detail with no film grain, noise, blur or painterly texture: the picture becomes the model's texture.",
];
const MODEL_NEGATIVE: &[&str] = &[
    "a landscape, room or other setting",
    "a ground plane, floor or cast shadow",
    "film grain, noise or texture overlays",
    "a second subject",
    "a cropped or cut-off subject",
    "text, captions or watermarks",
    "a frame or border",
];

/// Vanilla leader portraits (gfx/leaders, 1.19.4.1): a close head-and-shoulders crop (the face is about half the
/// height) on a plain pale backdrop. Generated portraits use that crop and backdrop. A custom backdrop is only for
/// council or otherwise unusual portraits.
/// Portrait framing, shared by the four portrait families.
const PORTRAIT_FRAMING: &str = "A close head-and-shoulders portrait in the painterly style of Hearts of Iron IV portraits, composed for a tall {w}x{h} crop: the head fills the upper half of the frame, the face about half the height, and the shoulders run off the bottom and the sides.";
const PORTRAIT_PERIOD: &str = "Period-appropriate clothing.";
const PORTRAIT_INVENTED: &str =
    "An invented person: the face, the name and the likeness are fictional and must not resemble any real historical figure.";
// Each portrait family's own vanilla backdrop, measured on HOI4 1.19.4.1 in the two top corners and the side strips
// (see validate::portrait_backdrop): leaders and commanders share one pale cream-grey template, scientists have a
// pale blue-grey one lit from the upper left, and vanilla's operative portraits are dark charcoal.
/// Leader and commander portraits: 424 of 447 decodable files outside the scientists folder.
const LEADER_PORTRAIT_RULES: &[&str] = &[
    PORTRAIT_FRAMING,
    "Vanilla's leader portrait backdrop: one plain, smooth studio background in pale cream-grey (about RGB 229, 226, 209 at the top), evenly lit and darker toward the bottom (about 190, 186, 170 at the sides by the shoulders), with no objects, room, landscape, flag or pattern. Not the blue-grey of scientist portraits and not a dark background. Use another backdrop only when the brief asks for a council or otherwise unusual portrait.",
    PORTRAIT_PERIOD,
    PORTRAIT_INVENTED,
];
/// Scientist portraits: the 217 files of the scientists folder, one template.
const SCIENTIST_PORTRAIT_RULES: &[&str] = &[
    PORTRAIT_FRAMING,
    "Vanilla's scientist portrait backdrop: one plain, smooth studio background in pale blue-grey (about RGB 135, 171, 193 at the top left), lit from the upper left and darker toward the lower right (about 93, 129, 151 at the right by the shoulders), with no objects, laboratory, blackboard, room or pattern. Not the pale cream of leader portraits. Use another backdrop only when the brief asks for a council or otherwise unusual portrait.",
    PORTRAIT_PERIOD,
    PORTRAIT_INVENTED,
];
/// Operative portraits: La Resistance's 163 operative portraits (`portrait_*_operative_*`), corners RGB about (34, 31, 24),
/// the side band at two thirds of the height about (95, 92, 85).
const OPERATIVE_PORTRAIT_RULES: &[&str] = &[
    PORTRAIT_FRAMING,
    "Vanilla's operative portrait backdrop: a dark warm charcoal studio backdrop with a soft lighter spotlight behind the figure. The top corners are near black-brown (about RGB 34, 31, 24), and a warm grey glow centred behind the head and shoulders lights the backdrop around them, lightest beside the shoulders (about RGB 95, 92, 85) and fading smoothly into the dark corners. Plain, with no objects, room, street or pattern. Not the pale cream of leader portraits, not the blue-grey of scientists, and not an evenly dark background. Use another backdrop only when the brief asks for a council or otherwise unusual portrait.",
    PORTRAIT_PERIOD,
    PORTRAIT_INVENTED,
];
const FRAME_STRIP_RULES: &[&str] = &[
    "One centred emblem with a strong silhouette that reads at {w}x{h} pixels, drawn in the style of Hearts of Iron IV faction and interface art.",
    "Real transparent background: no backdrop, no frame, no plate. Leave clear space around the emblem: the tool adds the second frame (its highlight or its disabled look) itself.",
];
/// The widest a landmark may be, width over height of one frame's subject. Vanilla's 52 landmark strips (HOI4
/// 1.19.4.1, frame 1 at 128x64 and 64x32) run from 0.34 (Big Ben) to 4.00 (the small Cataract dam), median 1.41, 95th
/// percentile 3.77, and every frame's box is centred within 0.47 to 0.55 of the frame. On 0.3.3, 6 of 12 generated
/// landmarks (dams, a grain elevator, a palace) were refused by the vanilla-match check as wider than vanilla.
pub const BUILDING_MAX_ASPECT: f64 = 4.0;

const BUILDING_RULES: &[&str] = &[
    "The Hearts of Iron IV landmark building form: one real-world landmark (a monument, a government building, a tower, a statue, a dam or an industrial site) as a small, detailed painted miniature, seen from the front at a slightly raised angle. It stands on nothing: the base of the building is the bottom of the drawing, with no ground, rock or hill under it (a dam or a canal keeps only its water).",
    "Compact and centred: the building is at most about four times as wide as it is tall, with its mass in the middle of the drawing. Show a long subject such as a dam, a wall, a grain elevator or a palace foreshortened, from an angle, or as its central part, never as a long thin strip.",
    "Muted stone greys, browns and weathered copper green, crisp painted detail and a thin dark outline around the whole silhouette, readable at {w}x{h} pixels.",
    "Real transparent background: no sky, no landscape, no hill and no backdrop. Leave a small empty margin around the building.",
    "Frame 1 is the intact building. Frame 2, a second candidate of the same asset, shows the same building at the same scale and position as ruins (broken walls and rubble, lower than the intact building) with one small stylised orange flame above it.",
];
/// Landmark negatives: the hill, landscape and smoke the old rules allowed.
const BUILDING_NEGATIVE: &[&str] = &[
    "text",
    "letters or numbers",
    "watermark or signature",
    "a drawn checkerboard pattern",
    "a frame or border",
    "a hill, landscape, sky or backdrop",
    "ground, rocks or a diorama base under the building",
    "a fantasy castle",
    "a large smoke cloud",
    "a long, thin panorama much wider than it is tall",
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
const COUNTER_RULES: &[&str] = &[
    "Draw the unit seen from the side as ONE bold, solid black silhouette: a single filled shape with a clear outline, wider than tall, that still reads as the unit when shrunk to a tiny map counter.",
    "Plain flat white background, or a real transparent one. No ground line, no cast shadow, no scenery, no second object.",
    "Show inner detail only as a few thin white gaps cut into the black shape (wheels, a hatch, a gap between parts). No grey shading, no colour, no texture.",
    "Draw only the unit itself: the tool adds the vanilla green or grey material, the lighting, the outline and the NATO box.",
];
const COUNTER_NEGATIVE: &[&str] = &[
    "text, letters or numbers",
    "a NATO symbol, a box, a frame or a border",
    "colour, grey shading or gradients",
    "a ground line, a cast shadow or scenery",
    "several objects",
    "watermark or signature",
];
const EMBLEM_RULES: &[&str] = &[
    "The Hearts of Iron IV division template symbol form: one plain heraldic or unit symbol (an animal head, a skull, a helmet, a star, wings, crossed blades or tools, a tower, a plant or a lightning bolt) seen flat, straight on or in profile, which the game shows as an embossed grey-green metal sign.",
    "Draw it as ONE bold, solid black silhouette, centred, compact, readable at {w}x{h} pixels and at 30x12 pixels.",
    "Plain flat white background, or a real transparent one. No badge, no shield plate, no ring or frame around it unless the ring is the emblem.",
    "Show inner detail only as a few thin white gaps cut into the black shape (an eye, a line between parts). No grey shading, no colour, no texture.",
    "The tool adds vanilla's green material, the lighting and the outline itself.",
];
const EMBLEM_NEGATIVE: &[&str] = &[
    "text, letters or numbers",
    "a frame, a border or a backing plate",
    "colour, grey shading or gradients",
    "several objects",
    "watermark or signature",
];
const INTERFACE_RULES: &[&str] = &["Match the style of Hearts of Iron IV interface art; the exact size is {w}x{h} pixels."];
/// GUI textures: the shared form of vanilla's interface material. A request's `texture_kind` (button, frame, panel,
/// background) adds that kind's form ([`crate::family::GuiTextureKind::form_rule`]) and picks its vanilla references.
const GUI_TEXTURE_RULES: &[&str] = &[
    "The Hearts of Iron IV GUI texture form: flat interface material seen straight on, with no perspective, no scene and no subject: dark charcoal, gunmetal grey, olive green or worn leather surfaces with thin bevelled edges and a faint worn texture.",
    "The exact size is {w}x{h} pixels, and the texture fills it the way the element it is made for does.",
];
/// GUI texture negatives. A frame or border is allowed: it is what a frame texture is.
const GUI_TEXTURE_NEGATIVE: &[&str] =
    &["text", "letters or numbers", "watermark or signature", "a drawn checkerboard pattern", "a scene or a single object", "perspective"];
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
            row.rules = match family {
                Family::ScientistPortrait => SCIENTIST_PORTRAIT_RULES,
                Family::OperativePortrait => OPERATIVE_PORTRAIT_RULES,
                _ => LEADER_PORTRAIT_RULES,
            };
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
            negative: FORM_NEGATIVE,
            note: "GFX_focus_ is the majority sprite prefix and GFX_goal_ the original (goal_ stays acceptable). Every vanilla focus has a _shine companion sprite.",
            ..base(Focus, [100, 88], &[[100, 88], [102, 90], [99, 88], [98, 88]], "gfx/interface/goals/{feature_slug}", "focus_", "interface/{feature_slug}_goals.gfx")
        },
        Row {
            margin: 1,
            rules: IDEA_SPIRIT_RULES,
            note: "Ideas share the GFX_idea_ namespace across four families, so the prefix cannot tell them apart: the consuming script slot and this folder do. Spirits are 60x68.",
            ..base(IdeaSpirit, [60, 68], &[[60, 68], [62, 70]], "gfx/interface/ideas/{feature_slug}", "idea_", "interface/{feature_slug}_ideas.gfx")
        },
        Row {
            margin: 3,
            rules: CONCERN_RULES,
            negative: FORM_NEGATIVE,
            note: "Industrial concerns and manufacturers are 64x64 (about 80 percent of their ideas); the dominant size of the shared idea folder is the spirit size, which is why this row is a documented exception in the drift test. The badge spans 58 of 64 px in the median of 505 vanilla files, so the fit margin is 3 px.",
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
            margin: 1,
            rules: CATEGORY_ICON_RULES,
            negative: FORM_NEGATIVE,
            note: "Two vanilla category icons are 21-frame animations and are not generated here. The disc is 38 px tall on the 40 px canvas in vanilla (95 percent of the height in 91 files), so the fit margin is 1 px.",
            ..base(DecisionCategoryIcon, [52, 40], &[[52, 40]], "gfx/interface/decisions/{feature_slug}/category_icons", "decision_category_", "interface/{feature_slug}_decisions.gfx")
        },
        Row {
            margin: 1,
            note: "Dynamic modifiers use this family (GFX_modifiers_*), sometimes aliased to GFX_idea_*.",
            rules: STATE_MODIFIER_RULES,
            negative: STATE_MODIFIER_NEGATIVE,
            ..base(StateModifier, [32, 32], &[[32, 32]], "gfx/interface/state_modifiers/{feature_slug}", "modifiers_", "interface/{feature_slug}_state_modifiers.gfx")
        },
        Row {
            margin: 3,
            grayscale: true,
            checks: LINE_ART_CHECKS,
            rules: BLUEPRINT_RULES,
            negative: BLUEPRINT_NEGATIVE,
            note: "Special project icons are pale line-art drawings on transparent: thin pure-white lines whose alpha makes them pale (median 90), about 4 percent of the canvas inked, in a box of 86 by 74 percent of the canvas. The line_art recipe redraws any generated image that way, so the model only has to give a clean shape. The line_art check refuses a filled area larger than 250 px (vanilla's largest is 171, its median 4), so filled art cannot pass validation.",
            ..base(SpecialProjectIcon, [161, 98], &[[161, 98]], "gfx/interface/special_project/project_icons/{feature_slug}", "sp_", "interface/special_projects/{feature_slug}_projects.gfx")
        },
        Row {
            margin: 4,
            rules: RAID_RULES,
            note: "Only the 32x32 raid icons are generated; the wide banner strips in the same folder are interface pieces. The 32x32 icon is the raid's map icon, which the game looks up as GFX_raid_type_icon_<raid type> (24 of the 25 vanilla ones are 32x32), so the asset name is the raid type key, or the raid sets custom_map_icon to the sprite. The pictogram spans 23 of 32 px in the median of 30 vanilla files, so the fit margin is 4 px. Vanilla's map icons are one gold ramp, light at the top and dark at the bottom, with a black outline on the shape's edge; the raid recipe's gold_gradient step recolours any subject that way.",
            ..base(Raid, [32, 32], &[[32, 32]], "gfx/interface/military_raids/{feature_slug}", "raid_type_icon_", "interface/{feature_slug}_raids.gfx")
        },
        Row {
            margin: 1,
            rules: MIO_DEPARTMENT_RULES,
            negative: MIO_DEPARTMENT_NEGATIVE,
            note: "Vanilla sprite names are free-form (GFX_military_industrial_organization_*, GFX_mio_*); this tool names them GFX_mio_department_<asset>. Vanilla department icons fill the canvas (98 percent in the median of 337 files, most touch an edge), so the fit margin is 1 px.",
            ..base(MioDepartment, [48, 48], &[[48, 48]], "gfx/interface/military_industrial_organization/department_icons/{feature_slug}", "mio_department_", "interface/military_industrial_organization/{feature_slug}_mio.gfx")
        },
        Row {
            margin: 8,
            rules: MIO_TRAIT_RULES,
            negative: MIO_TRAIT_NEGATIVE,
            note: "Trait icons are 48x48. The trait and policy folders are measured together; policies are the 60x60 files. All 44 vanilla trait icons keep 8 px clear on their tightest side (the badge spans 32 of 48 px), where the in-game frame likely overlaps, so the fit margin is 8 px.",
            ..base(MioTrait, [48, 48], &[[48, 48]], "gfx/interface/military_industrial_organization/trait_icons/{feature_slug}", "mio_trait_", "interface/military_industrial_organization/{feature_slug}_mio.gfx")
        },
        Row {
            note: "Policy icons are the 60x60 files of the combined trait and policy measurement, which is why this row is a documented exception in the drift test.",
            rules: MIO_POLICY_RULES,
            negative: MIO_POLICY_NEGATIVE,
            ..base(MioPolicy, [60, 60], &[[60, 60]], "gfx/interface/military_industrial_organization/policy_icons/{feature_slug}", "mio_policy_", "interface/military_industrial_organization/{feature_slug}_mio.gfx")
        },
        Row {
            note: "Only the 72x88 side icons are generated; the multi-frame strips in the folder need separate frame art. One request makes the emblem's stages the way vanilla's pairs do (`bop_<TAG>_<name>_side` and `bop_<TAG>_<name>_improved_side`): the model draws one emblem in muted colour, the standard stage is its grey, the improved stage is the same emblem in its own colours with a pale gold glow, and the weakened stages (the same emblem cracked, as vanilla's `_bad_side` grey with no glow and `_bad_improved_side` in colour with a red glow) are made when processing is asked for them with cracked_stage. The sprites are GFX_bop_<name>_side, GFX_bop_<name>_improved_side, and with cracked_stage GFX_bop_<name>_bad_side and GFX_bop_<name>_bad_improved_side.",
            rules: BOP_SIDE_RULES,
            negative: BOP_SIDE_NEGATIVE,
            sprite: Some(("interface/{feature_slug}_bop.gfx", "GFX_{prefix}{asset}_side", &[])),
            margin: 7,
            ..base(BopSide, [72, 88], &[[72, 88]], "gfx/interface/bop/{feature_slug}", "bop_", "interface/{feature_slug}_bop.gfx")
        },
        Row {
            sprite: Some(("interface/{feature_slug}_medals.gfx", "GFX_medal_icon_{asset}_large", &[])),
            note: "A unit medal: script names GFX_medal_icon_<x> with frame = N, and every vanilla GFX_medal_icon_* has a GFX_medal_icon_*_large twin at twice the size (28x42 and 56x84 frames). This tool makes one 56x84 frame registered as the _large sprite and derives the 28x42 GFX_medal_icon_<asset> from it, the way vanilla's small strips are its large ones downscaled; script uses frame = 1. The medal is fitted into vanilla's box, 33x56 in the 56x84 frame (the median of the 85 frames of vanilla's large strips), not edge to edge. The measured dominant 16x28 files are older medal sprites that no vanilla script or GUI names.",
            ..base(Medal, [56, 84], &[[56, 84]], "gfx/interface/medals/{feature_slug}", "medal_icon_", "interface/{feature_slug}_medals.gfx")
        },
        Row {
            margin: 8,
            rules: AGENCY_UPGRADE_RULES,
            negative: AGENCY_UPGRADE_NEGATIVE,
            note: "Vanilla names these GFX_agency_*. All 15 vanilla upgrades keep exactly 8 px clear (the medallion spans 52 of 68 px), where the in-game frame likely overlaps, so the fit margin is 8 px.",
            ..base(AgencyUpgrade, [68, 68], &[[68, 68]], "gfx/interface/operatives/icons/{feature_slug}", "agency_", "interface/{feature_slug}_agency_upgrades.gfx")
        },
        Row {
            sprite: Some(("interface/{feature_slug}_operations.gfx", "GFX_operations_{asset}", &[])),
            background: Background::Opaque,
            margin: 0,
            checks: CARD_CHECKS,
            rules: OPERATION_ICON_RULES,
            negative: OPERATION_ICON_NEGATIVE,
            note: "The 85x85 operation icon: an operation's icon = GFX_operations_<x> and its map_icon = GFX_operations_<x>_map (vanilla common/operations/00_operations.txt, 17 pairs). The model draws an opaque square close-up and the operation_card step turns it into vanilla's slightly tilted, outlined card with transparent corners and a soft shadow, so the art is checked for the corners, never for opacity. The 48x48 map icon (the same picture in a round gold ring) is derived from the finished card as a companion file map/<x>_map and sprite GFX_operations_<x>_map. The 59x58 phase icons (GFX_phase_<x>_small) beside the phase pictures are another sprite and are not generated.",
            ..base(OperationIcon, [85, 85], &[[85, 85]], "gfx/interface/operations/{feature_slug}", "", "interface/{feature_slug}_operations.gfx")
        },
        Row {
            rules: COMMANDER_TRAIT_RULES,
            negative: COMMANDER_TRAIT_NEGATIVE,
            note: "Commander and personal traits are both 23x33 and both named GFX_trait_*; the script slot (trait group) tells them apart.",
            ..base(CommanderTrait, [23, 33], &[[23, 33]], "gfx/interface/traits/{feature_slug}", "trait_", "interface/{feature_slug}_traits.gfx")
        },
        Row {
            note: "See commander_trait: same size and naming, different script slot. Vanilla keeps them in traits/personal.",
            ..base(PersonalTrait, [23, 33], &[[23, 33]], "gfx/interface/traits/{feature_slug}/personal", "trait_", "interface/{feature_slug}_traits.gfx")
        },
        Row {
            margin: 1,
            rules: DOCTRINE_RULES,
            note: "Only the 64x64 doctrine icons are generated; the wide 212x83 and banner art in the folder are interface pieces. Vanilla draws the units as black silhouettes with a pale outline and glow (luminance 180 at alpha 136 one pixel out, alpha 37 two pixels out) and the warm accents with a dark shadow; the doctrine recipe's doctrine_look step turns any subject into that. Vanilla's glow runs close to the canvas edge: 55 of the 105 icons touch it and 73 keep at most 1 px clear, so the checked margin is 1 px, and the subject is fitted 3 px further in, the reach of the shadow around an accent, so the glow keeps it.",
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
            rules: OPERATION_PICTURE_RULES,
            negative: OPERATION_PICTURE_NEGATIVE,
            checks: CARD_CHECKS,
            note: "LOW CONFIDENCE. The 210x176 operation phase picture: a phase's picture names it in full, and all 71 vanilla ones are GFX_phase_<x> (GFX_operations_* are the 85x85 and 48x48 operation icons). The model draws an opaque photograph and the phase_card step turns it into vanilla's tilted card with transparent corners, so the art is checked for the corners, never for opacity. Vanilla's 71 phase pictures (operations/images) are sepia photographs on tilted worn cards with transparent corners, 86 percent of the canvas covered like the report event cards.",
            ..base(OperationPicture, [210, 176], &[[210, 176]], "gfx/interface/operations/{feature_slug}/pictures", "phase_", "interface/{feature_slug}_operations.gfx")
        },
        Row {
            background: Background::Transparent,
            margin: 2,
            checks: TECH_CHECKS,
            rules: TECHNOLOGY_RULES,
            negative: ICON_NEGATIVE,
            sprite: Some(("interface/{feature_slug}_technologies.gfx", "GFX_{asset}_medium", &[])),
            note: "The engine binds technology art by name: GFX_<technology key>_medium, so the asset name is the technology key. 131x52 (341 files) is the other common size. Measured transparent, so it follows the icon chain. Every inspected vanilla picture is a flat side profile facing right, and the subject's box is long and low (width over height: median 3.09, 10th percentile 2.30 at 146x54), which the side_view check uses as a hint. Tier numerals are baked into vanilla's textures, not drawn by the GUI; the process input overlay stamps them in vanilla's glyphs and place.",
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
            rules: COUNTER_RULES,
            negative: COUNTER_NEGATIVE,
            checks: COUNTER_CHECKS,
            note: "The battalion icon: a strip of two 76x42 frames (152x42, noOfFrames = 2). Frame 1 is the unit's side-view silhouette in vanilla's fixed green material (red equals blue, lit from above, a soft black halo), frame 2 the NATO box (the same grey box in all 111 vanilla strips) with the sub-unit's symbol. The model draws only a black silhouette; the unit_counter recipe renders both frames, with the symbol given as counter_symbol or inferred from the asset name. An imported finished strip is kept only when it already is vanilla's material (the unit_material check); any other is drawn again from its frame 1. The game looks it up as GFX_unit_<sub-unit key>_icon_medium, so the asset name is the sub-unit key (not its sprite key).",
            ..base(CounterLarge, [76, 42], &[[76, 42]], "gfx/interface/counters/divisions_large/{feature_slug}", "unit_", "")
        },
        Row {
            sprite: Some(("interface/{feature_slug}_counters.gfx", "GFX_unit_{asset}_icon_medium_white", &[])),
            rules: COUNTER_RULES,
            negative: COUNTER_NEGATIVE,
            checks: COUNTER_CHECKS,
            note: "The on-map counter: a strip of two 30x12 frames (60x12, noOfFrames = 2), light grey because the game multiplies it by the country colour. Generated like the large counter: the silhouette is area-averaged down to vanilla's 9 px height and rendered grey with a tight dark halo, frame 2 is the small NATO box. The game looks it up as GFX_unit_<sub-unit key>_icon_medium_white; the tooltip text icon GFX_unit_<sub-unit key>_icon_small (60x12, 2 frames, under gfx/texticons) is derived from it: frame 1 tinted green as vanilla's are, frame 2 kept. An imported finished strip is kept only when it already is vanilla's grey material; any other is drawn again from its frame 1.",
            ..base(CounterSmall, [30, 12], &[[30, 12]], "gfx/interface/counters/divisions_small/{feature_slug}", "onmap_unit_", "")
        },
        Row {
            frames: 1,
            margin: 1,
            sprite: Some(("interface/{feature_slug}_counters.gfx", "GFX_div_templ_{n}_large", &[])),
            note: "Single frame, 76x42 (the large division template symbol). The game offers the symbols as GFX_div_templ_<n>_large with GFX_div_templ_<n>_small (30x12); vanilla uses 0-43 and 65-121. Install picks n, the next number after vanilla's 121 and every number the mod or an earlier install already uses, records it, and derives the grey 30x12 small symbol. The model draws a black silhouette; the division_symbol recipe fits it at vanilla's size (median ink area, centred at 38, 21.5) and renders vanilla's green material. An imported finished symbol is kept only when it already is vanilla's material; any other is drawn again as a silhouette.",
            checks: COUNTER_CHECKS,
            rules: EMBLEM_RULES,
            negative: EMBLEM_NEGATIVE,
            ..base(DivisionEmblem, [76, 42], &[[76, 42]], "gfx/interface/counters/division_templates_large/{feature_slug}", "custom_", "")
        },
        Row {
            automation: Automation::Generate,
            rules: BUILDING_RULES,
            negative: BUILDING_NEGATIVE,
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
            rules: GUI_TEXTURE_RULES,
            negative: GUI_TEXTURE_NEGATIVE,
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
        Row {
            background: Background::Opaque,
            margin: 0,
            checks: MODEL_CHECKS,
            rules: MODEL_RULES,
            negative: MODEL_NEGATIVE,
            sprite: None,
            note: "Not game art: the input picture of an image-to-3D tool (Meshy), one whole subject on a plain light grey backdrop. The caller sets the canvas (portrait 2:3 for figures, landscape for animals and vehicles). Only the caller's own style anchors are sent, the raw or processed PNG is the result, and install refuses it.",
            ..base(ModelReference, [0, 0], &[], "docs/assets/{feature_slug}/model_reference", "model_reference_", "")
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

/// The rubric id of a family: one per group (officer spirits and every other icon get the icon rubric). v3 asks
/// every question against the vanilla references beside the candidate in the preview.
pub fn rubric_of(family: Family) -> String {
    match family {
        Family::ModelReference => "interface_model_v3".into(),
        _ => format!("{}_v3", family.group().as_str()),
    }
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
            let small = is_small_icon(r.family, canvas);
            let mut checks: Vec<String> = r.checks.iter().map(|s| s.to_string()).collect();
            let mut rules: Vec<String> = r.rules.iter().map(|s| s.to_string()).collect();
            let mut negative: Vec<String> = r.negative.iter().map(|s| s.to_string()).collect();
            if small {
                checks.insert(checks.len().saturating_sub(2), "small_detail".to_string());
                rules.insert(rules.len().saturating_sub(1), SMALL_CANVAS_RULE.to_string());
                negative.extend(SMALL_CANVAS_NEGATIVE.iter().map(|s| s.to_string()));
            }
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
                    checks,
                    reference_dirs: vec![format!("references/{}", r.family.as_str())],
                    rubric: rubric_of(r.family),
                    prompt: PromptSpec { rules, negative },
                    grayscale: r.grayscale,
                    source_canvas: r.source_canvas,
                    animation: animation_of(r.family),
                },
                pack: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn portraits_are_close_on_vanillas_backdrop_and_spirits_only_need_to_read() {
        let set = by_family();
        for f in [Family::LeaderPortrait, Family::CommanderPortrait, Family::OperativePortrait, Family::ScientistPortrait] {
            let r = set[&f].imagegen.prompt.rules.join(" ");
            assert!(r.contains("close head-and-shoulders") && r.contains("face about half the height"), "{f}");
            assert!(r.contains("portrait backdrop: ") && r.contains("council or otherwise unusual"), "{f}");
            assert!(!r.contains("slightly textured backdrop"), "{f}");
            // each family names its own vanilla backdrop
            let own = match f {
                Family::ScientistPortrait => "pale blue-grey (about RGB 135, 171, 193",
                Family::OperativePortrait => "dark warm charcoal studio backdrop with a soft lighter spotlight behind the figure",
                _ => "pale cream-grey (about RGB 229, 226, 209",
            };
            assert!(r.contains(own), "{f}: {r}");
            assert!(set[&f].imagegen.checks.iter().any(|c| c == "backdrop"), "{f}");
        }
        // idea spirits may be scenes or emblems: the rule allows both and asks only for readability
        let r = set[&Family::IdeaSpirit].imagegen.prompt.rules.join(" ");
        assert!(r.contains("or a small scene") && r.contains("readable at {w}x{h} pixels"), "{r}");
        assert!(!r.contains("never a scene"));
        let focus = set[&Family::Focus].imagegen.prompt.rules.join(" ");
        assert!(focus.contains("invented colours, never a real country's flag, unless the brief names that country"));
    }

    #[test]
    fn portraits_ask_for_an_invented_person() {
        // the backend refused fictional leaders described by nation and office as likenesses of real people
        for rules in [super::LEADER_PORTRAIT_RULES, super::SCIENTIST_PORTRAIT_RULES, super::OPERATIVE_PORTRAIT_RULES, super::ADVISOR_RULES] {
            assert!(rules.iter().any(|r| r.contains("must not resemble any real historical figure")));
        }
    }

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
    /// * `operation_icon`: the operation folder is dominated by the 210x176 pictures and the 59x58 phase icons (71 files
    ///   each); the operation icons are the 20 files of 85x85.
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
                    assert!(n < dom_n, "operation_icon is the 85x85 minority of its folder");
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
            if matches!(f, Family::ReportEvent | Family::OperationPicture | Family::OperationIcon | Family::AdvisorCard) {
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
    fn the_documented_family_notes_hold() {
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
        // every family is generated end to end: the counters and the division symbol from a silhouette the unit art
        // recipes finish in vanilla's material
        for (f, p) in by_family() {
            assert_eq!(p.imagegen.automation, Automation::Generate, "{f}");
        }
        for f in [Family::CounterLarge, Family::CounterSmall, Family::DivisionEmblem] {
            let rules = by_family()[&f].imagegen.prompt.rules.join(" ");
            assert!(rules.contains("solid black silhouette") && rules.contains("transparent"), "{f}: {rules}");
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
            assert_eq!(p.imagegen.rubric, rubric_of(f), "{f}");
            assert!(p.imagegen.rubric.starts_with(f.group().as_str()), "{f}");
            if p.imagegen.background == Background::Opaque {
                // cards keep transparent corners on purpose: they are checked for the corners instead of for opacity
                let card = matches!(f, Family::ReportEvent | Family::OperationPicture | Family::OperationIcon | Family::AdvisorCard);
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

    /// Words that can trip a provider's violence moderation stay out of the built-in rules: the forms name equipment,
    /// vehicles and insignia in neutral words. The MIO department rule keeps the model off paper and scenes.
    #[test]
    fn rules_use_neutral_words_and_the_mio_department_asks_for_the_equipment_itself() {
        let charged = ["gun", "guns", "weapon", "weapons", "kill", "blood", "attack", "bomb", "bombs", "explosion", "combat", "war"];
        for (f, p) in by_family() {
            let text = p.imagegen.prompt.rules.join(" ").to_ascii_lowercase();
            let hit: Vec<&str> = text.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| charged.contains(w)).collect();
            assert!(hit.is_empty(), "{f}: the rules use {hit:?}: {text}");
        }
        let mio = &by_family()[&Family::MioDepartment];
        let rules = mio.imagegen.prompt.rules.join(" ");
        assert!(rules.contains("Depict the equipment itself") && rules.contains("design bureau"), "{rules}");
        let negative = mio.imagegen.prompt.negative.join("; ");
        for n in ["a scene", "blueprint", "panels, dividers", "a second main object"] {
            assert!(negative.contains(n), "{n}: {negative}");
        }
        let sp = &by_family()[&Family::SpecialProjectIcon];
        assert!(sp.imagegen.prompt.rules.join(" ").contains("line drawing"));
        assert!(sp.imagegen.prompt.negative.iter().any(|n| n.contains("shading")));
    }

    /// The families whose output drifted from vanilla's form state that form in their first rule, and
    /// nothing in their rules or negatives contradicts it. The phrases are the measured vanilla form (HOI4 1.19.4.1).
    #[test]
    fn reviewed_families_state_their_vanilla_form() {
        let set = by_family();
        let cases: [(Family, &str, &[&str]); 14] = [
            (
                Family::Focus,
                "national focus icon form: one emblem",
                &["do not share one frame", "round plaque or medallion", "heraldic shield", "no frame at all", "Pick the form", "invented colours"],
            ),
            (
                Family::DecisionCategoryIcon,
                "decision category icon form: one symbol on a small round disc",
                &["steel blue, dark red, tan and dark grey"],
            ),
            (
                Family::AgencyUpgrade,
                "intelligence agency upgrade form: a round painted vignette",
                &["no ring, rim, frame or border around the disc", "Rich but sober colour", "no overall orange or sepia cast", "no crowded room"],
            ),
            (
                Family::Building,
                "landmark building form: one real-world landmark",
                &["no ground, rock or hill", "Frame 2", "ruins", "orange flame", "at most about four times as wide as it is tall", "foreshortened"],
            ),
            (Family::CommanderTrait, "unit leader trait form: a small military medal", &["ribbon", "monochrome metal"]),
            (Family::DivisionEmblem, "division template symbol form", &["solid black silhouette"]),
            (
                Family::MioTrait,
                "trait icon form: one small, simple glyph",
                &["drawn flat", "light shading", "Light tones overall", "little dark area", "no badge, disc, plate"],
            ),
            (
                Family::OperationIcon,
                "operation icon form: one square picture",
                &["warm, muted colour", "only this form", "isolated object on a blank background"],
            ),
            (Family::OperationPicture, "operation phase picture form: one period photograph", &["only this form", "operation_icon"]),
            (Family::GuiTexture, "GUI texture form: flat interface material", &["straight on"]),
            (Family::MioDepartment, "department icon form: ONE piece of equipment", &["one bold, saturated colour", "never grey, metallic"]),
            (Family::MioPolicy, "policy icon form: one object", &["No perspective depth"]),
            (Family::BopSide, "balance of power side form: one person", &["head-and-shoulders portrait", "muted natural colours"]),
            (Family::StateModifier, "state modifier icon form: one bold, simple object", &["at most two elements", "no small details"]),
        ];
        for (f, form, more) in cases {
            let p = &set[&f].imagegen.prompt;
            assert!(p.rules[0].contains(form), "{f}: first rule {}", p.rules[0]);
            let rules = p.rules.join(" ");
            for m in more {
                assert!(rules.contains(m), "{f}: {m} missing from {rules}");
            }
            // the old rules asked for a loose subject with no frame, which is not these families' form
            assert!(!rules.contains("One centred subject"), "{f}");
        }
        let neg = |f: Family| set[&f].imagegen.prompt.negative.join("; ");
        assert!(!neg(Family::MioTrait).contains("a frame or border") && neg(Family::MioTrait).contains("round badge"));
        assert!(neg(Family::OperationIcon).contains("isolated object") && neg(Family::OperationIcon).contains("round vignette"));
        assert!(neg(Family::OperationPicture).contains("round vignette") && neg(Family::OperationPicture).contains("painted illustration"));
        assert!(neg(Family::CommanderTrait).contains("natural colours"));
        assert!(!neg(Family::GuiTexture).contains("frame"), "a frame texture is a frame");
        assert!(neg(Family::Building).contains("hill, landscape") && neg(Family::Building).contains("diorama base"));
        assert!(neg(Family::BopSide).contains("cracks") && neg(Family::MioPolicy).contains("view into the distance"));
        assert!(neg(Family::StateModifier).contains("small details") && neg(Family::MioDepartment).contains("grey or metallic"));
        // one form, one size: the 59x58 phase icons are another sprite
        assert_eq!(set[&Family::OperationIcon].accepted, vec![[85, 85]]);
        // the form rule of each GUI texture kind
        for k in crate::family::GuiTextureKind::ALL {
            assert!(k.form_rule().contains(&format!("{} form", k.as_str().replace("background", "window background"))), "{k}");
        }
    }

    /// The 0.3.2 gallery's agency upgrades measured dark, warm and busy, and its MIO traits were dark painted objects,
    /// and thin parts do not read at 32x32. The negatives name those, and every icon family with a side of 40 px or less
    /// gets the small-canvas rule, its negatives and the `small_detail` check, and no other family does.
    #[test]
    fn small_icons_get_the_readability_rule_and_the_0_3_2_drift_is_named() {
        let set = by_family();
        let neg = |f: Family| set[&f].imagegen.prompt.negative.join("; ");
        assert!(
            neg(Family::AgencyUpgrade).contains("warm brown and orange")
                && neg(Family::AgencyUpgrade).contains("haze")
                && neg(Family::AgencyUpgrade).contains("many small objects")
        );
        assert!(neg(Family::MioTrait).contains("dark, murky") && neg(Family::MioTrait).contains("fine mechanical detail"));
        assert!(!neg(Family::MioTrait).contains("without shading"), "vanilla's glyphs are flat with light shading");
        assert!(neg(Family::Building).contains("long, thin panorama"));
        let small: Vec<Family> = set.values().filter(|p| super::is_small_icon(p.family, p.canvas)).map(|p| p.family).collect();
        assert_eq!(
            small,
            vec![Family::Decision, Family::DecisionCategoryIcon, Family::StateModifier, Family::Raid, Family::CommanderTrait, Family::PersonalTrait]
        );
        for (f, p) in &set {
            let rules = p.imagegen.prompt.rules.join(" ");
            let is_small = small.contains(f);
            assert_eq!(rules.contains("Small canvas: at"), is_small, "{f}");
            assert_eq!(p.imagegen.checks.iter().any(|c| c == "small_detail"), is_small, "{f}");
            assert_eq!(p.imagegen.prompt.negative.iter().any(|n| n.contains("thin parallel lines")), is_small, "{f}");
            if is_small {
                assert!(rules.contains("no thin parallel lines") && rules.contains("one bold object"), "{f}");
                assert!(crate::validate::small_detail_limit(*f).is_some(), "{f} has a measured limit");
                // the margin rule stays last
                assert!(p.imagegen.prompt.rules.last().unwrap().contains("margin"), "{f}");
            }
        }
    }

    // ------------------------------------------------------------ emblem scale against the installed game

    fn vanilla_files(folder: &str, prefix: &str, size: [u32; 2]) -> Vec<crate::pixels::Rgba> {
        let g = std::env::var("HOI4_GAME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| "C:/Program Files (x86)/Steam/steamapps/common/Hearts of Iron IV".into());
        let mut stack = vec![g.join(folder)];
        let mut paths = Vec::new();
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "dds") && p.file_name().unwrap().to_string_lossy().starts_with(prefix) {
                    paths.push(p);
                }
            }
        }
        paths.sort();
        // size [0, 0] asks for the large medal strips: 84 px tall, a whole number of 56 px frames
        let wanted = |img: &crate::pixels::Rgba| {
            if size == [0, 0] {
                img.height == 84 && img.width >= 56 && img.width.is_multiple_of(56)
            } else {
                [img.width, img.height] == size
            }
        };
        paths.iter().filter_map(|p| crate::dds::decode(&std::fs::read(p).ok()?).ok()).filter(|img| wanted(img)).collect()
    }

    /// The 56x84 frames of vanilla's large medal strips (every vanilla medal is a strip of such frames).
    fn medal_frames() -> Vec<crate::pixels::Rgba> {
        let mut out = Vec::new();
        for strip in vanilla_files("gfx/interface/medals", "", [0, 0]) {
            out.extend(strip.split_frames(strip.width / 56).unwrap_or_default());
        }
        out
    }

    /// The emblem of the plaque, disc, badge and pictogram families is fitted at vanilla's size: for each vanilla file
    /// a solid subject of the same aspect goes through the family's own finishing, and the median span (the larger of
    /// the width and height shares of the visible box) is compared with vanilla's median. The first five are never
    /// smaller than vanilla's by more than one pixel; the last five, fitted to vanilla's margins, are within one pixel
    /// either way. Measured on HOI4 1.19.4.1: focus 0.966, decision category icon 0.950, MIO department 0.979,
    /// doctrine 0.953, commander trait 0.909, MIO trait 0.667, agency upgrade 0.765, raid
    /// 0.719, idea concern 0.906.
    #[test]
    fn plaque_and_disc_emblems_are_fitted_at_vanillas_size() {
        // (family, folder, file prefix, also not larger than vanilla)
        let cases: [(Family, &str, &str, bool); 10] = [
            (Family::Focus, "gfx/interface/goals", "goal", false),
            (Family::DecisionCategoryIcon, "gfx/interface/decisions", "decision_category", false),
            (Family::MioDepartment, "gfx/interface/military_industrial_organization/department_icons", "", false),
            (Family::Doctrine, "gfx/interface/doctrines", "", false),
            (Family::CommanderTrait, "gfx/interface/traits", "trait_", false),
            (Family::MioTrait, "gfx/interface/military_industrial_organization/trait_icons", "", true),
            (Family::AgencyUpgrade, "gfx/interface/operatives/icons", "", true),
            (Family::Raid, "gfx/interface/military_raids", "", true),
            (Family::IdeaConcern, "gfx/interface/ideas", "idea_", true),
            (Family::Medal, "gfx/interface/medals", "", true),
        ];
        let span = |img: &crate::pixels::Rgba| {
            let b = img.alpha_bbox(16).unwrap();
            (f64::from(b[2] - b[0]) / f64::from(img.width)).max(f64::from(b[3] - b[1]) / f64::from(img.height))
        };
        let median = |mut v: Vec<f64>| {
            v.sort_by(f64::total_cmp);
            v[v.len() / 2]
        };
        let set = by_family();
        let mut report = String::new();
        for (family, folder, prefix, both_ways) in cases {
            let p = &set[&family];
            let mut files = if family == Family::Medal { medal_frames() } else { vanilla_files(folder, prefix, p.canvas) };
            if family == Family::Focus {
                files.extend(vanilla_files(folder, "focus", p.canvas));
            }
            if files.len() < 10 {
                eprintln!("SKIPPED emblem scale vs vanilla ({family}): Hearts of Iron IV not found (set HOI4_GAME_DIR)");
                continue;
            }
            let mut van = Vec::new();
            let mut ours = Vec::new();
            // vanilla's span over every file; ours over an even sample of at most 40 shapes, which keeps the test
            // quick in debug builds
            let step = files.len().div_ceil(40);
            van.extend(files.iter().filter(|f| f.alpha_bbox(16).is_some()).map(span));
            for f in files.iter().step_by(step).filter(|f| f.alpha_bbox(16).is_some()) {
                let b = f.alpha_bbox(16).unwrap();
                // the same shape, eight times larger, as a generated cut-out
                let (w, h) = ((b[2] - b[0]) * 8, (b[3] - b[1]) * 8);
                let mut src = crate::pixels::Rgba::new(w + 40, h + 40);
                for y in 20..20 + h {
                    for x in 20..20 + w {
                        src.set_px(x, y, [120, 100, 80, 255]);
                    }
                }
                let plan = crate::chain::plan_finishing(p, p.canvas, &src, false, false, "").unwrap();
                let (out, _) = crate::process::run_chain(&src, &plan.ops).unwrap();
                ours.push(span(&out));
            }
            let (v, o) = (median(van), median(ours));
            let px = 1.0 / f64::from(p.canvas[0].min(p.canvas[1]));
            report.push_str(&format!("{family}: vanilla {v:.3}, ours {o:.3} ({} files)\n", files.len()));
            assert!(o >= v - px - 1e-9, "{family}: our emblem spans {o:.3} of the canvas, vanilla's {v:.3}\n{report}");
            if both_ways {
                assert!(o <= v + px + 1e-9, "{family}: our emblem spans {o:.3} of the canvas, larger than vanilla's {v:.3}\n{report}");
            }
        }
        eprintln!("emblem span vs vanilla (median)\n{report}");
    }
}
