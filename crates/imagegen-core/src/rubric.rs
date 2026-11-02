//! Visual review rubrics.
//!
//! Deterministic checks cannot judge composition, style or readability at game size. Those questions go to a
//! reviewer (a person, or an agent that can look at the preview) and are recorded next to the deterministic
//! results. An approval needs every question answered `true`; anything else is `changes_requested`.

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

/// Questions for a rubric id. Rubrics are per group (`icon_v1`, `picture_v1`, `portrait_v1`, `flag_v1`,
/// `unit_v1`, `interface_v1`), so every family of a group, officer corps spirits included, gets its group's
/// questions. Anything else falls back to the generic questions.
pub fn questions(rubric: &str) -> &'static [Question] {
    if rubric == "icon_v1" {
        return ICON_V1;
    }
    match rubric.strip_suffix("_v2").or_else(|| rubric.strip_suffix("_v1")).unwrap_or(rubric) {
        "icon" => ICON,
        "picture" => PICTURE,
        "portrait" => PORTRAIT,
        "flag" => FLAG,
        "unit" => UNIT,
        "interface" => INTERFACE,
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
        assert!(questions("anything_else").iter().any(|q| q.0 == "looks_correct"));
        // the 0.1 gap: officer spirits fell through to the generic rubric
        for f in crate::family::Family::ALL.iter().copied() {
            let id = crate::builtin::rubric_of(f);
            assert_ne!(questions(&id), GENERIC, "{f} resolves to the generic rubric");
        }
        assert!(questions(&crate::builtin::rubric_of(crate::family::Family::OfficerCorpsSpirit)).iter().any(|q| q.0 == "silhouette_clear"));
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
