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

//! Every `.esl` in the tree parses, and names no namespace prefix it does not declare.
//!
//! **Why this is not "every `.esl` compiles".** Most of them cannot compile against the bootstrap
//! chain alone: a statistics fixture needs its companion fixtures, a parsing probe needs a lexicon
//! layer, a publication chain file needs the files before it. Asserting a clean compile would
//! either fail on all of those or need a per-file chain, which is the thing the suites that own
//! those files already do. So this asserts the two properties that are file-local and that no
//! other test covers for the files it does not own:
//!
//! 1. the file PARSES; and
//! 2. every `prefix:` it uses in code position is declared by a `namespace` line in that file.
//!
//! **What it is for.** A rename that moves a name across namespaces has a second-order effect a
//! residue grep cannot see: it introduces a prefix into files that never declared one. That
//! happened three times during D89 — `justification:` into 12 places, then `eigentt:` into six
//! more, then `institution:` — and each time the failure surfaced as a compile error in whichever
//! suite happened to own the file. Nine of the 62 `.esl` files in the tree are named by no source
//! file or script at all — `experiments/parsing/probes/*.esl`, inputs to a harness that reads the
//! directory — so nothing else would report a break in them. That is a lower bound on the
//! unchecked set: being named by a script is not the same as being compiled by a test.
//!
//! It is also what makes removing an unused `namespace` alias safe to do mechanically: an alias is
//! unused exactly when deleting it leaves this test green.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Repo root, from this test binary's manifest directory.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("kernel/ has a parent")
        .to_path_buf()
}

fn esl_files(root: &Path) -> Vec<PathBuf> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "*.esl"])
        .output()
        .expect("git ls-files runs");
    assert!(out.status.success(), "git ls-files failed");
    String::from_utf8(out.stdout)
        .expect("paths are utf-8")
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| root.join(l))
        .collect()
}

/// Blank out string literals and `//` comments — a prefix mentioned in prose or inside an IRI
/// literal is not a use, and descriptions in this tree mention other namespaces constantly.
fn code_only(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' if in_string => {
                chars.next();
                out.push(' ');
                out.push(' ');
            }
            '"' => {
                in_string = !in_string;
                out.push('"');
            }
            _ if in_string => out.push(' '),
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            _ => out.push(c),
        }
    }
    out
}

fn declared_prefixes(src: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in src.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("namespace ") {
            if let Some(name) = rest.split(['=', ' ']).find(|s| !s.is_empty()) {
                out.insert(name.trim().to_string());
            }
        }
    }
    out
}

/// Prefixes used in code position: `foo:Bar` / `foo:bar_baz` where `foo` is lower-case.
fn used_prefixes(code: &str) -> BTreeSet<String> {
    let bytes: Vec<char> = code.chars().collect();
    let mut out = BTreeSet::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == ':' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            if next.is_ascii_alphabetic() || next == '_' {
                // walk back over the prefix
                let mut j = i;
                while j > 0 && (bytes[j - 1].is_ascii_alphanumeric() || bytes[j - 1] == '_') {
                    j -= 1;
                }
                if j < i && bytes[j].is_ascii_lowercase() {
                    // not part of a longer `a:b:c` chain's tail, and not `urn:...`
                    let prefix: String = bytes[j..i].iter().collect();
                    let preceded_by_colon = j > 0 && bytes[j - 1] == ':';
                    if !preceded_by_colon && prefix != "urn" && prefix != "http" {
                        out.insert(prefix);
                    }
                }
            }
        }
        i += 1;
    }
    out
}

#[test]
fn every_esl_declares_every_prefix_it_uses() {
    let root = repo_root();
    let files = esl_files(&root);
    assert!(
        files.len() > 50,
        "expected the tree's .esl files, got {}",
        files.len()
    );

    let mut problems: Vec<String> = Vec::new();
    for path in &files {
        let src = std::fs::read_to_string(path).expect("esl file reads");
        // A file with no `namespace` line declares nothing and uses nothing qualified; the
        // publication chains include such continuation fragments.
        let declared = declared_prefixes(&src);
        if declared.is_empty() {
            continue;
        }
        let used = used_prefixes(&code_only(&src));
        let missing: Vec<&String> = used.difference(&declared).collect();
        if !missing.is_empty() {
            let rel = path.strip_prefix(&root).unwrap_or(path).display();
            problems.push(format!("  {rel}: uses undeclared {missing:?}"));
        }
    }
    assert!(
        problems.is_empty(),
        "every .esl must declare the namespace prefixes it uses:\n{}",
        problems.join("\n")
    );
}

#[test]
fn every_esl_parses() {
    let root = repo_root();
    let mut problems: Vec<String> = Vec::new();
    for path in esl_files(&root) {
        let src = std::fs::read_to_string(&path).expect("esl file reads");
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .display()
            .to_string();
        match eigenius_kernel::esl::lexer::tokenize(&src) {
            Err(e) => problems.push(format!("  {rel}: tokenize: {e:?}")),
            Ok(tokens) => {
                if let Err(e) = eigenius_kernel::esl::parser::parse(&tokens) {
                    problems.push(format!("  {rel}: parse: {e:?}"));
                }
            }
        }
    }
    assert!(
        problems.is_empty(),
        "every .esl must parse:\n{}",
        problems.join("\n")
    );
}
