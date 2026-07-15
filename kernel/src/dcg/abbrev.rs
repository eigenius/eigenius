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
//! **Abbreviation-definition extraction** — the Schwartz–Hearst algorithm (Schwartz & Hearst 2003,
//! *Pac Symp Biocomput*), which pulls `long form (SHORT)` pairs out of a document.
//!
//! Pure text in, pairs out: no layer, no lexicon, no kernel. That is the whole reason it is its own
//! module. It lived in `glossary.rs` next to the code that *grounds* those pairs to concepts and emits
//! `lexicon:LexicalEntry` resources — three concerns in one file, of which only this one is a
//! string algorithm. Grounding and emission (which do need the chain) stay in `super::glossary`.
//!
//! [`AbbreviationProposer`] is the seam for an LLM extractor: it proposes candidate definitions the
//! deterministic scan missed. Like every other proposer in the engine it only *proposes* — a definition
//! it invents still has to ground to a real concept and pass the felicity gate before it becomes an
//! entry.

use std::collections::BTreeSet;

/// One extracted abbreviation definition: the surface short form, the **minimal** long form that
/// defines it (Schwartz-Hearst), and the full candidate `context` window before the paren. The
/// context lets grounding retry a **fuller** long form when the minimal one doesn't match a lexicon
/// surface string (e.g. `MMR`'s minimal `mismatch repair` vs the lexicon's `DNA mismatch repair`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbbrDef {
    pub short_form: String,
    pub long_form: String,
    pub context: String,
}

/// A candidate short form (the parenthetical) is admissible per Schwartz-Hearst: 2–10 chars, at most
/// two tokens, first char alphanumeric, and at least one letter (so `(1c)` / `(a, b)` are rejected).
fn is_valid_short_form(s: &str) -> bool {
    let n = s.chars().count();
    if !(2..=10).contains(&n) {
        return false;
    }
    if s.split_whitespace().count() > 2 {
        return false;
    }
    let first_alnum = s.chars().next().is_some_and(|c| c.is_alphanumeric());
    first_alnum && s.chars().any(|c| c.is_alphabetic())
}

/// The core Schwartz-Hearst test: find the shortest suffix of `long` whose characters contain those of
/// `short` as an **ordered subsequence**, scanned right-to-left, with the short form's FIRST char
/// constrained to a word start in `long`. `None` if no such match (⇒ not a definition).
fn find_best_long_form(short: &str, long: &str) -> Option<String> {
    let s: Vec<char> = short.chars().collect();
    let l: Vec<char> = long.chars().collect();
    if s.is_empty() || l.is_empty() {
        return None;
    }
    let mut s_index = s.len() as isize - 1;
    let mut l_index = l.len() as isize - 1;
    while s_index >= 0 {
        let curr = s[s_index as usize].to_ascii_lowercase();
        if !curr.is_alphanumeric() {
            s_index -= 1;
            continue;
        }
        // Move left until `long[l_index]` matches `curr` AND — for the first short char — its left
        // neighbour is a word boundary (so the abbreviation's first letter starts a word).
        while (l_index >= 0 && l[l_index as usize].to_ascii_lowercase() != curr)
            || (s_index == 0 && l_index > 0 && l[(l_index - 1) as usize].is_alphanumeric())
        {
            l_index -= 1;
        }
        if l_index < 0 {
            return None;
        }
        l_index -= 1;
        s_index -= 1;
    }
    // The long form begins at the word-initial char the first short char matched (`l_index + 1`).
    let start = (l_index + 1).max(0) as usize;
    let long_form: String = l[start..].iter().collect();
    let long_form = long_form.trim().to_string();
    (!long_form.is_empty()).then_some(long_form)
}

/// A found long form is admissible if it is non-empty and no longer than `min(|SF|+5, |SF|·2)` words
/// (the Schwartz-Hearst length bound: a definition can't be arbitrarily longer than the abbreviation).
fn is_valid_long_form(short: &str, long: &str) -> bool {
    let sf_len = short.chars().count();
    let max_words = (sf_len + 5).min(sf_len * 2);
    let wc = long.split_whitespace().count();
    (1..=max_words).contains(&wc)
}

/// Extract `Long Form (SHORT)` abbreviation definitions from raw document text (Schwartz-Hearst).
/// Runs on the **raw** text — upstream of `strip_bracketed_asides`, which would drop the `(SHORT)`
/// parenthetical — so the binding is captured even though the body sentence later loses the paren.
/// First-seen wins per short form (deduped, case-insensitively).
pub fn extract_abbreviations(text: &str) -> Vec<AbbrDef> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '(' {
            i += 1;
            continue;
        }
        let Some(close) = (i + 1..chars.len()).find(|&j| chars[j] == ')') else {
            break;
        };
        let inner: String = chars[i + 1..close].iter().collect();
        let short = inner.trim();
        if is_valid_short_form(short) {
            // Candidate long form: the `min(|SF|+5, |SF|·2)` words immediately preceding the `(`.
            let pre: String = chars[..i].iter().collect();
            let sf_len = short.chars().count();
            let max_words = (sf_len + 5).min(sf_len * 2);
            let words: Vec<&str> = pre.split_whitespace().collect();
            let take = words.len().saturating_sub(max_words);
            let candidate = words[take..].join(" ");
            if let Some(long) = find_best_long_form(short, &candidate) {
                if is_valid_long_form(short, &long) && seen.insert(short.to_lowercase()) {
                    out.push(AbbrDef {
                        short_form: short.to_string(),
                        long_form: long,
                        context: candidate,
                    });
                }
            }
        }
        i = close + 1;
    }
    out
}

/// Proposes abbreviation definitions the deterministic extractor misses (non-parenthetical). Untrusted
/// — every proposal is validated before use. The no-op default is deterministic-only; a live Anthropic
/// impl is behind the `use-llm` feature.
pub trait AbbreviationProposer {
    /// Propose `(short, long)` definitions found in `text`. May be empty; may include spurious entries
    /// (the caller validates). `context` should be set to the long form (the proposer gives it whole).
    fn propose(&self, text: &str) -> Vec<AbbrDef>;
}

/// The no-op proposer: deterministic Schwartz-Hearst extraction only (the CI default, no LLM).
pub struct NoAbbreviationProposer;

impl AbbreviationProposer for NoAbbreviationProposer {
    fn propose(&self, _text: &str) -> Vec<AbbrDef> {
        Vec::new()
    }
}

/// Deterministic Schwartz-Hearst extraction UNION an untrusted proposer's suggestions, deduped by
/// short form (deterministic wins). Each proposal is **fail-closed validated**: a well-formed short
/// form ([`is_valid_short_form`]) that actually occurs in `text` (rejecting hallucinated
/// abbreviations); grounding + the kernel felicity gate validate the rest downstream.
pub fn extract_abbreviations_with(text: &str, proposer: &dyn AbbreviationProposer) -> Vec<AbbrDef> {
    let mut out = extract_abbreviations(text);
    let mut seen: BTreeSet<String> = out.iter().map(|d| d.short_form.to_lowercase()).collect();
    let text_lc = text.to_lowercase();
    for d in proposer.propose(text) {
        let sf = d.short_form.trim().to_lowercase();
        if is_valid_short_form(d.short_form.trim())
            && text_lc.contains(&sf)
            && !d.long_form.trim().is_empty()
            && seen.insert(sf)
        {
            out.push(d);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defs(text: &str) -> Vec<(String, String)> {
        extract_abbreviations(text)
            .into_iter()
            .map(|d| (d.short_form, d.long_form))
            .collect()
    }

    #[test]
    fn extracts_the_wrn_definitions() {
        // The real WRN first-page introductions (the ones the CNL-v2 rewrite dropped, §4b).
        assert_eq!(
            defs("cancers with microsatellite instability (MSI), which results from deficient DNA mismatch repair"),
            vec![("MSI".to_string(), "microsatellite instability".to_string())],
        );
        // `MMR` = **M**is**M**atch **R**epair — both M's come from `mismatch` (positions 0 and 3), so
        // Schwartz-Hearst returns the MINIMAL long form `mismatch repair`, correctly dropping the
        // unnecessary `DNA` modifier (the abbreviation doesn't need it).
        assert_eq!(
            defs("defects in DNA mismatch repair (MMR) promote a hypermutable state"),
            vec![("MMR".to_string(), "mismatch repair".to_string())],
        );
        // `MSI` matches a non-word-initial S and I (both from "instability") — the subsequence match,
        // not first-letters, is what makes this work.
        assert_eq!(
            find_best_long_form("MSI", "microsatellite instability").as_deref(),
            Some("microsatellite instability"),
        );
    }

    #[test]
    fn rejects_non_definitions() {
        // A figure/table reference is not an abbreviation definition: no matching long form.
        assert!(defs("we analysed the data (Fig. 1c)").is_empty());
        // A parenthetical aside whose chars don't subsequence-match the preceding text.
        assert!(defs("the result was clear (see below)").is_empty());
        // An over-long "short form" (>2 tokens) is not an abbreviation candidate.
        assert!(defs("the process (a slow and careful one) matters").is_empty());
    }

    #[test]
    fn short_form_validity() {
        assert!(is_valid_short_form("MSI"));
        assert!(is_valid_short_form("PARP-1"));
        assert!(!is_valid_short_form("a")); // too short
        assert!(!is_valid_short_form("123")); // no letter
        assert!(!is_valid_short_form("one two three")); // >2 tokens
    }

    #[test]
    fn dedups_repeated_definitions_first_seen_wins() {
        let text = "microsatellite instability (MSI) is common; later, MSI (microsatellite instability) recurs";
        assert_eq!(
            defs(text),
            vec![("MSI".to_string(), "microsatellite instability".to_string())],
        );
    }

    /// A stand-in proposer returning fixed suggestions (the deterministic mirror of the live LLM tail).
    struct MockProposer(Vec<AbbrDef>);
    impl AbbreviationProposer for MockProposer {
        fn propose(&self, _text: &str) -> Vec<AbbrDef> {
            self.0.clone()
        }
    }
    fn ad(short: &str, long: &str) -> AbbrDef {
        AbbrDef {
            short_form: short.to_string(),
            long_form: long.to_string(),
            context: long.to_string(),
        }
    }

    #[test]
    fn llm_tail_adds_non_parenthetical_and_rejects_hallucinations() {
        // No parenthetical → Schwartz-Hearst finds nothing; the proposer supplies the definition.
        let text = "MSI stands for microsatellite instability. MMR is a repair system.";
        assert!(extract_abbreviations(text).is_empty());

        let proposer = MockProposer(vec![
            ad("MSI", "microsatellite instability"), // valid: short form occurs in the text
            ad("XYZ", "not in the text at all"),     // HALLUCINATION: short form absent → rejected
            ad("a", "too short a form"),             // invalid short form → rejected
        ]);
        let got = extract_abbreviations_with(text, &proposer);
        assert_eq!(
            got.iter()
                .map(|d| d.short_form.as_str())
                .collect::<Vec<_>>(),
            vec!["MSI"],
            "only the valid, text-present proposal survives (fail-closed on hallucinations)"
        );
        assert_eq!(got[0].long_form, "microsatellite instability");
    }

    #[test]
    fn deterministic_wins_over_the_proposer_on_dedup() {
        // The parenthetical is extracted deterministically; a conflicting proposal for the same short
        // form is dropped (deterministic wins).
        let text = "microsatellite instability (MSI) matters";
        let proposer = MockProposer(vec![ad("MSI", "a wrong expansion")]);
        let got = extract_abbreviations_with(text, &proposer);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].long_form, "microsatellite instability");
    }

    #[test]
    fn no_proposer_is_deterministic_only() {
        let text = "MSI stands for microsatellite instability";
        assert!(extract_abbreviations_with(text, &NoAbbreviationProposer).is_empty());
    }

    /// Live end-to-end check of the Anthropic proposer: a non-parenthetical definition the
    /// Schwartz-Hearst extractor cannot see should be recovered. Requires `ANTHROPIC_API_KEY` and
    /// the `use-llm` feature; ignored by default (network + cost). Run with:
    /// `cargo test -p eigenius-kernel --features use-llm -- --ignored anthropic_proposer_live`
    #[cfg(feature = "use-llm")]
    #[test]
    #[ignore = "hits the live Anthropic API; requires ANTHROPIC_API_KEY"]
    fn anthropic_proposer_live_recovers_non_parenthetical() {
        let Some(proposer) = super::AnthropicAbbreviationProposer::from_env() else {
            panic!("ANTHROPIC_API_KEY not set");
        };
        // No parentheses → the deterministic extractor finds nothing; the LLM must supply it.
        let text = "The Werner syndrome protein, WRN, is a RecQ helicase. \
                    Microsatellite instability, or MSI, is a hallmark of mismatch repair deficiency.";
        assert!(extract_abbreviations(text).is_empty());
        let got = extract_abbreviations_with(text, &proposer);
        let shorts: Vec<_> = got.iter().map(|d| d.short_form.as_str()).collect();
        assert!(shorts.contains(&"WRN"), "expected WRN in {shorts:?}");
        assert!(shorts.contains(&"MSI"), "expected MSI in {shorts:?}");
    }
}
