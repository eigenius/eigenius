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

//! D97 — the UMLS SPECIALIST Lexicon, read.
//!
//! The NLM Lexical Systems Group's `LEXICON` file (`references/specialist/LEXICON`, provisioned by
//! `scripts/provision-specialist.sh`) lists one record per lexical item:
//!
//! ```text
//! {base=essential
//! entry=E0026171
//!     cat=adj
//!     variants=inv;periph
//!     compl=pphr(to,np)
//!     compl=pphr(for,np)
//!     nominalization=essentiality|noun|E0220318
//! }
//! ```
//!
//! (Fields are indented by a tab in the file.) [`Lexicon::parse`] keeps every field as written and indexes each record under its base and its
//! spelling variants, lowercased, with its category. The accessors read the fields D97 uses: the
//! prepositions a complement names ([`Record::pp_prepositions`]) and the nominalizations
//! ([`Record::nominalizations`]). SPECIALIST speaks for a lemma, not a sense; which senses a fact
//! goes on is the importer's decision (D97, decisions 1 and 7).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

/// One SPECIALIST record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub base: String,
    /// The record's EUI, `E0026171`.
    pub entry: String,
    pub spelling_variants: Vec<String>,
    /// Every field after `entry=`, in order, `cat` included: `("compl", "pphr(to,np)")`.
    pub fields: Vec<(String, String)>,
}

/// A nominalization a record names: `nominalization=essentiality|noun|E0220318`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nominalization {
    pub lemma: String,
    pub cat: String,
    pub entry: String,
}

impl Record {
    /// The record's category — `noun`, `verb`, `adj`, … — or empty if it has none.
    pub fn cat(&self) -> &str {
        self.values("cat").next().unwrap_or("")
    }

    /// The values of the field `key`, in order.
    pub fn values<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.fields
            .iter()
            .filter(move |(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// The prepositions the complements in field `key` name, `pphr(p, …)`: an adjective's or a noun's
    /// `compl` (`essential` → `for`, `to`), a verb's `tran`. The object after the preposition may be a
    /// noun phrase, a gerund or a clause; each names `p`.
    pub fn pp_prepositions(&self, key: &str) -> BTreeSet<String> {
        self.values(key).filter_map(pphr_preposition).collect()
    }

    /// The nominalizations the record names.
    pub fn nominalizations(&self) -> Vec<Nominalization> {
        self.values("nominalization")
            .filter_map(|v| {
                let mut parts = v.split('|');
                Some(Nominalization {
                    lemma: parts.next()?.to_string(),
                    cat: parts.next()?.to_string(),
                    entry: parts.next().unwrap_or("").to_string(),
                })
            })
            .collect()
    }
}

/// The preposition a `pphr(p, object)` complement names; `None` for any other complement.
fn pphr_preposition(value: &str) -> Option<String> {
    let (p, _) = value.strip_prefix("pphr(")?.split_once(',')?;
    let p = p.trim();
    (!p.is_empty() && p.chars().all(|c| c.is_ascii_lowercase() || c == ' ')).then(|| p.to_string())
}

/// A line of `LEXICON` that is no part of a record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// 1-based.
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LEXICON line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

/// The lexicon: every record, indexed by lowercased form and category.
#[derive(Debug, Clone, Default)]
pub struct Lexicon {
    records: Vec<Record>,
    index: BTreeMap<(String, String), Vec<usize>>,
}

impl Lexicon {
    /// Read and parse `LEXICON` at `path`.
    pub fn read(path: &Path) -> Result<Lexicon, Box<dyn std::error::Error>> {
        Ok(Self::parse(&std::fs::read_to_string(path)?)?)
    }

    /// Parse the text of `LEXICON`. A record opens with `{base=`, lists its `spelling_variant=` and
    /// `entry=` lines, then its fields indented by a tab, and closes with `}`; any other line is an
    /// error, so a changed format fails rather than yielding a partial lexicon.
    pub fn parse(text: &str) -> Result<Lexicon, ParseError> {
        let mut lexicon = Lexicon::default();
        let mut open: Option<Record> = None;
        for (i, line) in text.lines().enumerate() {
            let err = |message: &str| ParseError {
                line: i + 1,
                message: message.to_string(),
            };
            if let Some(base) = line.strip_prefix("{base=") {
                if open.is_some() {
                    return Err(err("a record opens before the last one closed"));
                }
                open = Some(Record {
                    base: base.to_string(),
                    entry: String::new(),
                    spelling_variants: Vec::new(),
                    fields: Vec::new(),
                });
                continue;
            }
            if line.trim().is_empty() {
                continue;
            }
            let Some(rec) = open.as_mut() else {
                return Err(err("a line outside a record"));
            };
            if line == "}" {
                if rec.entry.is_empty() {
                    return Err(err("a record with no entry= line"));
                }
                lexicon.insert(open.take().expect("open record"));
            } else if let Some(v) = line.strip_prefix("spelling_variant=") {
                rec.spelling_variants.push(v.to_string());
            } else if let Some(v) = line.strip_prefix("entry=") {
                rec.entry = v.to_string();
            } else if let Some((k, v)) = line.strip_prefix('\t').and_then(|f| f.split_once('=')) {
                rec.fields.push((k.to_string(), v.to_string()));
            } else if let Some(flag) = line.strip_prefix('\t') {
                // A field with no value: `stative`, `proper`.
                rec.fields.push((flag.to_string(), String::new()));
            } else {
                return Err(err("a line that is neither a field nor a record boundary"));
            }
        }
        if open.is_some() {
            return Err(ParseError {
                line: text.lines().count(),
                message: "the last record does not close".to_string(),
            });
        }
        Ok(lexicon)
    }

    fn insert(&mut self, rec: Record) {
        let at = self.records.len();
        let cat = rec.cat().to_string();
        let mut forms: BTreeSet<String> = rec
            .spelling_variants
            .iter()
            .map(|s| s.to_lowercase())
            .collect();
        forms.insert(rec.base.to_lowercase());
        for form in forms {
            self.index.entry((form, cat.clone())).or_default().push(at);
        }
        self.records.push(rec);
    }

    /// Every record, in file order.
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// The records of category `cat` that `form` spells, as a base or a spelling variant, ignoring
    /// case.
    pub fn lookup<'a>(&'a self, form: &str, cat: &str) -> impl Iterator<Item = &'a Record> + 'a {
        self.index
            .get(&(form.to_lowercase(), cat.to_string()))
            .into_iter()
            .flatten()
            .map(move |&i| &self.records[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = "{base=essential
entry=E0026170
\tcat=noun
\tvariants=reg
}
{base=essential
entry=E0026171
\tcat=adj
\tvariants=inv;periph
\tposition=pred
\tcompl=pphr(to,np)
\tcompl=pphr(for,np)
\tcompl=fincomp(ts):subj
\tstative
\tnominalization=essentiality|noun|E0220318
\tnominalization=essentialness|noun|E0026173
}
{base=dependant
spelling_variant=dependent
entry=E0021391
\tcat=adj
\tvariants=inv;periph
\tcompl=pphr(on,np)
\tcompl=pphr(upon,np)
\tcompl=pphr(on,ingcomp:subjc)
\tnominalization=dependence|noun|E0021383
}
";

    #[test]
    fn a_record_is_found_by_base_or_spelling_variant_and_category() {
        let lex = Lexicon::parse(FIXTURE).unwrap();
        assert_eq!(lex.len(), 3);
        let adj: Vec<&Record> = lex.lookup("Essential", "adj").collect();
        assert_eq!(adj.len(), 1);
        assert_eq!(adj[0].entry, "E0026171");
        assert_eq!(lex.lookup("essential", "noun").count(), 1);
        assert_eq!(lex.lookup("essential", "verb").count(), 0);
        let dep: Vec<&Record> = lex.lookup("dependent", "adj").collect();
        assert_eq!(dep.len(), 1);
        assert_eq!(dep[0].base, "dependant");
    }

    #[test]
    fn a_complement_names_its_preposition() {
        let lex = Lexicon::parse(FIXTURE).unwrap();
        let essential = lex.lookup("essential", "adj").next().unwrap();
        assert_eq!(
            essential.pp_prepositions("compl"),
            ["for", "to"].map(String::from).into()
        );
        let dependent = lex.lookup("dependent", "adj").next().unwrap();
        assert_eq!(
            dependent.pp_prepositions("compl"),
            ["on", "upon"].map(String::from).into()
        );
        assert_eq!(
            pphr_preposition("pphr(in terms of,np)").as_deref(),
            Some("in terms of")
        );
        assert_eq!(
            pphr_preposition("pphr(to,np|gestational age|)").as_deref(),
            Some("to")
        );
        assert_eq!(pphr_preposition("infcomp:subjc"), None);
        assert!(essential.values("stative").eq([""]));
    }

    #[test]
    fn a_record_names_its_nominalizations() {
        let lex = Lexicon::parse(FIXTURE).unwrap();
        let noms = lex
            .lookup("essential", "adj")
            .next()
            .unwrap()
            .nominalizations();
        assert_eq!(
            noms.iter().map(|n| n.lemma.as_str()).collect::<Vec<_>>(),
            ["essentiality", "essentialness"]
        );
        assert_eq!(noms[0].cat, "noun");
        assert_eq!(noms[0].entry, "E0220318");
    }

    #[test]
    fn a_malformed_line_is_an_error() {
        let err = Lexicon::parse("{base=x\nentry=E1\ncat=noun\n}\n").unwrap_err();
        assert_eq!(err.line, 3);
        assert!(Lexicon::parse("{base=x\nentry=E1\n\tcat=noun\n").is_err());
        assert!(Lexicon::parse("{base=x\n\tcat=noun\n}\n").is_err());
    }

    /// The provisioned release parses whole (`scripts/provision-specialist.sh`); skipped where it is
    /// not provisioned, as the file is gitignored.
    #[test]
    fn the_provisioned_lexicon_parses() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../references/specialist/LEXICON");
        if !path.exists() {
            eprintln!("SKIP: {} not provisioned", path.display());
            return;
        }
        let lex = Lexicon::read(&path).unwrap();
        assert!(lex.len() > 500_000, "{} records", lex.len());
        let essential = lex.lookup("essential", "adj").next().unwrap();
        assert!(essential.pp_prepositions("compl").contains("for"));
    }
}
