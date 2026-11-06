//! Per-request choice of automatic style references: theme matching, nation-specific art and the request seed.
//!
//! A family's candidate pool is the same for every request. Which images are sent depends on the request:
//!
//! - **Theme.** The brief's words are matched against each file's name and folder tokens, directly and through a
//!   small built-in synonym table ([`THEMES`]). A brief about a fleet ranks `goal_generic_navy_*` above
//!   `goal_generic_air_*`.
//! - **Nation.** A file that belongs to one country (a tag prefix such as `GER_`, a vanilla `focus_GER_` /
//!   `goal_JAP_` / `idea_SOV_` name, a tag-named folder such as `gfx/leaders/JAP/`, or a country name or adjective in
//!   the name such as `focus_generic_attack_romania`) is left out unless the brief names that country.
//! - **Seed.** A hash of the family, the asset and the brief seeds every draw, so the same request gets the same set
//!   and two different briefs rarely share one.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::family::Family;
use crate::fsio::read_limited;
use crate::hashing::sha256_hex;

/// Largest country tag or localisation file read.
const MAX_LIST_BYTES: usize = 4 * 1024 * 1024;

/// The built-in theme table: each theme lists words and word stems (a trailing `*` matches any word that starts with
/// it). A brief word and a file token that fall in the same theme count as a theme match.
pub const THEMES: &[(&str, &[&str])] = &[
    (
        "industry",
        &[
            "industr*",
            "factor*",
            "production",
            "produce*",
            "manufactur*",
            "construct*",
            "civilian",
            "civ",
            "mil",
            "steel",
            "foundr*",
            "arsenal*",
            "workshop*",
            "machine*",
            "assembly",
        ],
    ),
    (
        "naval",
        &[
            "naval",
            "navy",
            "navies",
            "ship*",
            "fleet*",
            "sea",
            "seas",
            "submarin*",
            "carrier*",
            "battleship*",
            "destroyer*",
            "cruiser*",
            "dockyard*",
            "harbor*",
            "harbour*",
            "port",
            "ports",
            "marine*",
            "admiral*",
            "convoy*",
            "torpedo*",
            "anchor*",
            "ocean*",
        ],
    ),
    (
        "air",
        &[
            "air",
            "aircraft",
            "airforce",
            "airplane*",
            "plane",
            "planes",
            "bomber*",
            "fighter*",
            "cas",
            "aviat*",
            "pilot*",
            "luftwaffe",
            "airfield*",
            "airport*",
            "paratroop*",
            "glider*",
            "squadron*",
        ],
    ),
    (
        "army",
        &[
            "army",
            "armies",
            "infantry",
            "tank",
            "tanks",
            "armor*",
            "armour*",
            "artiller*",
            "cavalry",
            "motori*",
            "mechani*",
            "soldier*",
            "troop*",
            "division*",
            "regiment*",
            "rifle*",
            "militar*",
            "officer*",
            "general",
            "generals",
            "panzer*",
            "conscript*",
            "mountaineer*",
            "guard*",
        ],
    ),
    (
        "politics",
        &[
            "politic*",
            "party",
            "parties",
            "government*",
            "election*",
            "democra*",
            "fascis*",
            "communis*",
            "socialis*",
            "monarch*",
            "king",
            "kings",
            "kingdom",
            "queen*",
            "crown*",
            "throne*",
            "parliament*",
            "constitution*",
            "reform*",
            "propaganda",
            "ideolog*",
            "coup",
            "revolution*",
            "president*",
            "cabinet*",
            "republic*",
            "nationalis*",
            "liberal*",
            "dictator*",
            "vote*",
            "senate",
        ],
    ),
    (
        "research",
        &[
            "research*",
            "scien*",
            "technolog*",
            "universit*",
            "laborator*",
            "educat*",
            "nuclear",
            "atom*",
            "rocket*",
            "radar*",
            "engineer*",
            "invent*",
            "academ*",
            "school*",
            "electronic*",
            "computer*",
        ],
    ),
    (
        "economy",
        &[
            "econom*",
            "trade*",
            "money",
            "financ*",
            "bank*",
            "gold",
            "market*",
            "resource*",
            "oil",
            "export*",
            "import*",
            "tax",
            "taxes",
            "budget*",
            "invest*",
            "currenc*",
            "coin*",
            "commerc*",
            "rubber",
            "mining",
            "mine",
            "mines",
            "wealth*",
        ],
    ),
    (
        "intelligence",
        &[
            "intellig*",
            "spy",
            "spies",
            "espionag*",
            "agent*",
            "agency",
            "cipher*",
            "codebreak*",
            "secret*",
            "covert*",
            "crypt*",
            "decrypt*",
            "infiltrat*",
            "sabotag*",
            "operative*",
        ],
    ),
    (
        "diplomacy",
        &[
            "diplomac*",
            "diplomat*",
            "alliance*",
            "ally",
            "allies",
            "treat*",
            "pact*",
            "faction*",
            "friend*",
            "entente",
            "cooperat*",
            "embass*",
            "handshake*",
            "befriend*",
            "neutral*",
        ],
    ),
    (
        "war",
        &[
            "attack*",
            "war",
            "wars",
            "invade*",
            "invasion*",
            "conquest*",
            "conquer*",
            "offensive*",
            "claim*",
            "demand*",
            "annex*",
            "assault*",
            "blitz*",
            "strike*",
        ],
    ),
    (
        "defence",
        &["defen*", "fort", "forts", "fortif*", "fortress*", "bunker*", "wall", "walls", "coastal", "garrison*", "protect*", "shield*", "border*"],
    ),
    (
        "infrastructure",
        &[
            "infrastruct*",
            "rail*",
            "road*",
            "bridge*",
            "canal*",
            "dam",
            "dams",
            "electric*",
            "highway*",
            "transport*",
            "logistic*",
            "supply",
            "supplies",
        ],
    ),
    ("agriculture", &["agricultur*", "farm*", "food*", "grain*", "bread*", "harvest*", "crop*", "wheat", "rice", "fish*", "cattle", "peasant*"]),
    (
        "population",
        &[
            "populat*",
            "manpower",
            "people*",
            "worker*",
            "labor*",
            "labour*",
            "citizen*",
            "youth*",
            "women",
            "health*",
            "medic*",
            "hospital*",
            "welfare",
            "union*",
        ],
    ),
    ("religion", &["religio*", "church*", "faith*", "god", "pope*", "papal", "mosque*", "temple*", "holy", "clergy", "catholic*", "islam*"]),
];

/// Words in a brief that say nothing about its subject.
const STOP: &[&str] = &[
    "the",
    "and",
    "for",
    "with",
    "from",
    "into",
    "onto",
    "that",
    "this",
    "these",
    "those",
    "its",
    "his",
    "her",
    "their",
    "our",
    "your",
    "are",
    "was",
    "were",
    "has",
    "have",
    "had",
    "but",
    "not",
    "all",
    "any",
    "one",
    "two",
    "three",
    "over",
    "under",
    "about",
    "above",
    "below",
    "between",
    "behind",
    "before",
    "after",
    "while",
    "who",
    "whose",
    "which",
    "what",
    "where",
    "when",
    "icon",
    "icons",
    "image",
    "picture",
    "art",
    "artwork",
    "style",
    "focus",
    "national",
    "hearts",
    "iron",
    "hoi4",
    "hoi",
    "emblem",
    "symbol",
    "show",
    "shows",
    "showing",
    "depict",
    "depicting",
    "featuring",
    "large",
    "small",
    "big",
    "new",
    "old",
    "some",
    "many",
    "more",
    "most",
    "very",
    "own",
    "out",
    "off",
    "via",
    "per",
    "goal",
    "idea",
    "generic",
    "dds",
    "png",
    "tga",
    "gfx",
    "interface",
    "goals",
    "ideas",
    "event",
    "events",
];

/// File-name tokens that mark art made for any country.
const GENERIC: &[&str] = &["generic", "gen"];

/// Country tags of HOI4 1.19.4.1 (`common/country_tags/00_countries.txt`), used whether or not the game is found.
const BUILTIN_TAGS: &str = concat!(
    "ABK ADU AFA AFG ALB ALG ALT ANG ANU AOI ARG ARM AST ASY ATJ AUS AZR BAH BAN BAR BAS BAY BEG BEL BHR BHU BIA BLC BLI BLR ",
    "BLZ BOL BOS BOT BOU BRA BRD BRI BRM BRN BSK BUK BUL BYA CAM CAN CAR CAT CAY CBV CHA CHI CHL CHM CHR CHU CIN CIP CKK CMR ",
    "COG COL COR COS CPS CRC CRI CRO CSA CUB CYP CZE DAG DAH DDR DEN DIP DJI DNZ DOM DON ECU EGY ELS ENG EQG ERI EST ETH EVE ",
    "EZO FER FIJ FIN FOR FRA FSA FSM GAB GAL GAM GAR GBA GDC GDL GEN GEO GER GHA GLC GNA GNB GOW GRE GRN GSM GUA GUM GXC GYA ",
    "HAI HAN HAR HAW HBC HES HOL HON HRZ HUN HYD IAS ICE IMO INC INS INU IRE IRQ ISR ITA ITZ IVO JAM JAN JAP JOR KAL KAR KAS ",
    "KAT KAZ KBK KEN KHA KHI KHL KHM KKP KLT KOL KOM KOR KOS KSH KUB KUM KUR KUW KYR LAO LAT LBA LBV LEB LIB LIT LUX MAC MAD ",
    "MAL MAN MAY MEK MEL MEN MEX MIS MLD MLI MLT MLW MNT MOL MON MOR MPU MRT MYS MZB NAH NAV NEN NEP NGA NGR NIC NIR NMB NOA ",
    "NOR NWF NXM NZL OCC OKN OMA ORO OVO PAK PAL PAN PAP PAR PER PHI PLU PNG POK POL POR PRC PRE PRU PSH PSR PUE QAT QEM QUE ",
    "RAA RAJ RAN RAP RAR RAS RCG RCO RGB RHD RHI RIF RIG RJP RKA RKB RKC RKG RKH RKI RKK RKL RKM RKN RKO RKT RKU RKV RNA RNG ",
    "ROA ROM RUS RUT RWA SAB SAF SAM SAR SAU SAX SCO SDL SEN SER SHL SHX SIA SIC SID SIE SIK SIL SIN SKK SLO SLV SMI SND SNG ",
    "SOK SOL SOM SOV SPM SPR SRL SSI SUD SUR SWE SWI SYR TAH TAJ TAN TAT TAY THU TIB TIG TML TMS TNE TOG TOS TRA TRI TTS TUN ",
    "TUR TZN UAE UBD UDM UGA UKR URG USA USB UZB VAN VEN VGE VIN VLA VOL WES WGR WIS WLA WLS WPG WUR XIC XSM YAK YAM YEM YUC ",
    "YUG YUN ZAM ZIM ",
);

/// Tags that are also ordinary words or abbreviations, so they count as tags in lower case only when the game's file
/// names show it (`decision_cat_x` is a decision category picture, `focus_can_start` a title bar).
const LOWER_CASE_WORDS: &[&str] = &[
    "bar", "ban", "bay", "beg", "can", "car", "cat", "col", "den", "dip", "don", "for", "gab", "gal", "gam", "gar", "gen", "han", "has", "hon",
    "ice", "inc", "ins", "jam", "mad", "mal", "may", "mel", "men", "mis", "mol", "mon", "nav", "nor", "pal", "pan", "pap", "par", "per", "pol",
    "pre", "ran", "rap", "ras", "sam", "sen", "ser", "sic", "sin", "sol", "sur", "tan", "tat", "tos", "tra", "tun", "van", "vin", "was",
];

/// Built-in country names and adjectives of the major powers, used when the game's localisation is not available.
const BUILTIN_NATIONS: &[(&str, &[&str])] = &[
    ("GER", &["germany", "german", "germans", "reich"]),
    ("ENG", &["britain", "british", "united kingdom", "england", "english"]),
    ("SOV", &["soviet union", "soviet", "soviets", "ussr", "russia", "russian"]),
    ("USA", &["united states", "america", "american", "americans"]),
    ("FRA", &["france", "french"]),
    ("ITA", &["italy", "italian", "italians"]),
    ("JAP", &["japan", "japanese"]),
    ("CHI", &["china", "chinese"]),
];

/// Splits a name into lowercase alphanumeric tokens.
pub fn tokens(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_ascii_alphanumeric()).filter(|t| !t.is_empty()).map(|t| t.to_ascii_lowercase()).collect()
}

fn raw_tokens(s: &str) -> Vec<&str> {
    s.split(|c: char| !c.is_ascii_alphanumeric()).filter(|t| !t.is_empty()).collect()
}

fn word_matches(word: &str, entry: &str) -> bool {
    match entry.strip_suffix('*') {
        Some(stem) => word.starts_with(stem),
        None => word == entry,
    }
}

/// Themes a word belongs to.
pub fn themes_of(word: &str) -> Vec<&'static str> {
    THEMES.iter().filter(|(_, words)| words.iter().any(|e| word_matches(word, e))).map(|(t, _)| *t).collect()
}

/// Country tags and names, from the installed game when it is there, else from a pattern and a built-in list.
#[derive(Debug, Clone, Default)]
pub struct Nations {
    /// Upper-case tags (`GER`). Empty when only the pattern is known.
    pub tags: BTreeSet<String>,
    /// Lower-case names and adjectives (`germany`, `german`, `united kingdom`) with their tags.
    pub names: BTreeMap<String, BTreeSet<String>>,
    /// Where the lists come from.
    pub source: String,
}

impl Nations {
    /// Read the tags from `common/country_tags/*.txt` and the names from `localisation/english/countries_l_english.yml`
    /// of the installed game. Without the game, tags are recognised by pattern (three upper-case letters) and names
    /// from a built-in list of the major powers.
    pub fn load(game: Option<&Path>) -> Nations {
        let mut n = Nations::default();
        n.tags.extend(BUILTIN_TAGS.split_whitespace().map(str::to_string));
        for (tag, names) in BUILTIN_NATIONS {
            for name in *names {
                n.names.entry((*name).to_string()).or_default().insert((*tag).to_string());
            }
        }
        let Some(game) = game else {
            n.source = format!("the built-in list of {} vanilla tags, a three-capital-letter pattern and built-in names of the major powers (the game was not found)", n.tags.len());
            return n;
        };
        let mut files: Vec<_> = std::fs::read_dir(game.join("common/country_tags")).into_iter().flatten().flatten().map(|e| e.path()).collect();
        files.sort();
        for f in files.iter().filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("txt"))) {
            let Ok(bytes) = read_limited(f, MAX_LIST_BYTES) else { continue };
            for line in String::from_utf8_lossy(&bytes).lines() {
                let line = line.split('#').next().unwrap_or("").trim();
                let Some((tag, rest)) = line.split_once('=') else { continue };
                let tag = tag.trim();
                if is_tag_shape(tag) && rest.trim().trim_start_matches('"').starts_with("countries/") {
                    n.tags.insert(tag.to_string());
                }
            }
        }
        if let Ok(bytes) = read_limited(&game.join("localisation/english/countries_l_english.yml"), MAX_LIST_BYTES) {
            for line in String::from_utf8_lossy(&bytes).lines() {
                let Some((key, value)) = line.trim().split_once(':') else { continue };
                let tag = key.strip_suffix("_DEF").or_else(|| key.strip_suffix("_ADJ")).unwrap_or(key);
                if !n.tags.contains(tag) {
                    continue;
                }
                let Some(name) = value.split('"').nth(1) else { continue };
                let name = tokens(name).join(" ");
                let name = name.strip_prefix("the ").unwrap_or(&name).to_string();
                if name.len() >= 4 {
                    n.names.entry(name).or_default().insert(tag.to_string());
                }
            }
        }
        n.source = format!(
            "{} tags (the game's common/country_tags and the built-in list) and {} country names and adjectives",
            n.tags.len(),
            n.names.len()
        );
        n
    }

    /// A known tag: the built-in vanilla list, plus the game's own list when the game is there.
    fn is_tag(&self, s: &str) -> bool {
        self.tags.contains(s)
    }

    /// True when a request may use a file that [`Nations::nation_of`] marked (a regional file needs its region word
    /// in the brief).
    pub fn allows(brief: &Brief, mark: &NationMark) -> bool {
        match &mark.region {
            Some(w) => brief.words.iter().any(|b| b == w),
            None => mark.tags.iter().any(|t| brief.nations.contains(t)),
        }
    }

    /// Countries a brief names: a tag written in capitals (`JAP`), or a country name or adjective (`Japan`, `Japanese`).
    pub fn named_in(&self, brief: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for t in raw_tokens(brief) {
            if self.is_tag(t) {
                out.insert(t.to_string());
            }
        }
        let seq = format!(" {} ", tokens(brief).join(" "));
        for (name, tags) in &self.names {
            if seq.contains(&format!(" {name} ")) {
                out.extend(tags.iter().cloned());
            }
        }
        out
    }

    /// Why a file belongs to one country, with that country's tags (empty for a region word such as `NORDIC`), or
    /// `None` for art made for any country. `rel_dirs` are the folders between the family's texture folder and the
    /// file.
    pub fn nation_of(&self, stem: &str, rel_dirs: &[&str]) -> Option<NationMark> {
        for d in rel_dirs {
            if self.is_tag(d) {
                return Some(NationMark { reason: format!("in the country folder {d}/"), tags: BTreeSet::from([d.to_string()]), region: None });
            }
        }
        let raw = raw_tokens(stem);
        // a leading tag (`GER_x`), or the tag after the family word (`focus_GER_x`, `goal_jap_x`, `idea_SOV_x`)
        let after_prefix = matches!(raw.first().map(|t| t.to_ascii_lowercase()).as_deref(), Some("focus" | "goal" | "idea" | "decision" | "trait"));
        let lead = if after_prefix { raw.get(1) } else { raw.first() };
        if let Some(t) = lead {
            let upper = t.to_ascii_uppercase();
            let more = raw.len() > 1 + usize::from(after_prefix);
            // capitals: any known tag, or the tag pattern for a mod's own tags; lower case (`focus_jap_x`, as the kit's
            // shelf copy names vanilla files): a known tag after the family word that is not an ordinary word
            let capital = *t == upper && is_tag_shape(t);
            let lower = after_prefix && t.len() == 3 && *t == t.to_ascii_lowercase() && self.is_tag(&upper) && !LOWER_CASE_WORDS.contains(t);
            if more && (capital || lower) {
                return Some(NationMark { reason: format!("name starts with the country tag {upper}"), tags: BTreeSet::from([upper]), region: None });
            }
            // a region or dynasty written like a tag (`focus_NORDIC_x`, `focus_CONGO_x`)
            if after_prefix && t.len() > 3 && t.chars().all(|c| c.is_ascii_uppercase()) {
                return Some(NationMark {
                    reason: format!("name starts with the regional name {t}"),
                    tags: BTreeSet::new(),
                    region: Some(t.to_ascii_lowercase()),
                });
            }
        }
        let seq = format!(" {} ", tokens(stem).join(" "));
        for (name, tags) in &self.names {
            if seq.contains(&format!(" {name} ")) {
                return Some(NationMark { reason: format!("name names the country '{name}'"), tags: tags.clone(), region: None });
            }
        }
        None
    }
}

/// Why a file belongs to one country or region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NationMark {
    /// For the plan.
    pub reason: String,
    /// The country's tags (empty for a region).
    pub tags: BTreeSet<String>,
    /// The region word (lower case), for a regional file.
    pub region: Option<String>,
}

fn is_tag_shape(s: &str) -> bool {
    s.len() == 3 && s.bytes().next().is_some_and(|b| b.is_ascii_uppercase()) && s.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// What a request asks for, as the selection reads it.
#[derive(Debug, Clone)]
pub struct Brief {
    /// The brief's subject words (lower case, stop words removed).
    pub words: Vec<String>,
    /// The themes the brief's words fall in.
    pub themes: BTreeSet<&'static str>,
    /// Countries the brief names.
    pub nations: BTreeSet<String>,
    /// Seed of the draws: a hash of the family, the asset and the brief's words.
    pub seed: u64,
}

impl Brief {
    /// Read a request.
    pub fn new(family: Family, asset: &str, brief: &str, nations: &Nations) -> Brief {
        let mut words: Vec<String> = Vec::new();
        for w in tokens(brief).into_iter().chain(tokens(asset)) {
            if w.len() >= 3 && !w.bytes().all(|b| b.is_ascii_digit()) && !STOP.contains(&w.as_str()) && !words.contains(&w) {
                words.push(w);
            }
        }
        let themes = words.iter().flat_map(|w| themes_of(w)).collect();
        let normal = tokens(brief).join(" ");
        let hash = sha256_hex(format!("{}\n{}\n{}", family.as_str(), asset, normal).as_bytes());
        let seed = u64::from_str_radix(&hash[..16], 16).unwrap_or(0);
        Brief { words, themes, nations: nations.named_in(brief), seed }
    }

    /// How well a file matches the brief, with the matched terms. A brief word equal to a file token (or sharing its
    /// first five letters) scores 3, each theme they share 2.
    pub fn score(&self, file_tokens: &[String]) -> (u32, Vec<String>) {
        let mut score = 0;
        let mut terms = Vec::new();
        for w in &self.words {
            let hit = file_tokens.iter().any(|t| t == w || (t.len() >= 5 && w.len() >= 5 && t[..5] == w[..5]));
            if hit {
                score += 3;
                terms.push(w.clone());
            }
        }
        let file_themes: BTreeSet<&str> = file_tokens.iter().flat_map(|t| themes_of(t)).collect();
        for t in self.themes.intersection(&file_themes) {
            score += 2;
            terms.push(format!("theme {t}"));
        }
        (score, terms)
    }
}

/// True when the file name marks it as made for any country (`goal_generic_x`, `generic_x`).
pub fn is_generic(file_tokens: &[String]) -> bool {
    file_tokens.iter().take(2).any(|t| GENERIC.contains(&t.as_str()))
}

/// A small deterministic generator (SplitMix64) for the seeded draws.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    /// Seeded generator.
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    /// Next value.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Shuffle in place (Fisher-Yates).
    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = (self.next_u64() % (i as u64 + 1)) as usize;
            v.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nations() -> Nations {
        let mut n = Nations::load(None);
        n.names.entry("siam".into()).or_default().insert("SIA".into());
        n.names.entry("romania".into()).or_default().insert("ROM".into());
        n
    }

    #[test]
    fn synonym_table_covers_the_documented_themes() {
        for (word, theme) in [
            ("factories", "industry"),
            ("fleet", "naval"),
            ("submarines", "naval"),
            ("bombers", "air"),
            ("artillery", "army"),
            ("elections", "politics"),
            ("rocketry", "research"),
            ("banking", "economy"),
            ("espionage", "intelligence"),
            ("treaty", "diplomacy"),
            ("invasion", "war"),
            ("fortifications", "defence"),
            ("railways", "infrastructure"),
            ("harvest", "agriculture"),
            ("manpower", "population"),
            ("church", "religion"),
        ] {
            assert!(themes_of(word).contains(&theme), "{word} -> {theme}");
        }
        assert!(themes_of("portrait").is_empty(), "`port` is a whole word, not a stem");
        assert!(themes_of("seal").is_empty());
    }

    #[test]
    fn nation_specific_names_are_recognised() {
        let n = nations();
        for stem in
            ["focus_GER_afrikakorps", "goal_JAP_x", "focus_jap_x", "GER_army", "idea_SIA_reform", "focus_generic_attack_romania", "focus_NORDIC_x"]
        {
            assert!(n.nation_of(stem, &[]).is_some(), "{stem}");
        }
        for stem in ["goal_generic_navy", "focus_generic_military_industry", "goal_anti_comintern_pact", "focus_continuous_air", "idea_army_reform"] {
            assert!(n.nation_of(stem, &[]).is_none(), "{stem}");
        }
        assert!(n.nation_of("portrait_1", &["JAP"]).is_some(), "a tag-named folder");
        assert!(n.nation_of("portrait_1", &["Africa"]).is_none(), "a region folder of generic portraits");
        // the pattern alone, without the game's tag list
        let bare = Nations::load(None);
        assert!(bare.nation_of("focus_XYZ_thing", &[]).is_some(), "the capital pattern covers a mod's own tags");
        assert!(bare.nation_of("goal_generic_CAS", &[]).is_none());
        // the kit's shelf copy names vanilla files in lower case
        for stem in ["focus_jap_imperial_glory", "focus_hol_dutch_navy", "focus_BRA_coffee", "focus_bra_coffee"] {
            assert!(bare.nation_of(stem, &[]).is_some(), "{stem}");
        }
        // an ordinary word that is also a tag stays a word
        for stem in ["decision_cat_industry", "focus_can_start_bg", "focus_car_industry", "goal_gen_staff", "focus_air_superiority"] {
            assert!(bare.nation_of(stem, &[]).is_none(), "{stem}");
        }
    }

    #[test]
    fn a_brief_names_countries_by_tag_name_or_adjective() {
        let n = nations();
        assert_eq!(n.named_in("A Japanese carrier fleet"), BTreeSet::from(["JAP".to_string()]));
        assert_eq!(n.named_in("the GER army"), BTreeSet::from(["GER".to_string()]));
        assert!(n.named_in("a steel factory at war").is_empty(), "lower-case words are not tags");
        assert!(n.named_in("rebuild the navy").is_empty());
    }

    #[test]
    fn scoring_prefers_theme_and_noun_matches() {
        let n = nations();
        let b = Brief::new(Family::Focus, "grand_fleet", "Build a grand battleship fleet", &n);
        assert!(b.themes.contains("naval"));
        let navy = b.score(&tokens("goal_generic_navy_battleship"));
        let air = b.score(&tokens("goal_generic_air_fighter"));
        assert!(navy.0 > air.0, "{navy:?} {air:?}");
        assert_eq!(air.0, 0);
        assert!(navy.1.iter().any(|t| t == "battleship"));
    }

    #[test]
    fn the_seed_depends_on_the_request_alone() {
        let n = nations();
        let a = Brief::new(Family::Focus, "a", "Steel works", &n);
        let b = Brief::new(Family::Focus, "a", "  steel   WORKS ", &n);
        let c = Brief::new(Family::Focus, "a", "Steel mills", &n);
        let d = Brief::new(Family::IdeaSpirit, "a", "Steel works", &n);
        assert_eq!(a.seed, b.seed);
        assert_ne!(a.seed, c.seed);
        assert_ne!(a.seed, d.seed);
        let mut v: Vec<u32> = (0..20).collect();
        Rng::new(7).shuffle(&mut v);
        let mut w: Vec<u32> = (0..20).collect();
        Rng::new(7).shuffle(&mut w);
        assert_eq!(v, w);
        assert_ne!(v, (0..20).collect::<Vec<_>>());
    }
}
