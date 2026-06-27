// Copyright 2026 The Eigenius Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Verb-form **generation** (D63 §8.9 Slice 6-aux, the morphology keystone): a base
//! verb lemma → its **gerund / present participle** (`ger`, "affecting") and **past
//! participle** (`pss`, "affected" / "gone"). The inverse direction to [`crate::morphy`]
//! (which lemmatizes inflected → base); the importer needs generation to emit the
//! participle [`crate::convert`] entries an auxiliary selects ("is affecting", "has
//! affected", "is affected").
//!
//! Two forms, two regimes:
//! - **Gerund** is fully regular in English (there are no irregular present
//!   participles), so [`gerund`] is pure orthography — silent-`e` drop, `ie → y`,
//!   `ee/oe/ye` retention, and monosyllabic consonant doubling.
//! - **Past participle** is regular (`-ed`) for the vast majority, with ~270 irregular
//!   bases ([`IRREGULAR_PP`]). The override table is **grounded**: every entry is an
//!   *invariant* (`pp == base`: cut/put/set) or an inflection **attested in the in-repo
//!   WordNet `verb.exc`** — candidate forms were sourced from Wikipedia's
//!   *List of English irregular verbs* and then **witnessed** against `verb.exc`
//!   (unattested extractions — kempt, durst, holpen — dropped fail-closed). The common
//!   `-t`/`-ed` twins (burnt/burned) are recovered productively at runtime from the
//!   attested `-t` form, so no non-word regular (weared, creeped) is ever admitted.
//!
//! Generation is **fail-closed**: an irregular not in the table falls back to the
//! regular `-ed`, so a wrong surface simply isn't found at lookup (no parse) rather than
//! producing a false one. Stress-conditioned doubling on polysyllables (begin →
//! *beginning*) is not detected — those miss the doubled form (graceful, no false parse).

/// Irregular base → past-participle form(s), **sorted by base** (binary search).
/// Grounded: each form is an invariant (`pp == base`) or attested in the in-repo
/// WordNet `verb.exc`; see the module docs and the `irregular_pp_*` tests. Source of
/// candidates: <https://en.wikipedia.org/wiki/List_of_English_irregular_verbs>,
/// filtered by the `verb.exc` witness.
pub const IRREGULAR_PP: &[(&str, &[&str])] = &[
    ("arise", &["arisen"]),
    ("awake", &["awoken"]),
    ("be", &["been"]),
    ("bear", &["borne", "born"]),
    ("beat", &["beaten", "beat"]),
    ("become", &["become"]),
    ("befall", &["befallen"]),
    ("beget", &["begot", "begotten"]),
    ("begin", &["begun"]),
    ("behold", &["beheld"]),
    ("bend", &["bent"]),
    ("beseech", &["besought"]),
    ("beset", &["beset"]),
    ("bespeak", &["bespoken"]),
    ("bet", &["bet", "betted"]),
    ("bid", &["bid"]),
    ("bind", &["bound"]),
    ("bite", &["bitten"]),
    ("bleed", &["bled"]),
    ("bless", &["blest"]),
    ("blow", &["blown"]),
    ("break", &["broken"]),
    ("breed", &["bred"]),
    ("bring", &["brought"]),
    ("broadcast", &["broadcast"]),
    ("build", &["built"]),
    ("burn", &["burnt"]),
    ("burst", &["burst"]),
    ("bust", &["bust"]),
    ("buy", &["bought"]),
    ("cast", &["cast"]),
    ("catch", &["caught"]),
    ("chide", &["chidden", "chid"]),
    ("choose", &["chosen"]),
    ("clad", &["clad"]),
    ("cleave", &["cleft", "cloven"]),
    ("cling", &["clung"]),
    ("clothe", &["clad"]),
    ("come", &["come"]),
    ("cost", &["cost"]),
    ("creep", &["crept"]),
    ("cut", &["cut"]),
    ("deal", &["dealt"]),
    ("dig", &["dug"]),
    ("dive", &["dove"]),
    ("do", &["done"]),
    ("draw", &["drawn"]),
    ("dream", &["dreamt"]),
    ("drink", &["drunk"]),
    ("drive", &["driven"]),
    ("dwell", &["dwelt"]),
    ("eat", &["eaten"]),
    ("fall", &["fallen"]),
    ("feed", &["fed"]),
    ("feel", &["felt"]),
    ("fight", &["fought"]),
    ("find", &["found"]),
    ("fit", &["fitted", "fit"]),
    ("flee", &["fled"]),
    ("fling", &["flung"]),
    ("fly", &["flown"]),
    ("forbid", &["forbidden"]),
    ("forecast", &["forecast"]),
    ("forego", &["foregone"]),
    ("foresee", &["foreseen"]),
    ("foretell", &["foretold"]),
    ("forget", &["forgotten"]),
    ("forgive", &["forgiven"]),
    ("forgo", &["forgone"]),
    ("forsake", &["forsaken"]),
    ("forswear", &["forsworn"]),
    ("freeze", &["frozen"]),
    ("get", &["got", "gotten"]),
    ("gild", &["gilt"]),
    ("gird", &["girt"]),
    ("give", &["given"]),
    ("go", &["gone"]),
    ("grind", &["ground"]),
    ("grow", &["grown"]),
    ("hang", &["hung"]),
    ("have", &["had"]),
    ("hear", &["heard"]),
    ("hew", &["hewn"]),
    ("hide", &["hidden"]),
    ("hit", &["hit"]),
    ("hoist", &["hoist"]),
    ("hold", &["held"]),
    ("hurt", &["hurt"]),
    ("inlay", &["inlaid"]),
    ("input", &["input"]),
    ("interweave", &["interwoven"]),
    ("keep", &["kept"]),
    ("kneel", &["knelt"]),
    ("knit", &["knit", "knitted"]),
    ("know", &["known"]),
    ("lade", &["laden"]),
    ("lay", &["laid"]),
    ("lead", &["led"]),
    ("lean", &["leant"]),
    ("leap", &["leapt"]),
    ("learn", &["learnt"]),
    ("leave", &["left"]),
    ("lend", &["lent"]),
    ("let", &["let"]),
    ("lie", &["lain"]),
    ("light", &["lit"]),
    ("lose", &["lost"]),
    ("make", &["made"]),
    ("mean", &["meant"]),
    ("meet", &["met"]),
    ("melt", &["molten"]),
    ("misgive", &["misgiven"]),
    ("mislay", &["mislaid"]),
    ("mislead", &["misled"]),
    ("misread", &["misread"]),
    ("misspell", &["misspelt"]),
    ("mistake", &["mistaken"]),
    ("misunderstand", &["misunderstood"]),
    ("mow", &["mown"]),
    ("offset", &["offset"]),
    ("outdo", &["outdone"]),
    ("outgrow", &["outgrown"]),
    ("output", &["output"]),
    ("outrun", &["outrun"]),
    ("overcome", &["overcome"]),
    ("overdo", &["overdone"]),
    ("overdraw", &["overdrawn"]),
    ("overdrive", &["overdriven"]),
    ("overfly", &["overflown"]),
    ("overgrow", &["overgrown"]),
    ("overhang", &["overhung"]),
    ("overhear", &["overheard"]),
    ("overlay", &["overlaid"]),
    ("overlie", &["overlain"]),
    ("overpay", &["overpaid"]),
    ("override", &["overridden"]),
    ("overrun", &["overrun"]),
    ("oversee", &["overseen"]),
    ("oversell", &["oversold"]),
    ("overshoot", &["overshot"]),
    ("oversleep", &["overslept"]),
    ("overspend", &["overspent"]),
    ("overtake", &["overtaken"]),
    ("overthrow", &["overthrown"]),
    ("overwrite", &["overwritten"]),
    ("partake", &["partaken"]),
    ("pay", &["paid"]),
    ("pen", &["penned", "pent"]),
    ("plead", &["pled"]),
    ("prepay", &["prepaid"]),
    ("preset", &["preset"]),
    ("proofread", &["proofread"]),
    ("prove", &["proven"]),
    ("put", &["put"]),
    ("quit", &["quit", "quitted"]),
    ("read", &["read"]),
    ("recast", &["recast"]),
    ("redo", &["redone"]),
    ("remake", &["remade"]),
    ("rend", &["rent"]),
    ("repay", &["repaid"]),
    ("reread", &["reread"]),
    ("rerun", &["rerun"]),
    ("reset", &["reset"]),
    ("retake", &["retaken"]),
    ("rethink", &["rethought"]),
    ("rewind", &["rewound"]),
    ("rewrite", &["rewritten"]),
    ("rid", &["rid", "ridded"]),
    ("ride", &["ridden"]),
    ("ring", &["rung"]),
    ("rise", &["risen"]),
    ("rive", &["riven"]),
    ("run", &["run"]),
    ("saw", &["sawn"]),
    ("say", &["said"]),
    ("see", &["seen"]),
    ("seek", &["sought"]),
    ("sell", &["sold"]),
    ("send", &["sent"]),
    ("set", &["set"]),
    ("sew", &["sewn"]),
    ("shake", &["shaken"]),
    ("shave", &["shaven"]),
    ("shed", &["shed"]),
    ("shine", &["shone"]),
    ("shit", &["shit", "shitted", "shat"]),
    ("shoe", &["shod"]),
    ("shoot", &["shot"]),
    ("show", &["shown"]),
    ("shrink", &["shrunk", "shrunken"]),
    ("shrive", &["shriven"]),
    ("shut", &["shut"]),
    ("sing", &["sung"]),
    ("sink", &["sunk", "sunken"]),
    ("sit", &["sat"]),
    ("slay", &["slain"]),
    ("sleep", &["slept"]),
    ("slide", &["slid", "slidden"]),
    ("sling", &["slung"]),
    ("slink", &["slunk"]),
    ("slit", &["slit"]),
    ("smell", &["smelt"]),
    ("smite", &["smitten"]),
    ("sneak", &["snuck"]),
    ("sow", &["sown"]),
    ("speak", &["spoken"]),
    ("speed", &["sped"]),
    ("spell", &["spelt"]),
    ("spend", &["spent"]),
    ("spill", &["spilt"]),
    ("spin", &["spun"]),
    ("spit", &["spat", "spit"]),
    ("split", &["split"]),
    ("spoil", &["spoilt"]),
    ("spread", &["spread"]),
    ("spring", &["sprung", "sprang"]),
    ("stand", &["stood"]),
    ("stave", &["stove"]),
    ("steal", &["stolen"]),
    ("stick", &["stuck"]),
    ("sting", &["stung"]),
    ("stink", &["stunk"]),
    ("strew", &["strewn"]),
    ("stride", &["stridden"]),
    ("strike", &["struck"]),
    ("string", &["strung"]),
    ("strive", &["striven"]),
    ("sublet", &["sublet"]),
    ("swear", &["sworn"]),
    ("sweat", &["sweat"]),
    ("sweep", &["swept"]),
    ("swell", &["swollen"]),
    ("swim", &["swum"]),
    ("swing", &["swung"]),
    ("take", &["taken"]),
    ("teach", &["taught"]),
    ("tear", &["torn"]),
    ("tell", &["told"]),
    ("think", &["thought"]),
    ("thrive", &["thriven"]),
    ("throw", &["thrown"]),
    ("thrust", &["thrust"]),
    ("tread", &["trodden", "trod"]),
    ("undercut", &["undercut"]),
    ("undergo", &["undergone"]),
    ("underlie", &["underlain"]),
    ("underpay", &["underpaid"]),
    ("undersell", &["undersold"]),
    ("understand", &["understood"]),
    ("undertake", &["undertaken"]),
    ("underwrite", &["underwritten"]),
    ("undo", &["undone"]),
    ("unfreeze", &["unfrozen"]),
    ("unmake", &["unmade"]),
    ("unwind", &["unwound"]),
    ("uphold", &["upheld"]),
    ("upset", &["upset"]),
    ("wake", &["woken"]),
    ("waylay", &["waylaid"]),
    ("wear", &["worn"]),
    ("weave", &["woven"]),
    ("wed", &["wed", "wedded"]),
    ("weep", &["wept"]),
    ("wet", &["wet", "wetted"]),
    ("win", &["won"]),
    ("wind", &["wound"]),
    ("withdraw", &["withdrawn"]),
    ("withhold", &["withheld"]),
    ("withstand", &["withstood"]),
    ("work", &["wrought"]),
    ("wring", &["wrung"]),
    ("write", &["written"]),
];

/// Whether `base`'s final consonant doubles before a vowel-initial suffix: a
/// **monosyllabic** consonant-vowel-consonant stem whose final letter is not `w/x/y`
/// (stop → stopp·ing/ed, run → runn·ing). Polysyllabic stress-final doublers (begin →
/// beginning) are not detected — they fall through undoubled (fail-closed).
fn doubles_final(base: &str) -> bool {
    let b = base.as_bytes();
    let n = b.len();
    if n < 3 {
        return false;
    }
    let is_vowel = |c: u8| matches!(c, b'a' | b'e' | b'i' | b'o' | b'u');
    let (last, mid, pre) = (b[n - 1], b[n - 2], b[n - 3]);
    if is_vowel(last) || matches!(last, b'w' | b'x' | b'y') || !is_vowel(mid) || is_vowel(pre) {
        return false;
    }
    // monosyllabic: exactly one vowel group in the whole stem.
    let mut groups = 0;
    let mut in_vowel = false;
    for &c in b {
        let v = is_vowel(c) || c == b'y';
        if v && !in_vowel {
            groups += 1;
        }
        in_vowel = v;
    }
    groups == 1
}

/// The **gerund / present participle** of `base` (`affect → affecting`). Pure
/// orthography — English has no irregular present participles (D63 §8.9 6-aux).
pub fn gerund(base: &str) -> String {
    if let Some(stem) = base.strip_suffix("ie") {
        return format!("{stem}ying"); // die → dying, lie → lying
    }
    if base.ends_with("ee") || base.ends_with("oe") || base.ends_with("ye") {
        return format!("{base}ing"); // see → seeing, hoe → hoeing, dye → dyeing
    }
    if base.len() >= 3 {
        if let Some(stem) = base.strip_suffix('e') {
            return format!("{stem}ing"); // make → making (silent-e drop)
        }
    }
    if doubles_final(base) {
        let last = &base[base.len() - 1..];
        return format!("{base}{last}ing"); // stop → stopping, run → running
    }
    format!("{base}ing") // be → being, play → playing, open → opening
}

/// The regular `-ed` past participle of `base` (`affect → affected`).
fn regular_ed(base: &str) -> String {
    if base.ends_with('e') {
        return format!("{base}d"); // love → loved, use → used
    }
    let b = base.as_bytes();
    if b.len() >= 2
        && b[b.len() - 1] == b'y'
        && !matches!(b[b.len() - 2], b'a' | b'e' | b'i' | b'o' | b'u')
    {
        return format!("{}ied", &base[..base.len() - 1]); // carry → carried
    }
    if doubles_final(base) {
        let last = &base[base.len() - 1..];
        return format!("{base}{last}ed"); // stop → stopped
    }
    format!("{base}ed") // affect → affected, play → played
}

/// The **past participle(s)** of `base`. An irregular base ([`IRREGULAR_PP`]) returns
/// its grounded form(s), plus — when the attested form is a devoiced `-t` (burnt) — the
/// productive regular twin (burned). A regular base returns the single `-ed` form.
pub fn past_participles(base: &str) -> Vec<String> {
    match IRREGULAR_PP.binary_search_by(|(b, _)| (*b).cmp(base)) {
        Ok(i) => {
            let forms = IRREGULAR_PP[i].1;
            let mut out: Vec<String> = forms.iter().map(|s| (*s).to_string()).collect();
            // `-t`/`-ed` twin recovery: an attested `base + "t"` (burnt, learnt, leapt)
            // implies the productive regular twin (burned, learned, leaped). Keyed on the
            // exact devoiced `-t`, so strong forms (worn) never admit a non-word (weared).
            if forms.iter().any(|f| *f == format!("{base}t")) {
                let reg = regular_ed(base);
                if !out.contains(&reg) {
                    out.push(reg);
                }
            }
            out
        }
        Err(_) => vec![regular_ed(base)],
    }
}

/// The **third-person-singular present** of `base` (`affect → affects`, the finite form
/// that heads a declarative; D63 §8.9 6-aux importer morphology). Regular `-s` with the
/// orthographic `-es` (sibilants + `o`) and consonant-`y → -ies` rules, plus the two
/// genuinely irregular auxiliaries (`be → is`, `have → has`; `do`/`go` are regular `-es`).
pub fn third_singular(base: &str) -> String {
    match base {
        "be" => return "is".to_string(),
        "have" => return "has".to_string(),
        _ => {}
    }
    let b = base.as_bytes();
    let n = b.len();
    let last = b[n - 1];
    // sibilants (s/x/z, -ch, -sh) and final -o take -es: kiss→kisses, fix→fixes,
    // watch→watches, push→pushes, go→goes, do→does.
    if matches!(last, b's' | b'x' | b'z' | b'o') || base.ends_with("ch") || base.ends_with("sh") {
        return format!("{base}es");
    }
    // consonant + y → -ies (carry→carries); vowel + y keeps the y (play→plays).
    if last == b'y' && n >= 2 && !matches!(b[n - 2], b'a' | b'e' | b'i' | b'o' | b'u') {
        return format!("{}ies", &base[..n - 1]);
    }
    format!("{base}s")
}

/// Irregular comparison: base → (comparative(s), superlative(s)). The textbook
/// suppletives + `shy` (keeps the `y`); everything else is regular `-er`/`-est` or
/// periphrastic. Grounded: validated against a ~200-adjective comparison list and the
/// in-repo WordNet `adj.exc` (D63 §8.12). Sorted by base for binary search.
const IRREGULAR_COMPARISON: &[(&str, &[&str], &[&str])] = &[
    ("bad", &["worse"], &["worst"]),
    ("far", &["farther", "further"], &["farthest", "furthest"]),
    ("good", &["better"], &["best"]),
    ("little", &["less", "littler"], &["least", "littlest"]),
    ("many", &["more"], &["most"]),
    ("much", &["more"], &["most"]),
    ("old", &["older", "elder"], &["oldest", "eldest"]),
    ("shy", &["shyer"], &["shyest"]),
];

/// The comparative/superlative of a gradable adjective (D63 §8.12 Slice 6-cmp).
/// `Synthetic` carries the `-er`/`-est` form(s); `Periphrastic` means the comparison is
/// `more`/`most` + the base (handled in the grammar), used for the long adjectives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Comparison {
    Synthetic {
        comparative: Vec<String>,
        superlative: Vec<String>,
    },
    Periphrastic,
}

/// Vowel-group count, **not** counting a silent final `e` (so cute/large read as
/// monosyllabic). A `y` counts as a vowel.
fn count_syllables(w: &str) -> usize {
    let stem = if w.ends_with('e') && !w.ends_with("ee") && w.len() > 2 {
        &w[..w.len() - 1]
    } else {
        w
    };
    let mut n = 0;
    let mut prev = false;
    for c in stem.chars() {
        let v = matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'y');
        if v && !prev {
            n += 1;
        }
        prev = v;
    }
    n.max(1)
}

/// Whether an adjective forms its comparison **periphrastically** (`more`/`most`):
/// not monosyllabic, and not a 2-syllable adjective ending in `-y`/`-le`/`-er`/`-ow`
/// (which stay synthetic — happy, simple, clever, narrow). Fuzzy + low-stakes (the
/// synthetic/periphrastic choice is genuinely variable; a wrong synthetic guess just
/// isn't looked up, and `more X` via the grammar still parses).
fn is_periphrastic(base: &str) -> bool {
    let syl = count_syllables(base);
    if syl <= 1 {
        return false;
    }
    if syl == 2
        && (base.ends_with('y')
            || base.ends_with("le")
            || base.ends_with("er")
            || base.ends_with("ow"))
    {
        return false;
    }
    true
}

/// The regular synthetic comparison stem suffixing (`-er`/`-est` family): `e`-final →
/// `+r`/`+st`, consonant-`y` → `-ier`/`-iest`, monosyllabic-CVC doubling, else `+er`/`+est`.
fn regular_comparison(base: &str, er: &str) -> String {
    if base.ends_with('e') {
        return format!("{base}{}", if er == "er" { "r" } else { "st" });
    }
    let b = base.as_bytes();
    if b.len() >= 2
        && b[b.len() - 1] == b'y'
        && !matches!(b[b.len() - 2], b'a' | b'e' | b'i' | b'o' | b'u')
    {
        return format!("{}i{er}", &base[..base.len() - 1]);
    }
    if doubles_final(base) {
        let last = &base[base.len() - 1..];
        return format!("{base}{last}{er}");
    }
    format!("{base}{er}")
}

/// The [`Comparison`] of a gradable adjective: irregular table → synthetic suppletive;
/// else periphrastic (long) or regular synthetic (`-er`/`-est`).
pub fn comparison(base: &str) -> Comparison {
    if let Ok(i) = IRREGULAR_COMPARISON.binary_search_by(|(b, _, _)| (*b).cmp(base)) {
        let (_, comp, sup) = IRREGULAR_COMPARISON[i];
        return Comparison::Synthetic {
            comparative: comp.iter().map(|s| (*s).to_string()).collect(),
            superlative: sup.iter().map(|s| (*s).to_string()).collect(),
        };
    }
    if is_periphrastic(base) {
        return Comparison::Periphrastic;
    }
    Comparison::Synthetic {
        comparative: vec![regular_comparison(base, "er")],
        superlative: vec![regular_comparison(base, "est")],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gerund_regular_and_orthographic_edges() {
        assert_eq!(gerund("affect"), "affecting");
        assert_eq!(gerund("depend"), "depending");
        assert_eq!(gerund("make"), "making"); // silent-e drop
        assert_eq!(gerund("write"), "writing");
        assert_eq!(gerund("use"), "using");
        assert_eq!(gerund("stop"), "stopping"); // CVC doubling
        assert_eq!(gerund("run"), "running");
        assert_eq!(gerund("swim"), "swimming");
        assert_eq!(gerund("die"), "dying"); // ie → y
        assert_eq!(gerund("lie"), "lying");
        assert_eq!(gerund("see"), "seeing"); // ee retained
        assert_eq!(gerund("dye"), "dyeing"); // ye retained
        assert_eq!(gerund("be"), "being"); // short word, no e-drop
        assert_eq!(gerund("play"), "playing"); // y, no doubling
        assert_eq!(gerund("fix"), "fixing"); // x, no doubling
        assert_eq!(gerund("open"), "opening"); // polysyllabic, no doubling
    }

    #[test]
    fn regular_past_participle_orthography() {
        assert_eq!(past_participles("affect"), ["affected"]);
        assert_eq!(past_participles("depend"), ["depended"]);
        assert_eq!(past_participles("use"), ["used"]); // e → d
        assert_eq!(past_participles("carry"), ["carried"]); // cons-y → ied
        assert_eq!(past_participles("play"), ["played"]); // vowel-y → ed
        assert_eq!(past_participles("stop"), ["stopped"]); // doubling
    }

    #[test]
    fn irregular_past_participle_from_table() {
        assert_eq!(past_participles("go"), ["gone"]);
        assert_eq!(past_participles("make"), ["made"]);
        assert_eq!(past_participles("take"), ["taken"]);
        assert_eq!(past_participles("write"), ["written"]);
        assert_eq!(past_participles("see"), ["seen"]);
        assert_eq!(past_participles("get"), ["got", "gotten"]); // two variants
        assert_eq!(past_participles("cut"), ["cut"]); // invariant
        assert_eq!(past_participles("put"), ["put"]);
        assert_eq!(past_participles("read"), ["read"]);
    }

    #[test]
    fn t_ed_twin_recovery_without_admitting_non_words() {
        // The attested `-t` form yields the regular twin too…
        assert_eq!(past_participles("burn"), ["burnt", "burned"]);
        assert_eq!(past_participles("learn"), ["learnt", "learned"]);
        assert_eq!(past_participles("leap"), ["leapt", "leaped"]);
        // …but a strong form never admits a non-word regular.
        assert_eq!(past_participles("wear"), ["worn"]); // not "weared"
        assert_eq!(past_participles("creep"), ["crept"]); // not "creeped"
    }

    #[test]
    fn third_singular_present() {
        assert_eq!(third_singular("affect"), "affects");
        assert_eq!(third_singular("breathe"), "breathes");
        assert_eq!(third_singular("take"), "takes");
        assert_eq!(third_singular("kiss"), "kisses"); // sibilant -es
        assert_eq!(third_singular("fix"), "fixes");
        assert_eq!(third_singular("watch"), "watches");
        assert_eq!(third_singular("push"), "pushes");
        assert_eq!(third_singular("go"), "goes"); // -o → -es
        assert_eq!(third_singular("do"), "does");
        assert_eq!(third_singular("carry"), "carries"); // cons-y → -ies
        assert_eq!(third_singular("play"), "plays"); // vowel-y → -s
        assert_eq!(third_singular("be"), "is"); // irregular
        assert_eq!(third_singular("have"), "has");
    }

    #[test]
    fn comparison_regular_irregular_and_periphrastic() {
        use Comparison::*;
        let syn = |c: &str, s: &str| Synthetic {
            comparative: vec![c.into()],
            superlative: vec![s.into()],
        };
        assert_eq!(comparison("large"), syn("larger", "largest")); // e-final
        assert_eq!(comparison("happy"), syn("happier", "happiest")); // consonant-y
        assert_eq!(comparison("big"), syn("bigger", "biggest")); // CVC doubling
        assert_eq!(comparison("high"), syn("higher", "highest")); // plain
        assert_eq!(comparison("cute"), syn("cuter", "cutest")); // silent-e monosyllable
        assert_eq!(comparison("simple"), syn("simpler", "simplest")); // 2-syll -le
        assert_eq!(comparison("narrow"), syn("narrower", "narrowest")); // 2-syll -ow
        assert_eq!(comparison("clever"), syn("cleverer", "cleverest")); // 2-syll -er
        assert_eq!(comparison("good"), syn("better", "best")); // suppletive
        assert_eq!(comparison("bad"), syn("worse", "worst"));
        assert_eq!(comparison("shy"), syn("shyer", "shyest")); // keeps the y
        assert_eq!(comparison("beautiful"), Periphrastic); // 3-syllable → more/most
        assert_eq!(comparison("difficult"), Periphrastic);
    }

    #[test]
    fn irregular_comparison_table_is_sorted() {
        let mut prev = "";
        for (b, _, _) in IRREGULAR_COMPARISON {
            assert!(
                *b > prev,
                "IRREGULAR_COMPARISON must be sorted by base at {b:?}"
            );
            prev = b;
        }
    }

    #[test]
    fn table_is_sorted_unique_lowercase_nonempty() {
        // Structural invariants the binary search + grounding rely on (runs in CI;
        // the corpus attestation witness below needs the WordNet dict and is ignored).
        let mut prev = "";
        for (base, forms) in IRREGULAR_PP {
            assert!(
                *base > prev,
                "IRREGULAR_PP must be sorted+unique by base at {base:?}"
            );
            prev = base;
            assert_eq!(
                *base,
                base.to_lowercase(),
                "base must be lowercase: {base:?}"
            );
            assert!(!forms.is_empty(), "no empty form list: {base:?}");
            for f in *forms {
                assert!(
                    !f.is_empty() && **f == *f.to_lowercase(),
                    "form lowercase nonempty: {f:?}"
                );
            }
        }
    }

    /// The grounding gate: every irregular past participle the table ships must be an
    /// invariant (`pp == base`) or **attested in the in-repo WordNet `verb.exc`** as an
    /// inflection of that base. Re-runs the witness that built the table. Ignored by
    /// default — reads the (git-ignored) WordNet corpus; run with `--ignored`.
    #[test]
    #[ignore = "reads the in-repo WordNet 3.0 verb.exc; run with --ignored"]
    fn irregular_pp_attested_in_verb_exc() {
        use std::collections::{BTreeMap, BTreeSet};
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../references/WordNet-3.0/dict/verb.exc");
        let text = std::fs::read_to_string(&path).expect("read verb.exc");
        // inflected → {bases}
        let mut attested: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for line in text.lines() {
            let mut it = line.split_whitespace();
            if let Some(infl) = it.next() {
                let bases: BTreeSet<String> = it.map(String::from).collect();
                if !bases.is_empty() {
                    attested.entry(infl.to_string()).or_default().extend(bases);
                }
            }
        }
        let mut ungrounded = Vec::new();
        for (base, forms) in IRREGULAR_PP {
            for f in *forms {
                let ok = f == base // invariant participle
                    || attested.get(*f).is_some_and(|s| s.contains(*base));
                if !ok {
                    ungrounded.push(format!("{base} -> {f}"));
                }
            }
        }
        assert!(
            ungrounded.is_empty(),
            "ungrounded irregular pp entries (not invariant, not in verb.exc): {ungrounded:?}"
        );
    }
}
