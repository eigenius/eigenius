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

//! **Published notations** — identifiers whose form is specified, not guessed.
//!
//! The document glossary's other extraction sources read ORTHOGRAPHY: Schwartz-Hearst matches
//! `Long Form (SHORT)`, apposition matches `<common-noun> <Name>`. Both are heuristics over how
//! English happens to be written, and both need a precision guard — recurrence, a stop-list — to
//! stay honest.
//!
//! A notation needs none of that. `p.Ile380Thr` is an HGVS protein change because HGVS says what
//! one looks like, and `P98170` is a UniProt accession because UniProt publishes the pattern. The
//! grammar IS the evidence, so there is no threshold to tune and no recurrence to wait for.
//!
//! That distinction is why this is a source of its own rather than a loosening of
//! `eigenius_kernel::dcg::named_entity`: `Ile380Thr` fails that recognizer three times over — an interior
//! capital is not Title-case, `p.Ile380Thr` does not begin uppercase, and the document never puts
//! a noun head in front of either (`an`, `for`, `of`, `than`, a table pipe, or a verb). Loosening
//! the shape to admit it would also admit `mRNA` and `HeLa`, which that rule exists to reject.

pub mod hgvs;

/// What a token was recognised as, carrying the parse rather than a name.
///
/// A recognised token must NOT become an opaque individual. `p.Ile380Thr` parses into a reference
/// residue, a position and an alternate residue; minting `ni_ile380thr` would put that structure
/// back into a string, which is the ad-hoc-predicate failure the parser exists to prevent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notation {
    /// A protein change — `p.Ile380Thr`, `Ile380Thr`, `N108Q`, `p.Ser369Leufs*12`.
    ProteinChange(hgvs::Change),
    /// A coding change — `c.131T>C`, `c.1105_1107del`.
    CodingChange(hgvs::Change),
    /// A residue and its position with no change stated — `Arg381`, `Ile380`.
    ResiduePosition { residue: String, position: u32 },
    /// A UniProt accession — `P98170`, `A5D8Q0`.
    UniProt(String),
    /// A Protein Data Bank entry — `2KNA`.
    PdbEntry(String),
    /// A dbSNP reference SNP — `rs370533935`.
    DbSnp(String),
    /// A ClinVar variation record — `VCV004699404`.
    ClinVar(String),
    /// An Ensembl gene or transcript, or a RefSeq transcript — `ENSG00000101966`, `NM_001167.4`.
    SequenceRecord(String),
    /// A genomic coordinate — `chrX:123900530`.
    GenomicCoordinate(String),
}

/// The one-letter amino-acid codes, in the IUPAC set, plus `*` for a stop.
const ONE_LETTER: &str = "ACDEFGHIKLMNPQRSTVWY";

/// The three-letter amino-acid codes, plus `Ter` for a stop.
const THREE_LETTER: &[&str] = &[
    "Ala", "Arg", "Asn", "Asp", "Cys", "Gln", "Glu", "Gly", "His", "Ile", "Leu", "Lys", "Met",
    "Phe", "Pro", "Ser", "Thr", "Trp", "Tyr", "Val", "Ter", "Sec", "Pyl", "Xaa",
];

fn is_three_letter(r: &str) -> bool {
    THREE_LETTER.contains(&r)
}

/// `N108Q` / `L145T` — a protein change in ONE-letter codes, which is how an editor's mutations are
/// written (`ABE9 = ABE8e + N108Q + L145T`). Deliberately separate from [`hgvs::parse_p`], whose
/// contract refuses the one-letter form because the registry's alleles must not be read from it.
fn parse_one_letter_change(body: &str) -> Option<hgvs::Change> {
    let mut cs = body.chars();
    let ref_res = cs.next()?;
    if !ONE_LETTER.contains(ref_res) {
        return None;
    }
    let rest: String = cs.collect();
    let split = rest.find(|c: char| !c.is_ascii_digit())?;
    let (pos, alt) = rest.split_at(split);
    if pos.is_empty() || alt.chars().count() != 1 {
        return None;
    }
    let a = alt.chars().next()?;
    if !ONE_LETTER.contains(a) && a != '*' {
        return None;
    }
    Some(hgvs::Change {
        start: pos.parse().ok()?,
        end: pos.parse().ok()?,
        ref_state: Some(ref_res.to_string()),
        alt: alt.to_string(),
    })
}

/// `Arg381`, `Ile380` — a residue and its position with NO change stated. Distinct from a change:
/// the document says where something is, not what it becomes.
fn parse_residue_position(t: &str) -> Option<Notation> {
    if !t.is_ascii() || t.len() < 4 || !is_three_letter(&t[..3]) {
        return None;
    }
    let pos = &t[3..];
    if pos.is_empty() || !pos.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(Notation::ResiduePosition {
        residue: t[..3].to_string(),
        position: pos.parse().ok()?,
    })
}

/// A UniProt accession, in the two published shapes:
/// `[OPQ][0-9][A-Z0-9]{3}[0-9]` and `[A-NR-Z][0-9][A-Z][A-Z0-9]{2}[0-9]`.
fn is_uniprot(t: &str) -> bool {
    let b = t.as_bytes();
    let up = |i: usize| b[i].is_ascii_uppercase();
    let dg = |i: usize| b[i].is_ascii_digit();
    let an = |i: usize| b[i].is_ascii_uppercase() || b[i].is_ascii_digit();
    if b.len() == 6 {
        let first_opq = matches!(b[0], b'O' | b'P' | b'Q');
        if first_opq && dg(1) && an(2) && an(3) && an(4) && dg(5) {
            return true;
        }
        let a_nr_z = up(0) && !matches!(b[0], b'O' | b'P' | b'Q');
        return a_nr_z && dg(1) && up(2) && an(3) && an(4) && dg(5);
    }
    false
}

/// Are a change's reference and alternate states real amino acids?
///
/// [`hgvs::parse_p`] checks a residue by SHAPE — three characters, Title-case — which is right for
/// a registry reading curated rows, and wrong here: over arbitrary prose it reads `Ile380Xyz` as a
/// protein change. A notation source must validate against the actual IUPAC set, so this re-checks
/// what `parse_p` admitted rather than loosening a contract the registry depends on. (Caught by
/// the negative test, not by inspection.)
fn is_real_protein_change(c: &hgvs::Change) -> bool {
    let residue_ok = |r: &str| match r.len() {
        1 => ONE_LETTER.contains(r),
        3 => is_three_letter(r),
        _ => false,
    };
    let ref_ok = c.ref_state.as_deref().is_some_and(residue_ok);
    // The alternate is a residue, a stop, or a frameshift whose first residue is one.
    let alt_ok = residue_ok(&c.alt)
        || c.alt == "*"
        || (c.alt.len() > 3 && is_three_letter(&c.alt[..3]) && c.alt[3..].starts_with("fs"));
    ref_ok && alt_ok
}

/// Does the token's sequence position read as HGVS writes one — at least one digit, no leading
/// zero? `Ile0380Thr` is not HGVS, and admitting it would make this source's grammar-is-the-
/// evidence claim false in the only place it is checkable.
fn position_well_formed(token: &str) -> bool {
    let digits: String = token
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    !digits.is_empty() && !digits.starts_with('0')
}

/// Recognise one whitespace-delimited token, stripped of surrounding punctuation by the caller.
///
/// Returns `None` for anything whose grammar does not match — this source never guesses. Order
/// matters only where two grammars could both match; each arm below is disjoint in practice.
pub fn recognize(token: &str) -> Option<Notation> {
    let t = token.trim();
    // Every notation here is ASCII by specification, so a token carrying anything else is not one
    // of them. Stated once, at the top, rather than guarded in each arm — and it is also what
    // keeps the byte-indexed readers below from meeting a multi-byte char at all.
    if t.is_empty() || !t.is_ascii() {
        return None;
    }
    // Changes, prefixed. `parse_p`/`parse_c` own their contracts; this only routes to them.
    if !t.starts_with("chr") && t.chars().any(|c| c.is_ascii_digit()) && !position_well_formed(t) {
        // A malformed position disqualifies every change and residue arm below. Accessions and
        // coordinates are exempt: `A5D8Q0` and `chrX:0123` carry no sequence position.
        if t.starts_with("p.") || t.starts_with("c.") || parse_residue_position(t).is_some() {
            return None;
        }
        if hgvs::parse_p(&format!("p.{t}")).is_ok() || parse_one_letter_change(t).is_some() {
            return None;
        }
    }
    if t.starts_with("p.") {
        if let Ok(c) = hgvs::parse_p(t) {
            return is_real_protein_change(&c).then_some(Notation::ProteinChange(c));
        }
        if let Some(c) = parse_one_letter_change(t.trim_start_matches("p.")) {
            return Some(Notation::ProteinChange(c));
        }
        return None;
    }
    if t.starts_with("c.") {
        return hgvs::parse_c(t).ok().map(Notation::CodingChange);
    }
    // Bare changes, as the report also writes them (`Ile380Thr`, `N108Q`). `parse_p` refuses an
    // unprefixed notation — correct for a registry, where coding and protein coordinates are
    // ambiguous — so the prefix is supplied here, for a glossary that only needs the parts.
    if let Ok(c) = hgvs::parse_p(&format!("p.{t}")) {
        if is_real_protein_change(&c) {
            return Some(Notation::ProteinChange(c));
        }
    }
    if let Some(n) = parse_residue_position(t) {
        return Some(n);
    }
    if let Some(c) = parse_one_letter_change(t) {
        return Some(Notation::ProteinChange(c));
    }
    // Accessions and records, each by its published pattern.
    if let Some(rest) = t.strip_prefix("rs") {
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
            return Some(Notation::DbSnp(t.to_string()));
        }
    }
    if let Some(rest) = t.strip_prefix("VCV") {
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
            return Some(Notation::ClinVar(t.to_string()));
        }
    }
    if is_uniprot(t) {
        return Some(Notation::UniProt(t.to_string()));
    }
    if (t.starts_with("ENSG") || t.starts_with("ENST") || t.starts_with("NM_"))
        && t.chars().any(|c| c.is_ascii_digit())
    {
        return Some(Notation::SequenceRecord(t.to_string()));
    }
    if let Some(rest) = t.strip_prefix("chr") {
        if rest.contains(':')
            && rest
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric())
        {
            return Some(Notation::GenomicCoordinate(t.to_string()));
        }
    }
    // A PDB id is four characters, a digit then three alphanumerics (`2KNA`). Last, because the
    // pattern is loose enough to shadow a short accession if tried earlier.
    let b = t.as_bytes();
    if b.len() == 4
        && b[0].is_ascii_digit()
        && b[0] != b'0'
        && b[1..].iter().all(|c| c.is_ascii_alphanumeric())
        && t.chars().any(|c| c.is_ascii_alphabetic())
    {
        return Some(Notation::PdbEntry(t.to_string()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn protein(t: &str) -> hgvs::Change {
        match recognize(t) {
            Some(Notation::ProteinChange(c)) => c,
            other => panic!("{t} should be a protein change, got {other:?}"),
        }
    }

    #[test]
    fn the_tokens_that_blocked_the_xiap_report() {
        // The 20-unit blocker, in both forms the document writes.
        for t in ["p.Ile380Thr", "Ile380Thr"] {
            let c = protein(t);
            assert_eq!(
                (c.start, c.ref_state.as_deref(), c.alt.as_str()),
                (380, Some("Ile"), "Thr"),
                "{t}"
            );
        }
        assert_eq!(protein("p.Ile380Val").alt, "Val");
        assert_eq!(protein("p.Ala379Thr").start, 379);
        assert_eq!(protein("p.Arg381Ter").alt, "Ter");

        // ONE-letter, how an editor's mutations are written (`ABE9 = ABE8e + N108Q + L145T`).
        let c = protein("L145T");
        assert_eq!(
            (c.start, c.ref_state.as_deref(), c.alt.as_str()),
            (145, Some("L"), "T")
        );
        assert_eq!(protein("N108Q").start, 108);

        assert_eq!(
            recognize("Arg381"),
            Some(Notation::ResiduePosition {
                residue: "Arg".into(),
                position: 381
            })
        );
        assert_eq!(
            recognize("P98170"),
            Some(Notation::UniProt("P98170".into()))
        );
        assert_eq!(
            recognize("A5D8Q0"),
            Some(Notation::UniProt("A5D8Q0".into()))
        );
        assert_eq!(recognize("2KNA"), Some(Notation::PdbEntry("2KNA".into())));
        assert_eq!(
            recognize("rs370533935"),
            Some(Notation::DbSnp("rs370533935".into()))
        );
        assert_eq!(
            recognize("VCV004699404"),
            Some(Notation::ClinVar("VCV004699404".into()))
        );
        assert_eq!(
            recognize("ENSG00000101966"),
            Some(Notation::SequenceRecord("ENSG00000101966".into()))
        );
        assert_eq!(
            recognize("NM_001167.4"),
            Some(Notation::SequenceRecord("NM_001167.4".into()))
        );
        assert!(matches!(
            recognize("chrX:123900530"),
            Some(Notation::GenomicCoordinate(_))
        ));
        assert!(matches!(
            recognize("c.1141C>T"),
            Some(Notation::CodingChange(_))
        ));
    }

    /// The precision side, which matters more: this source runs over OOV tokens and must not
    /// reclassify the document's ordinary vocabulary as notation. Every token below is one the
    /// XIAP corpus actually contains.
    #[test]
    fn what_is_not_a_notation_is_refused() {
        for t in [
            // The product names that remain OOV after this source — editors, predictors,
            // databases, versions. Recognising any as notation would be a false ground.
            "ABE9",
            "ABE8e",
            "ABE-NW1",
            "ABE7.10",
            "SIFT4G",
            "VEST4",
            "BayesDel",
            "AlphaFold",
            "AlphaMissense",
            "ClinPred",
            "MaveDB",
            "HGMD",
            "SpliceAI",
            "SiPhy",
            "LOVD",
            "dbNSFP",
            "v4.1",
            "4.8a",
            "2026AA",
            // Ordinary words, including the residue codes standing alone.
            "Thr",
            "Ile",
            "Ter",
            "the",
            "residue",
            "dimerisation",
            "paywalled",
            "helix",
            // Near-misses by shape.
            "Ile",
            "Ile380Xyz",
            "p.Ile380",
            "p.",
            "c.",
            "rs",
            "VCV",
            "chrX",
            "Ile0380Thr",
            // Years and bare numbers, which a loose coordinate or PDB pattern would swallow.
            "2026",
            "1998",
            "380",
            "0KNA",
            // Non-ASCII, which byte-indexed readers must refuse rather than panic on.
            "3₁₀",
            "α1",
            "Met375–Arg381",
            "p.Ilé380Thr",
        ] {
            assert_eq!(
                recognize(t),
                None,
                "`{t}` must not be recognised as notation"
            );
        }
    }

    /// A known ambiguity, recorded rather than hidden: the published UniProt pattern also admits
    /// gene symbols of the shape `letter digit …` (`P2RY12`). The mitigation is structural — this
    /// source is an OOV closure, so it only ever sees tokens the lexicon failed to supply, and a
    /// gene symbol the lexicon knows never reaches it.
    #[test]
    fn the_uniprot_pattern_collides_with_some_gene_symbols() {
        assert_eq!(
            recognize("P2RY12"),
            Some(Notation::UniProt("P2RY12".into()))
        );
    }

    /// `parse_p`'s contract is untouched by the move: the registry still refuses the one-letter
    /// form, and only `recognize` admits it.
    #[test]
    fn the_registrys_refusal_still_stands() {
        assert!(hgvs::parse_p("p.L44P").is_err());
        assert!(matches!(
            recognize("p.L44P"),
            Some(Notation::ProteinChange(_))
        ));
    }
}
