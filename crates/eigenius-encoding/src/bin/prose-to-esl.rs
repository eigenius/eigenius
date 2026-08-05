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

//! `prose-to-esl` — parse prose over a lexicon snapshot and write the D62 encoding record as
//! ESL source — the same record, in the language it was authored in.
//!
//! The pipeline lives in [`eigenius_encoding::pipeline`]; this binary only fixes the output
//! format. See that module for the flags and the fail-closed contract.

use std::process::ExitCode;

use clap::Parser as ClapParser;
use eigenius_encoding::pipeline::{run, Args, OutputFormat};

fn main() -> ExitCode {
    match run(&Args::parse(), OutputFormat::Esl) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("\nprose-to-esl: {e}");
            ExitCode::FAILURE
        }
    }
}
