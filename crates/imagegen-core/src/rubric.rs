//! Visual review rubrics.
//!
//! Deterministic checks cannot judge composition, style or readability at game size. Those questions go to a
//! reviewer (a person, or an agent that can look at the preview) and are recorded next to the deterministic
//! results. An approval needs every question answered `true`; anything else is `changes_requested`.
//!
//! The v3 rubrics (0.3.2) ask every question against the vanilla references that `hoi4.image_preview` puts beside
//! the candidate, because the 0.3.1 questions ("does the style match the approved references?") let a reviewer
//! approve art that looked nothing like vanilla. The answers are no longer enough on their own: the review also runs
//! the measured vanilla-match check (`crate::vanillamatch`). v1 and v2 stay readable for reviews recorded earlier.
//!
//! Every v3 rubric starts with [`SUBJECT`]: does the image show what the brief asks for? The measured checks compare
//! form, medium and framing with vanilla, and none of them can tell a tank from a tractor, so subject fidelity is a
//! reviewer's answer that an approval needs. The review reply quotes the brief beside the question.

use serde_json::Value;

/// Question key and the question as shown to the reviewer.
pub type Question = (&'static str, &'static str);

/// icon_v2: every icon family has a vanilla form (a loose subject, a plaque, a disc, a pictogram, a ribbon medal, line
/// art), stated in the plan. A frame is unwanted only where the form has none.
const ICON: &[Question] = &[
    ("readable_at_native_size", "Is the subject recognizable at the native game size (look at the 1x preview, not only the enlarged one)?"),
    ("family_form", "Does it have the family's vanilla form stated in the plan rules (for example a focus plaque, a decision category disc, a raid pictogram, a trait ribbon medal, blueprint line art, or a loose subject)?"),
    ("silhouette_clear", "Is the silhouette clear and simple?"),
    ("style_matches_references", "Does the painting style match the approved references side by side?"),
    ("no_text_or_watermark", "Is the art free of text, letters, numbers, signatures and watermarks?"),
    ("no_unintended_elements", "Is it free of drawn checkerboards, square backdrops and frames or plates that the family's form does not call for?"),
    ("period_and_theme_fit", "Does it fit the period and the theme of the feature?"),
];

/// icon_v1, kept so reviews recorded before icon_v2 still read.
const ICON_V1: &[Question] = &[
    ("readable_at_native_size", "Is the subject recognizable at the native game size (look at the 1x preview, not only the enlarged one)?"),
    ("silhouette_clear", "Is the silhouette clear and simple?"),
    ("style_matches_references", "Does the painting style match the approved references side by side?"),
    ("no_text_or_watermark", "Is the art free of text, letters, numbers, signatures and watermarks?"),
    ("no_unintended_elements", "Is it free of frames, plates, drawn checkerboards and other additions that were not asked for?"),
    ("period_and_theme_fit", "Does it fit the period and the theme of the feature?"),
];

const PICTURE: &[Question] = &[
    ("composition_fills_frame", "Does the composition suit the wide or tall crop and fill the frame?"),
    ("readable_at_native_size", "Is the main subject readable at the native game size?"),
    ("style_matches_references", "Does the style match the approved references side by side?"),
    ("no_text_or_watermark", "Is the art free of text, captions, signatures and watermarks?"),
    ("period_and_theme_fit", "Does it fit the period and the theme of the feature?"),
    ("treatment_correct", "Is the required treatment right (black and white for news; report-card treatment is applied afterwards by the kit tool)?"),
];

const FLAG: &[Question] = &[
    ("readable_at_normal_size", "Is the design clear at 82x52?"),
    ("readable_at_small_size", "Is it still recognizable at 10x7?"),
    ("distinct_from_other_flags", "Is it distinct from the other flags in the mod?"),
    ("no_text_or_watermark", "Is it free of text and watermarks?"),
];

const PORTRAIT: &[Question] = &[
    ("face_natural", "Does the face look natural, without generation artifacts?"),
    ("framing_correct", "Is the framing right for a tall portrait?"),
    ("style_matches_references", "Does the style match the approved references side by side?"),
    ("period_clothing", "Is the clothing appropriate to the period?"),
    ("no_text_or_watermark", "Is the art free of text and watermarks?"),
];

const GENERIC: &[Question] = &[
    ("looks_correct", "Does the art look right for its surface at the native size?"),
    ("matches_references", "Does it match the approved references?"),
    ("no_text_or_watermark", "Is it free of text and watermarks?"),
];

const UNIT: &[Question] = &[
    ("readable_at_native_size", "Is the subject recognizable at the native game size (look at the 1x preview, not only the enlarged one)?"),
    ("frames_consistent", "Do all frames share the same size, position and style, and does the strip read correctly frame by frame?"),
    ("style_matches_references", "Does the painting style match the approved references side by side?"),
    ("no_text_or_watermark", "Is the art free of text, letters, numbers, signatures and watermarks?"),
    ("no_unintended_elements", "Is it free of frames, plates, drawn checkerboards and other additions that were not asked for?"),
];

const INTERFACE: &[Question] = &[
    ("fits_the_surface", "Does the art fit the interface element it is made for, at the exact size the mod defined?"),
    ("style_matches_references", "Does the style match the approved references or the surrounding interface?"),
    ("no_text_or_watermark", "Is the art free of text and watermarks?"),
];

const ANIMATION: &[Question] = &[
    ("readable_at_native_size", "Is the subject recognizable at the native game size in every frame (look at the 1x contact sheet)?"),
    ("motion_reads_as_intended", "Watching the looping preview, does the motion read as intended (a pulse, a scroll, a turn, the change between key frames) without a jump or a flash?"),
    ("frames_consistent", "Do the frames keep one subject, one position and one palette, so nothing but the intended motion changes?"),
    ("loop_is_seamless", "Does the last frame run into the first without a visible seam?"),
    ("style_matches_references", "Does the style match the approved references or the surrounding interface?"),
    ("no_text_or_watermark", "Is the art free of text, letters, numbers, signatures and watermarks?"),
];

const VANILLA_FORM: Question = (
    "vanilla_form",
    "Next to the vanilla references in the preview, does it have the same form (for example a focus plaque, a decision category disc, a trait ribbon medal, special project line art without any fill, a raid pictogram, or a loose subject where vanilla has one)?",
);
const VANILLA_MEDIUM: Question = (
    "vanilla_medium",
    "Next to the vanilla references, is it in the same medium and palette (painted metal and enamel, a flat pictogram, pale line art, black units with a white outline and a glow, and so on), with no fill, texture or colour vanilla does not have?",
);
const VANILLA_FRAMING: Question = (
    "vanilla_framing",
    "Next to the vanilla references, does it sit at the same size and position on the canvas, with the same margins and no backdrop, plate or frame that vanilla does not have?",
);
const NO_TEXT: Question = (
    "no_text_or_watermark",
    "Is the art free of text, letters, numbers, signatures and watermarks (unless the vanilla references carry the same marks)?",
);
const READABLE: Question =
    ("readable_at_native_size", "Is the subject recognizable at the native game size (the 1x panels), as the vanilla references are?");
const PERIOD: Question = ("period_and_theme_fit", "Does it fit the period and the theme of the feature?");
/// The subject question of every v3 rubric.
pub const SUBJECT: Question = (
    "shows_brief_subject",
    "Does the image show the subject the brief asks for (the review reply quotes the brief), with nothing that contradicts it? The measured checks cannot judge this.",
);

/// icon_v3: form, medium and framing against the vanilla references beside the candidate.
const ICON_V3: &[Question] = &[SUBJECT, VANILLA_FORM, VANILLA_MEDIUM, VANILLA_FRAMING, READABLE, NO_TEXT, PERIOD];

const PICTURE_V3: &[Question] = &[
    SUBJECT,
    (
        "vanilla_composition",
        "Next to the vanilla references in the preview, does it use the same composition, crop and viewing angle (for example technology pictures at vanilla's fixed side angle), filling the frame the way they do?",
    ),
    (
        "vanilla_medium",
        "Next to the vanilla references, is it in the same medium and treatment (painted, black and white for news, the report-card look, vanilla's overlays where they apply)?",
    ),
    READABLE,
    NO_TEXT,
    PERIOD,
];

const PORTRAIT_V3: &[Question] = &[
    SUBJECT,
    ("vanilla_framing", "Next to the vanilla references in the preview, are the head and shoulders framed at the same size and position?"),
    ("vanilla_medium", "Next to the vanilla references, is it painted in the same style, palette and lighting?"),
    ("face_natural", "Does the face look natural, without generation artifacts?"),
    ("period_clothing", "Is the clothing appropriate to the period?"),
    NO_TEXT,
];

const FLAG_V3: &[Question] = &[
    SUBJECT,
    (
        "vanilla_style",
        "Next to the vanilla references in the preview, is it a flat flag design in the same style (simple fields and emblems, no photo texture, no folds or pole)?",
    ),
    ("readable_at_normal_size", "Is the design clear at 82x52?"),
    ("readable_at_small_size", "Is it still recognizable at 10x7?"),
    ("distinct_from_other_flags", "Is it distinct from the other flags in the mod?"),
    ("no_text_or_watermark", "Is it free of text and watermarks?"),
];

const UNIT_V3: &[Question] = &[
    SUBJECT,
    VANILLA_FORM,
    ("vanilla_medium", "Next to the vanilla references, does it have the same colouring and processing (for example vanilla's green unit counters)?"),
    ("frames_consistent", "Do all frames share the same size, position and style, and does the strip read correctly frame by frame?"),
    READABLE,
    NO_TEXT,
];

const INTERFACE_V3: &[Question] = &[
    SUBJECT,
    (
        "vanilla_style",
        "Next to the vanilla interface references in the preview (buttons, frames, panels, backgrounds), is it in the same style and material?",
    ),
    ("fits_the_surface", "Does the art fit the interface element it is made for, at the exact size the mod defined?"),
    ("no_text_or_watermark", "Is the art free of text and watermarks?"),
];

const ANIMATION_V3: &[Question] = &[
    SUBJECT,
    ("vanilla_style", "Next to the vanilla references or the surrounding interface, is it in the same style, palette and brightness?"),
    ("readable_at_native_size", "Is the subject recognizable at the native game size in every frame (look at the 1x contact sheet)?"),
    (
        "motion_reads_as_intended",
        "Watching the looping preview, does the motion read as intended (a pulse, a scroll, a turn, the change between key frames) without a jump or a flash?",
    ),
    ("frames_consistent", "Do the frames keep one subject, one position and one palette, so nothing but the intended motion changes?"),
    ("loop_is_seamless", "Does the last frame run into the first without a visible seam?"),
    ("no_text_or_watermark", "Is the art free of text, letters, numbers, signatures and watermarks?"),
];

const GENERIC_V3: &[Question] = &[
    SUBJECT,
    ("vanilla_style", "Next to the vanilla references in the preview, is it in the same form, medium and framing?"),
    ("looks_correct", "Does the art look right for its surface at the native size?"),
    ("no_text_or_watermark", "Is it free of text and watermarks?"),
];

/// Questions for a rubric id. Rubrics are per group (`icon`, `picture`, `portrait`, `flag`, `unit`, `interface`,
/// `animation`) and versioned: `_v3` (current, asked against the vanilla references), `_v2` (icon only, the family
/// form question) and `_v1`. Every family of a group, officer corps spirits included, gets its group's questions.
/// Anything else falls back to the generic questions.
pub fn questions(rubric: &str) -> &'static [Question] {
    if rubric == "icon_v1" {
        return ICON_V1;
    }
    if let Some(group) = rubric.strip_suffix("_v3") {
        return match group {
            "icon" => ICON_V3,
            "picture" => PICTURE_V3,
            "portrait" => PORTRAIT_V3,
            "flag" => FLAG_V3,
            "unit" => UNIT_V3,
            "interface" => INTERFACE_V3,
            "animation" => ANIMATION_V3,
            _ => GENERIC_V3,
        };
    }
    match rubric.strip_suffix("_v2").or_else(|| rubric.strip_suffix("_v1")).unwrap_or(rubric) {
        "icon" => ICON,
        "picture" => PICTURE,
        "portrait" => PORTRAIT,
        "flag" => FLAG,
        "unit" => UNIT,
        "interface" => INTERFACE,
        "animation" => ANIMATION,
        _ => GENERIC,
    }
}

/// Outcome of checking answers against a rubric.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswerCheck {
    /// Questions not answered at all.
    pub missing: Vec<&'static str>,
    /// Questions answered anything other than `true`.
    pub failed: Vec<&'static str>,
}

impl AnswerCheck {
    /// True when an approval is justified.
    pub fn all_true(&self) -> bool {
        self.missing.is_empty() && self.failed.is_empty()
    }
}

/// Compare an answers object with the rubric.
pub fn check_answers(rubric: &str, answers: &Value) -> AnswerCheck {
    let mut missing = Vec::new();
    let mut failed = Vec::new();
    for (key, _) in questions(rubric) {
        match answers.get(*key) {
            None | Some(Value::Null) => missing.push(*key),
            Some(Value::Bool(true)) => {}
            Some(_) => failed.push(*key),
        }
    }
    AnswerCheck { missing, failed }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_v3_rubric_asks_for_the_brief_subject_and_an_approval_needs_it() {
        for g in ["icon", "picture", "portrait", "flag", "unit", "interface", "animation", "other"] {
            let id = format!("{g}_v3");
            assert_eq!(questions(&id)[0], SUBJECT, "{id}");
            let mut answers: serde_json::Map<String, Value> = questions(&id).iter().map(|(k, _)| (k.to_string(), json!(true))).collect();
            assert!(check_answers(&id, &Value::Object(answers.clone())).all_true());
            answers.remove(SUBJECT.0);
            assert_eq!(check_answers(&id, &Value::Object(answers.clone())).missing, vec![SUBJECT.0], "{id}");
            answers.insert(SUBJECT.0.into(), json!(false));
            assert_eq!(check_answers(&id, &Value::Object(answers)).failed, vec![SUBJECT.0], "{id}");
        }
        assert!(!questions("icon_v2").contains(&SUBJECT), "reviews recorded under v2 keep their questions");
    }

    #[test]
    fn every_group_has_its_rubric_and_every_family_resolves_to_a_real_one() {
        assert!(questions("icon_v1").iter().any(|q| q.0 == "silhouette_clear"));
        // icon_v2 adds the family-form question; icon_v1 stays readable for older reviews
        assert!(questions("icon_v2").iter().any(|q| q.0 == "family_form"));
        assert!(!questions("icon_v1").iter().any(|q| q.0 == "family_form"));
        assert!(questions("picture_v1").iter().any(|q| q.0 == "composition_fills_frame"));
        assert!(questions("flag_v1").iter().any(|q| q.0 == "readable_at_small_size"));
        assert!(questions("portrait_v1").iter().any(|q| q.0 == "face_natural"));
        assert!(questions("unit_v1").iter().any(|q| q.0 == "frames_consistent"));
        assert!(questions("interface_v1").iter().any(|q| q.0 == "fits_the_surface"));
        assert!(questions("animation_v1").iter().any(|q| q.0 == "loop_is_seamless"));
        assert!(questions("anything_else").iter().any(|q| q.0 == "looks_correct"));
        // the 0.1 gap: officer spirits fell through to the generic rubric
        for f in crate::family::Family::ALL.iter().copied() {
            let id = crate::builtin::rubric_of(f);
            assert!(id.ends_with("_v3"), "{f}: {id}");
            assert_ne!(questions(&id), GENERIC_V3, "{f} resolves to the generic rubric");
            // every v3 rubric asks against the vanilla references
            assert!(questions(&id).iter().any(|q| q.1.contains("vanilla")), "{f}");
        }
        // v1 and v2 stay readable for older reviews
        assert!(questions("picture_v1").iter().any(|q| q.0 == "treatment_correct"));
        assert!(questions("icon_v3").iter().any(|q| q.0 == "vanilla_form"));
        assert!(questions("icon_v3").iter().any(|q| q.0 == "vanilla_medium"));
        assert!(questions(&crate::builtin::rubric_of(crate::family::Family::OfficerCorpsSpirit)).iter().any(|q| q.0 == "vanilla_form"));
    }

    #[test]
    fn approval_needs_every_answer_true() {
        let all: Value = questions("icon_v1").iter().map(|(k, _)| (k.to_string(), json!(true))).collect::<serde_json::Map<_, _>>().into();
        assert!(check_answers("icon_v1", &all).all_true());
        let mut some = all.clone();
        some["silhouette_clear"] = json!(false);
        some.as_object_mut().unwrap().remove("no_text_or_watermark");
        let c = check_answers("icon_v1", &some);
        assert_eq!(c.failed, vec!["silhouette_clear"]);
        assert_eq!(c.missing, vec!["no_text_or_watermark"]);
        assert!(!check_answers("icon_v1", &json!({})).all_true());
        let stringly = json!({"silhouette_clear": "yes"});
        assert!(check_answers("icon_v1", &stringly).failed.contains(&"silhouette_clear"), "'yes' is not true");
    }
}
