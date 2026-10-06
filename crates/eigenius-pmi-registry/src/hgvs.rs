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

//! The HGVS forms a registry row writes, read into an allele's identity fields (D99 §4): an interval
//! (1-based, inclusive, as HGVS counts), the reference state where the notation states it, and the
//! alternate state.
//!
//! Only the forms the registry's rows use are read: a coding substitution (`c.131T>C`), a coding
//! deletion (`c.1105del`, `c.1105_1107del`), a protein missense (`p.Leu44Pro`) and a protein
//! frameshift (`p.Ser369Leufs*12`). Anything else is refused with the reason, and the row is held
//! back — an allele built from a misread notation would be a different allele.

/// An allele's identity fields, read from one HGVS change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub start: u32,
    pub end: u32,
    /// The reference state the notation states (`T`, `Leu`); `None` where it states none (`del`).
    pub ref_state: Option<String>,
    /// The alternate state: a nucleotide, a residue, a frameshift (`Leufs*12`), or `""` for a deletion.
    pub alt: String,
}

fn position(s: &str) -> Result<u32, String> {
    s.parse::<u32>()
        .map_err(|_| format!("`{s}` is not a sequence position"))
}

/// A coding change, with or without its `c.` prefix.
pub fn parse_c(notation: &str) -> Result<Change, String> {
    let s = notation.trim();
    let body = s
        .strip_prefix("c.")
        .ok_or_else(|| format!("`{s}` is not a coding (c.) change"))?;
    if let Some(interval) = body.strip_suffix("del") {
        let (start, end) = match interval.split_once('_') {
            Some((a, b)) => (position(a)?, position(b)?),
            None => (position(interval)?, position(interval)?),
        };
        if end < start {
            return Err(format!("`{s}`: the interval runs backwards"));
        }
        return Ok(Change {
            start,
            end,
            ref_state: None,
            alt: String::new(),
        });
    }
    let (left, alt) = body
        .split_once('>')
        .ok_or_else(|| format!("`{s}` is neither a substitution nor a deletion"))?;
    let split = left
        .find(|c: char| !c.is_ascii_digit())
        .ok_or_else(|| format!("`{s}`: no reference base"))?;
    let (pos, ref_base) = left.split_at(split);
    let base = |b: &str| b.len() == 1 && "ACGT".contains(b);
    if !base(ref_base) || !base(alt) {
        return Err(format!("`{s}`: a substitution is one base for one base"));
    }
    let p = position(pos)?;
    Ok(Change {
        start: p,
        end: p,
        ref_state: Some(ref_base.to_string()),
        alt: alt.to_string(),
    })
}

/// A protein change, with or without its `p.` prefix, parenthesised or not.
pub fn parse_p(notation: &str) -> Result<Change, String> {
    let s = notation.trim();
    let body = s
        .strip_prefix("p.")
        .ok_or_else(|| format!("`{s}` is not a protein (p.) change"))?;
    let body = body
        .strip_prefix('(')
        .and_then(|b| b.strip_suffix(')'))
        .unwrap_or(body);
    let residue = |r: &str| {
        r.len() == 3
            && r.starts_with(|c: char| c.is_ascii_uppercase())
            && r[1..].chars().all(|c| c.is_ascii_lowercase())
    };
    if body.len() < 5 || !residue(&body[..3]) {
        return Err(format!(
            "`{s}`: expected a three-letter residue, a position and the change"
        ));
    }
    let (ref_res, rest) = body.split_at(3);
    let split = rest
        .find(|c: char| !c.is_ascii_digit())
        .ok_or_else(|| format!("`{s}`: no change after the position"))?;
    let (pos, alt) = rest.split_at(split);
    let missense = residue(alt);
    let frameshift = alt.len() > 3 && residue(&alt[..3]) && alt[3..].starts_with("fs");
    if !missense && !frameshift {
        return Err(format!(
            "`{s}`: `{alt}` is neither a residue nor a frameshift"
        ));
    }
    let p = position(pos)?;
    Ok(Change {
        start: p,
        end: p,
        ref_state: Some(ref_res.to_string()),
        alt: alt.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_coding_substitution() {
        assert_eq!(
            parse_c("c.131T>C").unwrap(),
            Change {
                start: 131,
                end: 131,
                ref_state: Some("T".into()),
                alt: "C".into()
            }
        );
    }

    #[test]
    fn a_coding_deletion_one_base_or_an_interval() {
        let one = parse_c("c.1105del").unwrap();
        assert_eq!((one.start, one.end, one.alt.as_str()), (1105, 1105, ""));
        let span = parse_c("c.1105_1107del").unwrap();
        assert_eq!((span.start, span.end), (1105, 1107));
    }

    #[test]
    fn a_protein_missense_and_a_frameshift() {
        let m = parse_p("p.Leu44Pro").unwrap();
        assert_eq!(
            (m.start, m.ref_state.as_deref(), m.alt.as_str()),
            (44, Some("Leu"), "Pro")
        );
        let f = parse_p("p.Ser369Leufs*12").unwrap();
        assert_eq!(
            (f.start, f.ref_state.as_deref(), f.alt.as_str()),
            (369, Some("Ser"), "Leufs*12")
        );
        assert_eq!(parse_p("p.(Leu44Pro)").unwrap(), m);
    }

    #[test]
    fn what_the_registry_does_not_write_is_refused() {
        for bad in [
            "c.131T>CG",
            "c.10_11insA",
            "131T>C",
            "p.L44P",
            "p.Leu44",
            "c.20_10del",
        ] {
            assert!(
                parse_c(bad).is_err() && parse_p(bad).is_err(),
                "{bad} must be refused"
            );
        }
    }
}
