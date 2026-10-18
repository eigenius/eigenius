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

//! Registry records → one ESL layer, and the findings a curator needs: what was held back, what
//! was converted with a note, what was not converted.
//!
//! Names:
//! - a case is `pmi:<Case ID>` and its subject `pmi:<Case ID>_proband` — keyed on Case ID, which
//!   is not unique in the live base; the synthetic set has no Airtable record ids (D99 §7);
//! - an allele is `allele:<accession>_<interval>_<alt>`, minted from its identity fields (D99 §4),
//!   so one allele in two cases is one resource;
//! - a reference is `seqref:<accession>`; a gene is `gene:<symbol>`, minted, not resolved to
//!   `ncbi:Gene` (D99 §9).
//!
//! Every minted resource except the registry and the source stubs carries a
//! `prov:DeclarationTrace` by the registry, timestamped with the case's creation date.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write as _};

use crate::hgvs::{self, Change};
use crate::hpo_history::{HpoHistory, Verdict};
use crate::record::{Case, GeneRow, Package, UNCONVERTED_FIELDS};

const PREAMBLE: &str = r#"namespace core          = "urn:eigenius:core";
namespace lexicon       = "urn:eigenius:lexicon";
namespace logic         = "urn:eigenius:logic";
namespace eigentt       = "urn:eigenius:eigentt";
namespace justification = "urn:eigenius:justification";
namespace prov          = "urn:eigenius:prov";
namespace enc           = "urn:eigenius:encoding";
namespace ontology      = "urn:eigenius:ontology";
namespace variant       = "urn:eigenius:variant";
namespace clinical      = "urn:eigenius:clinical";
namespace hp            = "urn:obo:HP";
namespace wn            = "urn:eigenius:wn";
namespace pmi           = "urn:eigenius:pmi";
namespace gene          = "urn:eigenius:pmi:gene";
namespace allele        = "urn:eigenius:variant:allele";
namespace seqref        = "urn:eigenius:variant:reference";
"#;

/// WordNet's *have*, "suffer from; be ill with" — the phenotype predicate (D99 §6).
const HAVE_ILLNESS: &str = "wn:v00065370_t";

/// The registry, as the agent of every declaration.
const REGISTRY: &str = "pmi:registry";

/// What happened to an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Disposition {
    /// Not converted until someone declares which reading is right.
    Held,
    /// Converted, with something a curator should know.
    Noted,
    /// Not converted, by rule.
    Skipped,
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub disposition: Disposition,
    pub case: String,
    pub item: String,
    pub why: String,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Counts {
    pub cases: usize,
    pub resources: usize,
    pub declarations: usize,
}

/// One run's output.
#[derive(Debug)]
pub struct Conversion {
    pub esl: String,
    pub findings: Vec<Finding>,
    pub counts: Counts,
}

/// A qualified ESL name; the local part is quoted when it is not a bare identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Name {
    prefix: &'static str,
    local: String,
}

impl Name {
    /// `local` reduced to `[A-Za-z0-9_]`: every other character becomes `_`.
    fn new(prefix: &'static str, local: &str) -> Self {
        let local = local
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        Name { prefix, local }
    }

    fn suffixed(&self, suffix: &str) -> Self {
        Name::new(self.prefix, &format!("{}_{suffix}", self.local))
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.local.starts_with(|c: char| c.is_ascii_digit()) {
            write!(f, "{}:'{}'", self.prefix, self.local)
        } else {
            write!(f, "{}:{}", self.prefix, self.local)
        }
    }
}

fn string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `HP:0000103` → `hp:'0000103'`.
fn hp_class(id: &str) -> Option<String> {
    let local = id.strip_prefix("HP:")?;
    (!local.is_empty() && local.chars().all(|c| c.is_ascii_digit()))
        .then(|| format!("hp:'{local}'"))
}

/// The Zygosity field's three axes (D99 6a).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Zygosity {
    State(&'static str),
    Compound,
    Mosaic,
}

fn zygosity(value: &str) -> Option<Zygosity> {
    Some(match value.trim() {
        "Heterozygous" => Zygosity::State("clinical:heterozygous"),
        "Homozygous" => Zygosity::State("clinical:homozygous"),
        "Hemizygous" => Zygosity::State("clinical:hemizygous"),
        "Compound Heterozygous" => Zygosity::Compound,
        "Mosaic" => Zygosity::Mosaic,
        _ => return None,
    })
}

fn acmg(value: &str) -> Option<&'static str> {
    Some(match value.trim() {
        "Benign" => "clinical:benign",
        "Likely Benign" => "clinical:likely_benign",
        "VUS" => "clinical:vus",
        "Likely Pathogenic" => "clinical:likely_pathogenic",
        "Pathogenic" => "clinical:pathogenic",
        _ => return None,
    })
}

/// `Some(None)` is `Unknown`, which writes nothing (D99 6c). The schema's option text and the
/// export's differ after the first word (`Amorphic (complete LoF)`, `Amorphic (Complete Loss of
/// Function)`), so the first word decides.
fn impact(value: &str) -> Option<Option<&'static str>> {
    let word = value.split_whitespace().next()?.to_ascii_lowercase();
    Some(match word.as_str() {
        "amorphic" => Some("variant:amorphic"),
        "hypomorphic" => Some("variant:hypomorphic"),
        "hypermorphic" => Some("variant:hypermorphic"),
        "antimorphic" => Some("variant:antimorphic"),
        "unknown" => None,
        _ => return None,
    })
}

/// A span of releases: `in 2018-03-08`, or `2021-06-08 … 2026-02-16`.
fn releases((first, last): &(String, String)) -> String {
    if first == last {
        format!("in {first}")
    } else {
        format!("{first} … {last}")
    }
}

fn nonempty(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// What a case's declarations share.
struct CaseCtx {
    case_id: String,
    case: Name,
    subject: Name,
    source: Option<Name>,
    created: String,
}

/// An allele, its name and its identity fields.
struct AlleleSpec {
    name: Name,
    class: &'static str,
    reference: String,
    change: Change,
    hgvs: String,
}

impl AlleleSpec {
    fn new(class: &'static str, reference: &str, change: Change, hgvs: String) -> Self {
        let interval = if change.start == change.end {
            change.start.to_string()
        } else {
            format!("{}_{}", change.start, change.end)
        };
        let alt = if change.alt.is_empty() {
            "del"
        } else {
            change.alt.as_str()
        };
        AlleleSpec {
            name: Name::new("allele", &format!("{reference}_{interval}_{alt}")),
            class,
            reference: reference.to_string(),
            change,
            hgvs,
        }
    }
}

struct Emitter<'a> {
    hpo: &'a HpoHistory,
    out: String,
    minted: BTreeSet<Name>,
    findings: Vec<Finding>,
    counts: Counts,
}

impl<'a> Emitter<'a> {
    fn finding(&mut self, disposition: Disposition, case: &str, item: String, why: String) {
        self.findings.push(Finding {
            disposition,
            case: case.to_string(),
            item,
            why,
        });
    }

    fn block(&mut self, name: &Name, class: &str, fields: &[(&str, String)]) {
        writeln!(self.out, "resource {name} : {class} {{").unwrap();
        for (key, value) in fields {
            writeln!(self.out, "    {key} = {value};").unwrap();
        }
        self.out.push_str("}\n\n");
    }

    /// A resource and the registry's trace of it. A name already minted is not emitted again.
    fn resource(&mut self, ctx: &CaseCtx, name: &Name, class: &str, fields: &[(&str, String)]) {
        if !self.minted.insert(name.clone()) {
            return;
        }
        self.block(name, class, fields);
        self.trace(ctx, name, true);
        self.counts.resources += 1;
    }

    fn trace(&mut self, ctx: &CaseCtx, name: &Name, sourced: bool) {
        let mut fields = vec![
            ("prov:resource", name.to_string()),
            ("prov:was_attributed_to", REGISTRY.to_string()),
        ];
        if let (true, Some(source)) = (sourced, &ctx.source) {
            fields.push(("prov:had_primary_source", source.to_string()));
        }
        fields.push(("prov:timestamp", string(&ctx.created)));
        self.block(&name.suffixed("trace"), "prov:DeclarationTrace", &fields);
    }

    /// A registry Declaration of `proposition`, sourced to the case's records unless `sourced` is
    /// false.
    fn declaration(
        &mut self,
        ctx: &CaseCtx,
        name: Name,
        description: &str,
        proposition: &str,
        sourced: bool,
    ) {
        let mut fields = vec![
            ("core:description", string(description)),
            ("prov:was_attributed_to", REGISTRY.to_string()),
        ];
        if let (true, Some(source)) = (sourced, &ctx.source) {
            fields.push(("prov:had_primary_source", source.to_string()));
        }
        fields.push(("eigentt:proposition", format!("type_expr({proposition})")));
        self.block(&name, "justification:Declaration", &fields);
        self.trace(ctx, &name, sourced);
        self.counts.declarations += 1;
    }

    fn prose(&mut self, ctx: &CaseCtx, local: &str, section: &str, text: Option<&str>) {
        let Some(text) = text else { return };
        let name = ctx.case.suffixed(local);
        self.resource(
            ctx,
            &name,
            "enc:DiscourseUnit",
            &[
                ("enc:prose", string(text)),
                ("enc:unit_kind", "enc:kind_prose".to_string()),
                ("enc:section", string(&format!("{} {section}", ctx.case_id))),
            ],
        );
    }

    fn source_stub(&mut self, name: &Name, description: String) {
        if self.minted.insert(name.clone()) {
            self.block(
                name,
                "prov:Source",
                &[("core:description", string(&description))],
            );
        }
    }

    fn case(&mut self, case: &Case) {
        let id = case.case_id.trim().to_string();
        if !case.study_consent {
            self.finding(
                Disposition::Skipped,
                &id,
                "the case".into(),
                "no study consent (D99 6d)".into(),
            );
            return;
        }
        let Some(created) = nonempty(&case.created) else {
            self.finding(
                Disposition::Held,
                &id,
                "the case".into(),
                "no Case Created Timestamp: a declaration's trace needs its date".into(),
            );
            return;
        };
        let sex = match case.gender.as_slice() {
            [] => None,
            [one] if one == "N/A" => None,
            [one] if one == "Male" => Some("clinical:male"),
            [one] if one == "Female" => Some("clinical:female"),
            other => {
                self.finding(
                    Disposition::Held,
                    &id,
                    "the case".into(),
                    format!(
                        "Gender {other:?} names several subjects; the registry records neither how many nor who (D99 §7)"
                    ),
                );
                return;
            }
        };
        let case_name = Name::new("pmi", &id);
        let ctx = CaseCtx {
            subject: case_name.suffixed("proband"),
            source: nonempty(&case.medical_records).map(|_| case_name.suffixed("medical_records")),
            case: case_name,
            case_id: id.clone(),
            created: created.to_string(),
        };
        self.counts.cases += 1;
        writeln!(
            self.out,
            "// ── {id} ──────────────────────────────────────────\n"
        )
        .unwrap();

        if let Some(source) = &ctx.source {
            self.source_stub(
                source,
                format!("{id}'s medical records: clinic notes and lab reports (tier 3, UAB ShareFile). Not held; the registry's Medical Records field links them."),
            );
        } else {
            self.finding(
                Disposition::Noted,
                &id,
                "Medical Records".into(),
                "empty: the case's declarations name no primary source".into(),
            );
        }
        if nonempty(&case.case_notes).is_some() {
            self.source_stub(
                &ctx.case.suffixed("case_notes"),
                format!("{id}'s analyst notes (tier 2, Google Drive). Not held; the registry's Case Notes field links them."),
            );
        }

        let mut subject = vec![(
            "core:description",
            string(&format!("The individual {id} is about.")),
        )];
        if let Some(year) = case.year_of_birth {
            subject.push(("clinical:birth_year", year.to_string()));
        }
        if let Some(sex) = sex {
            subject.push(("clinical:sex", sex.to_string()));
        }
        self.resource(&ctx, &ctx.subject, "clinical:Individual", &subject);
        self.resource(
            &ctx,
            &ctx.case,
            "clinical:Case",
            &[
                ("clinical:case_id", string(&id)),
                ("clinical:subject", format!("[{}]", ctx.subject)),
            ],
        );

        self.prose(&ctx, "diagnosis", "Diagnosis", nonempty(&case.diagnosis));
        self.prose(&ctx, "symptoms", "Symptoms", nonempty(&case.symptoms));
        self.prose(
            &ctx,
            "case_history",
            "Case History",
            nonempty(&case.case_history),
        );
        self.prose(
            &ctx,
            "family_goal",
            "Participant/Family Goal",
            nonempty(&case.family_goal),
        );
        self.prose(
            &ctx,
            "medications",
            "Medications",
            nonempty(&case.medications),
        );
        self.prose(
            &ctx,
            "next_steps",
            "Case Review Next Steps",
            nonempty(&case.next_steps),
        );
        self.prose(
            &ctx,
            "outcome_details",
            "Outcome (Details)",
            nonempty(&case.outcome_details),
        );

        for term in &case.linked_records.hp_terms {
            self.phenotype(&ctx, &term.hp_id, &term.phenotype);
        }

        let rows = &case.linked_records.gene_info;
        if !rows.is_empty() && !case.genomic_consent {
            self.finding(
                Disposition::Skipped,
                &id,
                format!("{} Gene Info row(s)", rows.len()),
                "no genomic-data-sharing consent (D99 6d)".into(),
            );
            return;
        }
        self.gene_rows(&ctx, rows);
    }

    fn phenotype(&mut self, ctx: &CaseCtx, hp_id: &str, label: &str) {
        let id = hp_id.trim();
        let item = format!("{id} \"{label}\"");
        let Some(class) = hp_class(id) else {
            self.finding(Disposition::Held, &ctx.case_id, item, "not an HP ID".into());
            return;
        };
        let verdict = self.hpo.check(id, label);
        let version = self.hpo.version.clone();
        let earlier = self.hpo.releases().len();
        match &verdict {
            Verdict::Label => {}
            Verdict::Synonym { scope, former } => {
                let mut why = format!("a synonym ({scope}) of the term in {version}");
                if let Some(span) = former {
                    write!(why, "; its label {}", releases(span)).unwrap();
                }
                self.finding(Disposition::Noted, &ctx.case_id, item.clone(), why);
            }
            Verdict::FormerLabel { first, last } => {
                let current = self
                    .hpo
                    .term(id)
                    .map(|t| t.label.clone())
                    .unwrap_or_default();
                self.finding(
                    Disposition::Noted,
                    &ctx.case_id,
                    item.clone(),
                    format!(
                        "stale: the term's label {}; {version} labels it \"{current}\" and keeps no synonym for it",
                        releases(&(first.clone(), last.clone()))
                    ),
                );
            }
            Verdict::NotATerm => {
                self.finding(
                    Disposition::Held,
                    &ctx.case_id,
                    item,
                    format!("not a live term of {version}"),
                );
                return;
            }
            Verdict::Mismatch { current, named } => {
                let mut why = format!(
                    "{id} is \"{current}\" in {version}, and \"{label}\" is not its label in any of {earlier} earlier releases"
                );
                if !named.is_empty() {
                    write!(why, "; \"{label}\" names {}", named.join(", ")).unwrap();
                }
                self.finding(Disposition::Held, &ctx.case_id, item, why);
                return;
            }
        }
        debug_assert!(verdict.passes());
        self.declaration(
            ctx,
            ctx.case.suffixed(&format!("hp_{}", &id[3..])),
            &format!(
                "{}'s subject has {label} ({id}), as the registry codes it.",
                ctx.case_id
            ),
            &format!("{HAVE_ILLNESS}(ontology:kind_of({class}), {})", ctx.subject),
            true,
        );
    }

    fn gene_rows(&mut self, ctx: &CaseCtx, rows: &[GeneRow]) {
        // Gene symbol → the nucleotide alleles of its Compound Heterozygous rows (None: protein only).
        let mut compound: BTreeMap<String, Vec<Option<Name>>> = BTreeMap::new();
        for (i, row) in rows.iter().enumerate() {
            let n = i + 1;
            let change = nonempty(&row.genotype)
                .or(nonempty(&row.protein_change))
                .unwrap_or("no change");
            let item = format!("Gene Info {n} ({} {change})", row.gene_name.trim());
            match self.gene_row(ctx, n, row, &item) {
                Ok((Zygosity::Compound, allele)) => compound
                    .entry(row.gene_name.trim().to_string())
                    .or_default()
                    .push(allele),
                Ok(_) => {}
                Err(why) => self.finding(Disposition::Held, &ctx.case_id, item, why),
            }
        }
        for (gene, alleles) in compound {
            match alleles.as_slice() {
                [Some(a1), Some(a2)] => self.declaration(
                    ctx,
                    ctx.case.suffixed(&format!("in_trans_{}", Name::new("x", &gene).local)),
                    &format!("{}'s subject's two {gene} alleles are on different copies (Compound Heterozygous).", ctx.case_id),
                    &format!("clinical:InTrans({a1}, {a2})"),
                    true,
                ),
                _ => self.finding(
                    Disposition::Held,
                    &ctx.case_id,
                    format!("{gene} Compound Heterozygous"),
                    format!(
                        "InTrans needs exactly two rows with a nucleotide allele; {} row(s), {} with one",
                        alleles.len(),
                        alleles.iter().flatten().count()
                    ),
                ),
            }
        }
    }

    /// One Gene Info row. Every check runs before anything is emitted, so a held-back row writes
    /// nothing.
    fn gene_row(
        &mut self,
        ctx: &CaseCtx,
        n: usize,
        row: &GeneRow,
        item: &str,
    ) -> Result<(Zygosity, Option<Name>), String> {
        let symbol = row.gene_name.trim();
        let zyg_text = nonempty(&row.zygosity).ok_or("no Zygosity")?;
        let zyg = zygosity(zyg_text)
            .ok_or_else(|| format!("Zygosity `{zyg_text}` is not a registry option"))?;
        let refseq = nonempty(&row.refseq);
        let genotype = nonempty(&row.genotype);

        let nucleotide = match (refseq, genotype, nonempty(&row.hgvs)) {
            (Some(r), Some(g), hgvs) => {
                let expected = format!("{r}:{g}");
                if let Some(h) = hgvs.filter(|h| *h != expected) {
                    return Err(format!(
                        "HGVS Notation `{h}` is not RefSeq:Genotype `{expected}` (D99 §7)"
                    ));
                }
                Some(AlleleSpec::new(
                    "variant:NucleotideAllele",
                    r,
                    hgvs::parse_c(g)?,
                    expected,
                ))
            }
            (_, None, Some(h)) => return Err(format!("HGVS Notation `{h}` with no Genotype")),
            (None, Some(g), _) => {
                return Err(format!(
                    "Genotype `{g}` with no RefSeq: the change has no reference"
                ))
            }
            (_, None, None) => None,
        };
        let protein = match nonempty(&row.protein_change) {
            Some(p) => {
                let hgnc = nonempty(&row.hgnc_id)
                    .ok_or("a protein change with no HGNC ID: its reference is minted from the HGNC id (D99 §4)")?;
                Some((
                    format!("{hgnc}:p"),
                    AlleleSpec::new(
                        "variant:ProteinAllele",
                        &format!("{hgnc}:p"),
                        hgvs::parse_p(p)?,
                        p.to_string(),
                    ),
                ))
            }
            None => None,
        };
        if nucleotide.is_none() && protein.is_none() {
            return Err("neither a nucleotide nor a protein change".into());
        }
        let tier = match nonempty(&row.acmg) {
            Some(v) => Some(
                acmg(v)
                    .ok_or_else(|| format!("ACMG Classification `{v}` is not a registry option"))?,
            ),
            None => None,
        };
        let morph = match nonempty(&row.impact) {
            Some(v) => {
                impact(v).ok_or_else(|| format!("Variant Impact `{v}` is not a registry option"))?
            }
            None => None,
        };
        if zyg == Zygosity::Compound && nucleotide.is_none() {
            return Err("Compound Heterozygous on a protein-only row: no nucleotide allele to place in trans".into());
        }

        // Emit: gene, references, alleles.
        let gene = Name::new("gene", symbol);
        let mut gene_desc = format!("The gene the registry names {symbol}");
        if let Some(h) = nonempty(&row.hgnc_id) {
            write!(gene_desc, " ({h})").unwrap();
        }
        gene_desc
            .push_str(". Minted from the registry's symbol, not resolved to ncbi:Gene (D99 §9).");
        self.resource(
            ctx,
            &gene,
            "lexicon:Entity",
            &[("core:description", string(&gene_desc))],
        );
        if let Some(a) = &nucleotide {
            self.reference(ctx, "variant:Transcript", &a.reference, &gene, symbol);
            self.allele(ctx, a);
        }
        if let Some((accession, p)) = &protein {
            self.reference(ctx, "variant:ProteinProduct", accession, &gene, symbol);
            self.allele(ctx, p);
        }
        let subject = &ctx.subject;
        let case_id = &ctx.case_id;

        // What the subject carries, at the resolution the row has.
        let state = match zyg {
            Zygosity::State(s) => Some(s),
            Zygosity::Compound => Some("clinical:heterozygous"),
            Zygosity::Mosaic => None,
        };
        let state_word = zyg_text.to_ascii_lowercase();
        let carried = |a: &str| -> String {
            let mut facts = vec![match state {
                Some(s) => format!("clinical:Carries({subject}, {a}, {s})"),
                None => format!("clinical:Mosaic({subject}, {a})"),
            }];
            if row.de_novo {
                facts.push(format!("clinical:DeNovo({subject}, {a})"));
            }
            facts
                .into_iter()
                .reduce(|p, q| format!("logic:And({p}, {q})"))
                .unwrap()
        };
        let de_novo = if row.de_novo { ", de novo" } else { "" };
        let classified = match (&nucleotide, &protein) {
            (Some(a), _) => {
                self.declaration(
                    ctx,
                    ctx.case.suffixed(&format!("carries_{}", a.name.local)),
                    &format!(
                        "{case_id}'s subject carries {}, {state_word}{de_novo}.",
                        a.hgvs
                    ),
                    &carried(&a.name.to_string()),
                    true,
                );
                if let Some((_, p)) = &protein {
                    self.declaration(
                        ctx,
                        ctx.case.suffixed(&format!("translates_{}", a.name.local)),
                        &format!(
                            "{} translates to {}, as {case_id}'s row records it.",
                            a.hgvs, p.hgvs
                        ),
                        &format!("variant:TranslatesTo({}, {})", a.name, p.name),
                        true,
                    );
                }
                a.name.clone()
            }
            (None, Some((_, p))) => {
                // D99 §2: some nucleotide allele that translates to the protein change.
                let existential = |restriction: String| {
                    format!(
                        "forall (R : Prop) => (forall (a : exists a : variant:NucleotideAllele => {restriction}) => {} -> R) -> R",
                        carried("eigentt:fst(a)")
                    )
                };
                let translates = format!("variant:TranslatesTo(a, {})", p.name);
                self.declaration(
                    ctx,
                    ctx.case.suffixed(&format!("carries_{}", p.name.local)),
                    &format!("{case_id}'s subject carries a {symbol} variant changing the protein by {}, {state_word}{de_novo}; the nucleotide change is not recorded.", p.hgvs),
                    &existential(translates.clone()),
                    true,
                );
                self.finding(
                    Disposition::Noted,
                    case_id,
                    item.to_string(),
                    "protein change only: carried as an existential over the nucleotide alleles that translate to it (D99 §2)".into(),
                );
                if let Some(r) = refseq {
                    // D99 §7: a RefSeq with no change on it has no recorded source and builds no allele.
                    self.declaration(
                        ctx,
                        ctx.case.suffixed(&format!("refseq_{}", p.name.local)),
                        &format!("The {symbol} variant of {case_id}'s row is on {r}, as the row's RefSeq records it; no source states it."),
                        &existential(format!(
                            "logic:And({translates}, eigentt:Eq(core:string, eigentt:field(a, variant:reference), {}))",
                            string(r)
                        )),
                        false,
                    );
                    self.finding(
                        Disposition::Noted,
                        case_id,
                        format!("Gene Info {n} RefSeq {r}"),
                        "a RefSeq with no nucleotide change: declared without a primary source; builds no allele (D99 §7)".into(),
                    );
                }
                p.name.clone()
            }
            (None, None) => unreachable!("checked above"),
        };
        if let Some(tier) = tier {
            self.declaration(
                ctx,
                ctx.case
                    .suffixed(&format!("classified_{}", classified.local)),
                &format!(
                    "{classified} is classified {}, as {case_id}'s row records it.",
                    row.acmg.as_deref().unwrap_or_default().trim()
                ),
                &format!("clinical:ClassifiedAs({classified}, {tier})"),
                true,
            );
        }
        if let Some(morph) = morph {
            self.declaration(
                ctx,
                ctx.case.suffixed(&format!("impact_{}", classified.local)),
                &format!(
                    "{classified}'s effect on function is {}, as {case_id}'s row records it.",
                    &morph["variant:".len()..]
                ),
                &format!("variant:Impact({classified}, {morph})"),
                true,
            );
        }
        self.prose(
            ctx,
            &format!("gene_{n}_notes"),
            &format!("Gene Info {n} Notes"),
            nonempty(&row.notes),
        );
        Ok((zyg, nucleotide.map(|a| a.name)))
    }

    fn reference(
        &mut self,
        ctx: &CaseCtx,
        class: &str,
        accession: &str,
        gene: &Name,
        symbol: &str,
    ) {
        let kind = if class == "variant:Transcript" {
            "a transcript"
        } else {
            "the protein product, isoform unspecified,"
        };
        self.resource(
            ctx,
            &Name::new("seqref", accession),
            class,
            &[
                (
                    "core:description",
                    string(&format!("{accession}: {kind} of {symbol}.")),
                ),
                ("variant:accession", string(accession)),
                ("variant:gene", gene.to_string()),
            ],
        );
    }

    fn allele(&mut self, ctx: &CaseCtx, a: &AlleleSpec) {
        let mut fields = vec![
            ("variant:reference", string(&a.reference)),
            ("variant:start", a.change.start.to_string()),
            ("variant:end", a.change.end.to_string()),
        ];
        if let Some(r) = &a.change.ref_state {
            fields.push(("variant:ref_state", string(r)));
        }
        fields.push(("variant:alt", string(&a.change.alt)));
        fields.push(("variant:hgvs", string(&a.hgvs)));
        self.resource(ctx, &a.name, a.class, &fields);
    }
}

/// Convert a registry package. `input` names it in the layer's header.
pub fn convert(package: &Package, hpo: &HpoHistory, input: &str) -> Conversion {
    let mut e = Emitter {
        hpo,
        out: String::new(),
        minted: BTreeSet::new(),
        findings: Vec::new(),
        counts: Counts::default(),
    };
    writeln!(
        e.out,
        "// PMI Case Registry records as registry declarations (D99 step 4).\n// Generated by `pmi-registry-convert` from {input}; HPO {} with {} earlier releases. Do not edit.\n",
        hpo.version,
        hpo.releases().len()
    )
    .unwrap();
    e.out.push_str(PREAMBLE);
    e.out.push('\n');
    e.block(
        &Name::new("pmi", "registry"),
        "prov:Organization",
        &[
            ("core:description", string("The PMI Case Registry (UAB Precision Medicine Institute): the party that transcribed each registry value from a case's records.")),
            ("core:short_name", string("pmi_registry")),
        ],
    );
    for case in &package.cases {
        e.case(case);
    }
    for (field, why) in UNCONVERTED_FIELDS {
        e.finding(
            Disposition::Skipped,
            "all",
            (*field).to_string(),
            (*why).to_string(),
        );
    }
    Conversion {
        esl: e.out,
        findings: e.findings,
        counts: e.counts,
    }
}

/// The findings as a markdown report.
pub fn report(conversion: &Conversion, input: &str, hpo: &HpoHistory) -> String {
    let mut out = String::new();
    let c = conversion.counts;
    let span = match (hpo.releases().first(), hpo.releases().last()) {
        (Some(a), Some(b)) => format!(", {} … {}", a.tag, b.tag),
        _ => String::new(),
    };
    writeln!(out, "# PMI registry conversion report\n").unwrap();
    writeln!(out, "- Input: `{input}`").unwrap();
    writeln!(
        out,
        "- HPO: loaded release {}; {} earlier releases{span}",
        hpo.version,
        hpo.releases().len()
    )
    .unwrap();
    writeln!(
        out,
        "- Converted: {} case(s), {} resource(s), {} declaration(s)\n",
        c.cases, c.resources, c.declarations
    )
    .unwrap();
    for (disposition, title) in [
        (Disposition::Held, "Held back"),
        (Disposition::Noted, "Converted with a finding"),
        (Disposition::Skipped, "Not converted"),
    ] {
        let rows: Vec<&Finding> = conversion
            .findings
            .iter()
            .filter(|f| f.disposition == disposition)
            .collect();
        writeln!(out, "## {title}\n").unwrap();
        if rows.is_empty() {
            writeln!(out, "None.\n").unwrap();
            continue;
        }
        writeln!(out, "| Case | Item | Finding |\n|---|---|---|").unwrap();
        for f in rows {
            writeln!(
                out,
                "| {} | {} | {} |",
                f.case,
                f.item.replace('|', "\\|"),
                f.why.replace('|', "\\|")
            )
            .unwrap();
        }
        out.push('\n');
    }
    out
}
