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

//! `pmi-registry-convert` — PMI Case Registry records to an ESL layer and a findings report
//! (D99 step 4).
//!
//! ```text
//! pmi-registry-convert \
//!   --cases "experiments/uab/UAB Round 1/02-synthetic-pmi-registry/synthetic-cases.json" \
//!   --out registry.esl --report registry-report.md
//! ```
//!
//! The HP-label check needs `references/hpo/hp.json` (the loaded release) and
//! `references/hpo/history/*.obo` (`scripts/provision-hpo-history.sh`).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use eigenius_pmi_registry::convert::{convert, report, Disposition};
use eigenius_pmi_registry::hpo_history::HpoHistory;
use eigenius_pmi_registry::record::Package;

#[derive(Parser)]
#[command(about = "Convert PMI Case Registry records to an ESL layer (D99 step 4)")]
struct Args {
    /// The registry package: `{ "cases": [...] }`, linked records inlined.
    #[arg(long)]
    cases: PathBuf,
    /// The HPO release the chain loads, as OBO Graphs JSON.
    #[arg(long, default_value = "references/hpo/hp.json")]
    hpo_json: PathBuf,
    /// Earlier HPO releases, one `hp.obo` per file.
    #[arg(long, default_value = "references/hpo/history")]
    hpo_history: PathBuf,
    /// The ESL layer to write.
    #[arg(long)]
    out: PathBuf,
    /// The findings report (markdown) to write.
    #[arg(long)]
    report: PathBuf,
}

fn run(args: Args) -> Result<(), String> {
    let text = std::fs::read_to_string(&args.cases)
        .map_err(|e| format!("{}: {e}", args.cases.display()))?;
    let package: Package =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", args.cases.display()))?;
    let hpo = HpoHistory::load(&args.hpo_json, &args.hpo_history)?;
    let input = args.cases.display().to_string();
    let conversion = convert(&package, &hpo, &input);
    std::fs::write(&args.out, &conversion.esl)
        .map_err(|e| format!("{}: {e}", args.out.display()))?;
    std::fs::write(&args.report, report(&conversion, &input, &hpo))
        .map_err(|e| format!("{}: {e}", args.report.display()))?;
    let held = conversion
        .findings
        .iter()
        .filter(|f| f.disposition == Disposition::Held)
        .count();
    let c = conversion.counts;
    eprintln!(
        "{} case(s), {} resource(s), {} declaration(s); {held} item(s) held back → {}, {}",
        c.cases,
        c.resources,
        c.declarations,
        args.out.display(),
        args.report.display()
    );
    Ok(())
}

fn main() -> ExitCode {
    match run(Args::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
