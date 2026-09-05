//! Identify the object in an image and fetch its catalogue information from
//! the CDS Sesame name resolver (backed by SIMBAD), to stamp a proper name
//! like "Andromeda Galaxy (M 31)" instead of the bare file name.
//!
//! Sources of identity, in order of trust:
//! 1. the `OBJECT` keyword from the FITS / XISF header (what the capture
//!    software was told the target was);
//! 2. a catalogue designation found in the file name (`M31`, `NGC_7000`,
//!    `Sh2-155`, ...).
//!
//! When both exist they are cross-checked against the resolved object's alias
//! list; on disagreement the file name wins (it is what the user named the
//! image) and a note is attached. If nothing resolves, the file name is used
//! verbatim.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use crate::post::Label;

const SESAME_URL: &str = "https://cds.unistra.fr/cgi-bin/nph-sesame/-oxI/S?";

/// Catalogues we recognise in file names and prefer when presenting aliases.
/// Order = display priority.
struct Catalog {
    /// Upper-case prefixes accepted in file names.
    keys: &'static [&'static str],
    /// How we print it: prefix followed by the number.
    pretty: &'static str,
    /// Prefixes SIMBAD uses in its alias list (case-insensitive, whitespace
    /// collapsed), followed by the number.
    simbad: &'static [&'static str],
    /// Highest valid number (sanity filter against exposure times etc.).
    max: u32,
    /// Single-letter catalogues: the number must follow the letter directly
    /// ("M31", not "M_31") so that filter letters like `_B_` are not mistaken
    /// for Barnard designations.
    adjacent_only: bool,
}

const CATALOGS: &[Catalog] = &[
    Catalog { keys: &["M", "MESSIER"], pretty: "M ", simbad: &["M "], max: 110, adjacent_only: true },
    Catalog { keys: &["NGC"], pretty: "NGC ", simbad: &["NGC "], max: 7840, adjacent_only: false },
    Catalog { keys: &["IC"], pretty: "IC ", simbad: &["IC "], max: 5386, adjacent_only: false },
    Catalog { keys: &["SH2", "SH"], pretty: "Sh2-", simbad: &["SH 2-", "SH2-"], max: 313, adjacent_only: false },
    Catalog { keys: &["B", "BARNARD"], pretty: "Barnard ", simbad: &["Barnard "], max: 370, adjacent_only: true },
    Catalog { keys: &["LBN"], pretty: "LBN ", simbad: &["LBN "], max: 1125, adjacent_only: false },
    Catalog { keys: &["LDN"], pretty: "LDN ", simbad: &["LDN "], max: 1802, adjacent_only: false },
    Catalog { keys: &["VDB"], pretty: "vdB ", simbad: &["VdB ", "vdB "], max: 158, adjacent_only: false },
    Catalog { keys: &["CR", "COLLINDER"], pretty: "Cr ", simbad: &["Cr ", "Cl Collinder "], max: 471, adjacent_only: false },
    Catalog { keys: &["MEL", "MELOTTE"], pretty: "Mel ", simbad: &["Cl Melotte ", "Mel "], max: 245, adjacent_only: false },
    Catalog { keys: &["CED", "CEDERBLAD"], pretty: "Ced ", simbad: &["Ced "], max: 215, adjacent_only: false },
    Catalog { keys: &["ARP"], pretty: "Arp ", simbad: &["APG ", "Arp "], max: 338, adjacent_only: false },
    Catalog { keys: &["UGC"], pretty: "UGC ", simbad: &["UGC "], max: 12921, adjacent_only: false },
    Catalog { keys: &["PGC"], pretty: "PGC ", simbad: &["LEDA ", "PGC "], max: 9_999_999, adjacent_only: false },
    Catalog { keys: &["HD"], pretty: "HD ", simbad: &["HD "], max: 359083, adjacent_only: false },
    Catalog { keys: &["HIP"], pretty: "HIP ", simbad: &["HIP "], max: 120404, adjacent_only: false },
];

/// What Sesame told us about one object.
#[derive(Debug, Clone)]
pub struct ObjectInfo {
    /// SIMBAD main identifier, whitespace collapsed (e.g. "M 31").
    pub main_id: String,
    /// Best common name, if any (e.g. "Andromeda Galaxy").
    pub common_name: Option<String>,
    /// Recognised catalogue designations, pretty-printed, in display priority.
    pub designations: Vec<String>,
    /// Every alias, normalised for comparison.
    aliases_norm: HashSet<String>,
    /// SIMBAD object type code (e.g. "G", "HII", "OpC").
    pub otype: String,
    pub ra_deg: Option<f64>,
    pub dec_deg: Option<f64>,
}

impl ObjectInfo {
    /// Does `designation` (any spacing/case) refer to this object?
    pub fn matches(&self, designation: &str) -> bool {
        self.aliases_norm.contains(&normalize(designation))
    }

    pub fn type_description(&self) -> String {
        otype_description(&self.otype).to_string()
    }

    /// "RA 00h 42m 44s  Dec +41° 16′ 08″", or empty if unknown.
    pub fn coordinates(&self) -> String {
        match (self.ra_deg, self.dec_deg) {
            (Some(ra), Some(dec)) => format!("RA {}  Dec {}", fmt_ra(ra), fmt_dec(dec)),
            _ => String::new(),
        }
    }
}

/// Resolves names online, caching every answer for the duration of a run so
/// a folder of 300 subs of the same target costs one request.
pub struct Resolver {
    enabled: bool,
    agent: Option<ureq::Agent>,
    cache: HashMap<String, Option<ObjectInfo>>,
    /// Set (and lookups disabled) after the first network failure.
    pub failure: Option<String>,
}

impl Resolver {
    pub fn new(enabled: bool) -> Self {
        let agent = enabled.then(|| {
            ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(12)))
                .user_agent(format!(
                    "xisf2png/{} (+https://github.com/peterbuitho/xisf2png)",
                    env!("CARGO_PKG_VERSION")
                ))
                .build()
                .new_agent()
        });
        Resolver {
            enabled,
            agent,
            cache: HashMap::new(),
            failure: None,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled && self.failure.is_none()
    }

    /// Look a name up. `None` when disabled, not found, or after a network
    /// failure (which is recorded in `failure`).
    pub fn resolve(&mut self, query: &str) -> Option<&ObjectInfo> {
        if !self.enabled() {
            return None;
        }
        let key = normalize(query);
        if key.is_empty() {
            return None;
        }
        if !self.cache.contains_key(&key) {
            let agent = self.agent.as_ref()?;
            match fetch(agent, query) {
                Ok(info) => {
                    self.cache.insert(key.clone(), info);
                }
                Err(e) => {
                    self.failure = Some(e);
                    return None;
                }
            }
        }
        self.cache.get(&key).and_then(|o| o.as_ref())
    }
}

fn fetch(agent: &ureq::Agent, query: &str) -> Result<Option<ObjectInfo>, String> {
    let url = format!("{SESAME_URL}{}", percent_encode(query.trim()));
    let mut response = agent
        .get(&url)
        .call()
        .map_err(|e| format!("Sesame request failed: {e}"))?;
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("Sesame response unreadable: {e}"))?;
    parse_sesame(&body)
}

/// Parse Sesame's XML (`-ox` output). `Ok(None)` = nothing found.
pub fn parse_sesame(xml: &str) -> Result<Option<ObjectInfo>, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("Sesame XML invalid: {e}"))?;

    let Some(resolver) = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "Resolver" && child_text(n, "oname").is_some())
    else {
        return Ok(None);
    };

    // SIMBAD prefixes common names with "NAME " in identifiers; drop it for
    // display ("NAME Horsehead Nebula" -> "Horsehead Nebula").
    let main_id = collapse_ws(&child_text(&resolver, "oname").unwrap_or_default());
    let main_id = main_id
        .strip_prefix("NAME ")
        .map(str::to_string)
        .unwrap_or(main_id);
    let otype = child_text(&resolver, "otype").unwrap_or_default().trim().to_string();
    let ra_deg = child_text(&resolver, "jradeg").and_then(|s| s.trim().parse().ok());
    let dec_deg = child_text(&resolver, "jdedeg").and_then(|s| s.trim().parse().ok());

    let aliases: Vec<String> = resolver
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "alias")
        .filter_map(|n| n.text())
        .map(collapse_ws)
        .chain(std::iter::once(main_id.clone()))
        .collect();

    let common_name = pick_common_name(&aliases);

    let mut designations = Vec::new();
    for cat in CATALOGS {
        for alias in &aliases {
            if let Some(d) = catalog_designation(cat, alias) {
                if !designations.contains(&d) {
                    designations.push(d);
                }
            }
        }
    }

    let aliases_norm = aliases.iter().map(|a| normalize(a)).collect();

    Ok(Some(ObjectInfo {
        main_id,
        common_name,
        designations,
        aliases_norm,
        otype,
        ra_deg,
        dec_deg,
    }))
}

/// If `alias` is "<simbad prefix><number>" for this catalogue, return the
/// pretty form.
fn catalog_designation(cat: &Catalog, alias: &str) -> Option<String> {
    for prefix in cat.simbad {
        if alias.len() > prefix.len() && alias[..prefix.len()].eq_ignore_ascii_case(prefix) {
            let rest = alias[prefix.len()..].trim();
            if let Ok(n) = rest.parse::<u32>() {
                if n >= 1 && n <= cat.max {
                    return Some(format!("{}{}", cat.pretty, n));
                }
            }
        }
    }
    None
}

/// Choose the most presentable "NAME ..." alias. SIMBAD lists several
/// variants ("Andromeda", "Andromeda Galaxy", "And Nebula", "NORTH AMER NEB");
/// prefer a mixed-case one ending in a type word, skip abbreviations and
/// shouting.
fn pick_common_name(aliases: &[String]) -> Option<String> {
    const TYPE_WORDS: &[&str] = &[
        "nebula", "galaxy", "cluster", "cloud", "remnant", "loop", "complex", "star", "group",
        "association", "region", "filament", "chain", "triplet", "quintet", "sextet", "arc",
        "wall", "bubble", "shell", "ring", "pair", "stream", "dwarf",
    ];
    const CONSTELLATION_ABBR: &[&str] = &[
        "And", "Ant", "Aps", "Aqr", "Aql", "Ara", "Ari", "Aur", "Boo", "Cae", "Cam", "Cnc", "CVn",
        "CMa", "CMi", "Cap", "Car", "Cas", "Cen", "Cep", "Cet", "Cha", "Cir", "Col", "Com", "CrA",
        "CrB", "Crv", "Crt", "Cru", "Cyg", "Del", "Dor", "Dra", "Equ", "Eri", "For", "Gem", "Gru",
        "Her", "Hor", "Hya", "Hyi", "Ind", "Lac", "Leo", "LMi", "Lep", "Lib", "Lup", "Lyn", "Lyr",
        "Men", "Mic", "Mon", "Mus", "Nor", "Oct", "Oph", "Ori", "Pav", "Peg", "Per", "Phe", "Pic",
        "Psc", "PsA", "Pup", "Pyx", "Ret", "Sge", "Sgr", "Sco", "Scl", "Sct", "Ser", "Sex", "Tau",
        "Tel", "Tri", "TrA", "Tuc", "UMa", "UMi", "Vel", "Vir", "Vol", "Vul",
    ];
    const OTHER_ABBR: &[&str] = &["Neb", "Gal", "Cl", "Nebul", "Amer"];

    let names: Vec<&str> = aliases
        .iter()
        .filter_map(|a| a.strip_prefix("NAME "))
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .collect();
    if names.is_empty() {
        return None;
    }

    let is_shouting = |n: &str| {
        let letters: Vec<char> = n.chars().filter(|c| c.is_alphabetic()).collect();
        letters.len() >= 4 && letters.iter().all(|c| c.is_uppercase())
    };
    let has_abbreviation = |n: &str| {
        n.split_whitespace().any(|w| {
            let w = w.trim_matches(|c: char| !c.is_alphanumeric());
            CONSTELLATION_ABBR.contains(&w) || OTHER_ABBR.contains(&w)
        })
    };
    let has_type_word = |n: &str| {
        n.split_whitespace()
            .last()
            .map(|w| TYPE_WORDS.contains(&w.to_ascii_lowercase().as_str()))
            .unwrap_or(false)
    };

    let clean: Vec<&str> = names
        .iter()
        .copied()
        .filter(|n| !is_shouting(n) && !has_abbreviation(n))
        .collect();

    clean
        .iter()
        .copied()
        .find(|n| has_type_word(n))
        .or_else(|| clean.first().copied())
        .or_else(|| names.first().copied())
        .map(|s| s.to_string())
}

/// Find the first catalogue designation in a file name stem, e.g.
/// "2026-09-05_NGC7000_Ha_300s" -> "NGC 7000".
pub fn designation_in_name(name: &str) -> Option<String> {
    let upper: Vec<char> = name.to_ascii_uppercase().chars().collect();
    let n = upper.len();
    let is_sep = |c: char| c == ' ' || c == '_' || c == '-' || c == '.';

    let mut i = 0;
    while i < n {
        // Start of a letter run, not glued to a preceding letter/digit.
        if !upper[i].is_ascii_alphabetic() || (i > 0 && upper[i - 1].is_ascii_alphanumeric()) {
            i += 1;
            continue;
        }
        let start = i;
        while i < n && upper[i].is_ascii_alphabetic() {
            i += 1;
        }
        let word: String = upper[start..i].iter().collect();

        for cat in CATALOGS {
            if !cat.keys.contains(&word.as_str()) {
                continue;
            }
            let mut j = i;
            // "SH" must be followed by "2" (optionally separated) to be Sharpless.
            if word == "SH" {
                if j < n && is_sep(upper[j]) {
                    j += 1;
                }
                if j < n && upper[j] == '2' {
                    j += 1;
                } else {
                    continue;
                }
            }
            let sep_here = j < n && is_sep(upper[j]);
            if sep_here {
                if cat.adjacent_only {
                    continue;
                }
                j += 1;
            }
            let dstart = j;
            while j < n && upper[j].is_ascii_digit() && j - dstart < 8 {
                j += 1;
            }
            if j == dstart || (j < n && upper[j].is_ascii_alphabetic()) {
                continue;
            }
            let digits: String = upper[dstart..j].iter().collect();
            if let Ok(num) = digits.parse::<u32>() {
                if num >= 1 && num <= cat.max {
                    return Some(format!("{}{}", cat.pretty, num));
                }
            }
        }
    }
    None
}

/// The finished identification for one file.
pub struct Identification {
    pub label: Label,
    /// Something worth telling the user (cross-check disagreement etc.).
    pub note: Option<String>,
}

/// Work out what to stamp on an image, given the header's OBJECT keyword (if
/// any) and the file name stem.
pub fn identify(resolver: &mut Resolver, header_object: Option<&str>, stem: &str) -> Identification {
    let fallback = || Identification {
        label: Label::plain(stem),
        note: None,
    };
    if !resolver.enabled() {
        return fallback();
    }

    let file_desig = designation_in_name(stem);
    let header = header_object
        .map(str::trim)
        .map(|s| s.trim_matches('\'').trim())
        .filter(|s| !s.is_empty());

    // 1. Header OBJECT, cross-checked against the file name.
    if let Some(h) = header {
        if let Some(info) = resolver.resolve(h).cloned() {
            match &file_desig {
                Some(fd) if !info.matches(fd) => {
                    // Disagreement. Trust the file name if it resolves.
                    if let Some(info2) = resolver.resolve(fd).cloned() {
                        // "'NGC 7000' is NGC 7000" reads silly; only name the
                        // resolved object when it adds information.
                        let resolved_as = if normalize(h) == normalize(&info.main_id) {
                            String::new()
                        } else {
                            format!(" ({})", info.main_id)
                        };
                        return Identification {
                            label: compose(&info2, Some(fd)),
                            note: Some(format!(
                                "header OBJECT is '{h}'{resolved_as} but file name says {fd}; used file name"
                            )),
                        };
                    }
                    return Identification {
                        label: compose(&info, designation_in_name(h).as_deref()),
                        note: Some(format!(
                            "header OBJECT '{h}' ({}) does not match file name designation {fd}",
                            info.main_id
                        )),
                    };
                }
                Some(fd) => {
                    return Identification {
                        label: compose(&info, Some(fd)),
                        note: None,
                    };
                }
                None => {
                    return Identification {
                        label: compose(&info, designation_in_name(h).as_deref()),
                        note: None,
                    };
                }
            }
        }
    }

    // 2. File name designation alone.
    if let Some(fd) = &file_desig {
        if let Some(info) = resolver.resolve(fd).cloned() {
            return Identification {
                label: compose(&info, Some(fd)),
                note: None,
            };
        }
    }

    // 3. Nothing resolved (or the network went away).
    let mut id = fallback();
    if resolver.enabled() && (header.is_some() || file_desig.is_some()) {
        id.note = Some("object not found in SIMBAD; used file name".into());
    }
    id
}

/// Build the two-line label: "Common Name (Designation)" over
/// "other ids · type · coordinates".
fn compose(info: &ObjectInfo, preferred: Option<&str>) -> Label {
    let designation = preferred
        .map(str::to_string)
        .or_else(|| info.designations.first().cloned())
        .unwrap_or_else(|| info.main_id.clone());

    let title = match &info.common_name {
        Some(name) if normalize(name) != normalize(&designation) => {
            format!("{name} ({designation})")
        }
        _ => designation.clone(),
    };

    let key = normalize(&designation);
    let mut parts: Vec<String> = info
        .designations
        .iter()
        .filter(|d| normalize(d) != key)
        .take(3)
        .cloned()
        .collect();
    let ty = info.type_description();
    if !ty.is_empty() {
        parts.push(ty);
    }
    let coords = info.coordinates();
    if !coords.is_empty() {
        parts.push(coords);
    }

    Label {
        title,
        subtitle: (!parts.is_empty()).then(|| parts.join("  ·  ")),
    }
}

/// Canonical form for comparing identifiers: upper-case, no whitespace,
/// long catalogue names shortened to the abbreviations SIMBAD also uses.
pub fn normalize(s: &str) -> String {
    let mut u: String = s
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase();
    for (long, short) in [
        ("MESSIER", "M"),
        ("CLMELOTTE", "MEL"),
        ("MELOTTE", "MEL"),
        ("CLCOLLINDER", "CR"),
        ("COLLINDER", "CR"),
        ("CEDERBLAD", "CED"),
        ("LEDA", "PGC"),
        ("APG", "ARP"),
        ("SH2-", "SH2-"),
    ] {
        if u.starts_with(long) {
            u = format!("{short}{}", &u[long.len()..]);
        }
    }
    u
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn child_text<'a>(node: &roxmltree::Node<'a, '_>, name: &str) -> Option<String> {
    node.children()
        .find(|c| c.is_element() && c.tag_name().name() == name)
        .and_then(|c| c.text())
        .map(|t| t.to_string())
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn fmt_ra(deg: f64) -> String {
    let hours = deg.rem_euclid(360.0) / 15.0;
    let total = (hours * 3600.0).round() as i64;
    let (h, m, s) = (total / 3600 % 24, total / 60 % 60, total % 60);
    format!("{h:02}h {m:02}m {s:02}s")
}

fn fmt_dec(deg: f64) -> String {
    let sign = if deg < 0.0 { '−' } else { '+' };
    let total = (deg.abs() * 3600.0).round() as i64;
    let (d, m, s) = (total / 3600, total / 60 % 60, total % 60);
    format!("{sign}{d:02}° {m:02}′ {s:02}″")
}

/// Human-readable SIMBAD object types (the common ones for astrophotography;
/// unknown codes are shown as-is).
fn otype_description(code: &str) -> &str {
    match code.trim_end_matches('?') {
        // Galaxies
        "G" => "Galaxy",
        "AGN" => "Galaxy (active nucleus)",
        "GiG" => "Galaxy in a group",
        "GiP" => "Galaxy in a pair",
        "GiC" => "Galaxy in a cluster",
        "BiC" => "Brightest cluster galaxy",
        "IG" => "Interacting galaxies",
        "PaG" => "Pair of galaxies",
        "GrG" => "Group of galaxies",
        "CGG" => "Compact group of galaxies",
        "ClG" => "Cluster of galaxies",
        "SCG" => "Supercluster of galaxies",
        "SBG" => "Starburst galaxy",
        "EmG" => "Emission-line galaxy",
        "H2G" => "HII galaxy",
        "LSB" => "Low surface brightness galaxy",
        "rG" => "Radio galaxy",
        "SyG" => "Seyfert galaxy",
        "Sy1" => "Seyfert 1 galaxy",
        "Sy2" => "Seyfert 2 galaxy",
        "LIN" => "LINER galaxy",
        "QSO" => "Quasar",
        "BLL" | "Bla" => "Blazar",
        "PoG" => "Part of a galaxy",
        // Nebulae and interstellar medium
        "HII" => "HII region (emission nebula)",
        "PN" => "Planetary nebula",
        "SNR" => "Supernova remnant",
        "RNe" => "Reflection nebula",
        "DNe" => "Dark nebula",
        "GNe" | "Neb" => "Nebula",
        "EmO" => "Emission object",
        "MoC" => "Molecular cloud",
        "Cld" => "Cloud",
        "ISM" => "Interstellar medium",
        "bub" => "Bubble",
        "HH" => "Herbig-Haro object",
        "SFR" => "Star-forming region",
        "PoC" => "Part of a cloud",
        "glb" => "Globule",
        "cor" => "Dense core",
        "out" => "Outflow",
        "sh" => "Interstellar shell",
        "reg" => "Region",
        // Clusters and associations
        "OpC" => "Open cluster",
        "GlC" => "Globular cluster",
        "Cl*" => "Star cluster",
        "As*" => "Stellar association",
        "MGr" => "Moving group",
        "St*" => "Stellar stream",
        // Stars
        "*" => "Star",
        "**" => "Double or multiple star",
        "V*" => "Variable star",
        "Pe*" => "Peculiar star",
        "Em*" => "Emission-line star",
        "Be*" => "Be star",
        "WR*" => "Wolf-Rayet star",
        "Ce*" => "Cepheid variable",
        "Mi*" => "Mira variable",
        "LP*" => "Long-period variable",
        "RR*" => "RR Lyrae variable",
        "EB*" => "Eclipsing binary",
        "SB*" => "Spectroscopic binary",
        "Or*" => "Orion variable",
        "TT*" => "T Tauri star",
        "Y*O" => "Young stellar object",
        "sg*" => "Supergiant",
        "s*b" => "Blue supergiant",
        "s*r" => "Red supergiant",
        "s*y" => "Yellow supergiant",
        "RG*" => "Red giant",
        "AB*" => "AGB star",
        "pA*" => "Post-AGB star",
        "C*" => "Carbon star",
        "WD*" => "White dwarf",
        "N*" => "Neutron star",
        "Psr" => "Pulsar",
        "BH" => "Black hole",
        "XB*" => "X-ray binary",
        "SN*" => "Supernova",
        "No*" => "Nova",
        "Sy*" => "Symbiotic star",
        "PM*" => "High proper-motion star",
        "HS*" => "Hot subdwarf",
        "BD*" => "Brown dwarf",
        "LM*" => "Low-mass star",
        "Pl" => "Exoplanet",
        // Misc
        "gLe" => "Gravitational lens",
        "X" => "X-ray source",
        "Rad" => "Radio source",
        "IR" => "Infrared source",
        "UV" => "UV source",
        "gam" => "Gamma-ray source",
        "err" | "?" | "" => "",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn designations_from_names() {
        assert_eq!(designation_in_name("M31_Andromeda_2026-09-05"), Some("M 31".into()));
        assert_eq!(designation_in_name("2026-09-05_NGC7000_Ha_300s_0001"), Some("NGC 7000".into()));
        assert_eq!(designation_in_name("ngc_7000 stack"), Some("NGC 7000".into()));
        assert_eq!(designation_in_name("Sh2-155_RGB"), Some("Sh2-155".into()));
        assert_eq!(designation_in_name("SH2_101"), Some("Sh2-155".replace("155", "101")));
        assert_eq!(designation_in_name("IC1396_Elephant"), Some("IC 1396".into()));
        assert_eq!(designation_in_name("B33_horsehead"), Some("Barnard 33".into()));
        assert_eq!(designation_in_name("Messier 42"), None); // Messier requires adjacency? no: multi-letter allows sep
        assert_eq!(designation_in_name("Messier42"), Some("M 42".into()));
        // Filter letters and exposure times must not be mistaken for catalogues.
        assert_eq!(designation_in_name("Light_B_120s_0001"), None);
        assert_eq!(designation_in_name("M_31_L"), None);
        assert_eq!(designation_in_name("NGC7000A"), None);
        assert_eq!(designation_in_name("M999"), None);
        assert_eq!(designation_in_name("flat_2026"), None);
    }

    #[test]
    fn normalisation() {
        assert_eq!(normalize("M  31"), "M31");
        assert_eq!(normalize("SH 2-155"), "SH2-155");
        assert_eq!(normalize("Cl Melotte 22"), "MEL22");
        assert_eq!(normalize("Mel 22"), "MEL22");
        assert_eq!(normalize("Messier 31"), "M31");
    }

    #[test]
    fn sesame_parse() {
        let xml = r#"<?xml version="1.0"?><Sesame><Target option="S"><name>M31</name>
          <Resolver name="Sc=Simbad"><otype>AGN</otype><jradeg>10.68470833</jradeg><jdedeg>41.26875</jdedeg>
          <oname>M  31</oname><alias>M 31</alias><alias>NAME Andromeda</alias><alias>NAME Andromeda Galaxy</alias>
          <alias>NAME And Nebula</alias><alias>NGC 224</alias><alias>UGC 454</alias><alias>LEDA 2557</alias>
          </Resolver></Target></Sesame>"#;
        let info = parse_sesame(xml).unwrap().unwrap();
        assert_eq!(info.main_id, "M 31");
        assert_eq!(info.common_name.as_deref(), Some("Andromeda Galaxy"));
        assert_eq!(info.designations, vec!["M 31", "NGC 224", "UGC 454", "PGC 2557"]);
        assert!(info.matches("m31"));
        assert!(info.matches("NGC224"));
        assert!(!info.matches("NGC 7000"));
        assert_eq!(fmt_ra(10.68470833), "00h 42m 44s");
        assert_eq!(fmt_dec(41.26875), "+41° 16′ 08″");
        let label = compose(&info, Some("M 31"));
        assert_eq!(label.title, "Andromeda Galaxy (M 31)");
        assert_eq!(
            label.subtitle.as_deref(),
            Some("NGC 224  ·  UGC 454  ·  PGC 2557  ·  Galaxy (active nucleus)  ·  RA 00h 42m 44s  Dec +41° 16′ 08″")
        );

        let none = parse_sesame(r#"<Sesame><Target><name>ZZZ</name><INFO> *** Nothing found *** </INFO></Target></Sesame>"#).unwrap();
        assert!(none.is_none());
    }
}
