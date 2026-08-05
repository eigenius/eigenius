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

//! Eigenius CLI — primary developer interface for the Eigenius platform.

// Heap-profiling allocator (opt-in, `--features jemalloc-prof`). Swaps in jemalloc so a `serve`
// process dumps live-heap profiles under `_RJEM_MALLOC_CONF` (diagnosing the reseed OOM,
// docs/notes/reseed-oom-memory-investigation.md §6). Off by default → the system allocator, zero impact.
#[cfg(feature = "jemalloc-prof")]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

use clap::{Parser, Subcommand, ValueEnum};

// Phase 19a.5 (D31): mirror / env / institution lifecycle CLI verbs.
mod common;
mod data;
mod env;
mod institutions;
mod mirror;
mod scripts;
use eigenius_kernel::bootstrap;
use eigenius_kernel::context::ExecutionContext;
use eigenius_kernel::lattice;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::{eigon_json, Iri};
use eigenius_kernel::storage::memory::MemoryPersistentBackend;
use eigenius_kernel::storage::PersistentBackend;
use eigenius_kernel::validation::{CommitWorkingSet, Validator};
use std::sync::Arc;

/// Bootstrap a local-mode session against an in-memory persistent backend.
///
/// The CLI's local-mode commands (`load`, `query`, `reflect`, ...) need
/// a `PersistentBackend` to drive `lattice::commit_layer_default` post
/// D41 — the bare `LayerStorage::in_memory()` path has no backend
/// attached and `commit_layer_default` requires one. Returning the
/// backend alongside the context keeps it alive for the commit calls
/// without leaking it into every callsite.
fn bootstrap_local(
) -> Result<(ExecutionContext, Arc<MemoryPersistentBackend>), bootstrap::BootstrapError> {
    let backend = Arc::new(MemoryPersistentBackend::new());
    let storage = LayerStorage::with_persistent(Arc::clone(&backend) as Arc<dyn PersistentBackend>);
    let ctx = bootstrap::bootstrap_with_storage(storage)?;
    Ok((ctx, backend))
}

/// Single-layer commit through the D41 pipeline: takes the working
/// builder, runs it through `commit_layer_default`, and advances
/// `ctx.head` to the resulting layer. The CLI's local-mode handlers
/// share this exact shape across `load`, `query --file`, `reflect`,
/// and `load_file_into_context`.
fn commit_and_advance(
    ctx: &mut ExecutionContext,
    backend: &dyn PersistentBackend,
    name: &str,
) -> Result<Arc<Layer>, String> {
    let working = ctx
        .take_working(name)
        .map_err(|e| format!("take_working: {e}"))?;
    let layer = lattice::commit_layer_default(working, ctx.storage().clone(), backend)
        .map_err(|e| format!("{e}"))?;
    ctx.advance_head(Arc::clone(&layer), name)
        .map_err(|e| format!("advance_head: {e}"))?;
    Ok(layer)
}

/// Policy-aware variant: applies `explicit_tombstones` to the working
/// builder, then commits with the user-supplied `CommitPolicy`. Returns
/// the full `CommitOutcome` so callers can surface cascade results.
fn commit_and_advance_with_policy(
    ctx: &mut ExecutionContext,
    backend: &dyn PersistentBackend,
    name: &str,
    policy: lattice::CommitPolicy,
    explicit_tombstones: &[String],
) -> Result<lattice::CommitOutcome, String> {
    for tomb in explicit_tombstones {
        let iri = Iri::parse(tomb).map_err(|e| format!("invalid tombstone IRI {tomb:?}: {e}"))?;
        ctx.tombstone(iri).map_err(|e| format!("tombstone: {e}"))?;
    }
    let working = ctx
        .take_working(name)
        .map_err(|e| format!("take_working: {e}"))?;
    let mut ws = CommitWorkingSet::in_memory();
    let outcome = lattice::commit_layer(working, ctx.storage().clone(), backend, policy, &mut ws)
        .map_err(|e| format!("{e}"))?;
    ctx.advance_head(Arc::clone(&outcome.layer), name)
        .map_err(|e| format!("advance_head: {e}"))?;
    Ok(outcome)
}

/// CLI representation of `lattice::CommitPolicy`. `reject` is the
/// default; `cascade` opts into `CascadeTombstone`.
#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum CommitPolicyArg {
    #[default]
    Reject,
    Cascade,
}

impl CommitPolicyArg {
    fn to_lattice(self, max_violations: usize) -> lattice::CommitPolicy {
        match self {
            Self::Reject => lattice::CommitPolicy::Reject { max_violations },
            Self::Cascade => lattice::CommitPolicy::CascadeTombstone,
        }
    }

    fn to_proto(
        self,
        max_violations: usize,
    ) -> Option<eigenius_kernel::server::proto::CommitPolicy> {
        use eigenius_kernel::server::proto::{
            commit_policy::{CascadeTombstone, Reject, Variant},
            CommitPolicy as ProtoPolicy,
        };
        let variant = match self {
            Self::Reject => Variant::Reject(Reject {
                max_violations: max_violations as u32,
            }),
            Self::Cascade => Variant::CascadeTombstone(CascadeTombstone {}),
        };
        Some(ProtoPolicy {
            variant: Some(variant),
        })
    }
}

#[derive(Parser)]
#[command(name = "eigenius")]
#[command(about = "Eigenius — Typed Knowledge Graph Platform", long_about = None)]
#[command(version)]
struct Cli {
    /// Output as JSON
    #[arg(long, global = true)]
    json: bool,

    /// Connect to a remote gRPC endpoint instead of using the local kernel
    #[arg(long, global = true)]
    endpoint: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Load an Eigon-JSON file as a new layer, validate against the stack
    Load {
        /// Path to Eigon-JSON file
        #[arg(value_name = "FILE")]
        file: String,

        /// Branch to commit into (defaults to "main"). Remote mode only.
        #[arg(long, value_name = "BRANCH")]
        branch: Option<String>,

        /// Retroactive-validation policy (D41 §3.3). `reject` (default)
        /// fails the commit on any violation; `cascade` tombstones
        /// violating lower-layer IRIs iteratively to fixpoint.
        #[arg(long, value_enum, default_value_t = CommitPolicyArg::Reject)]
        commit_policy: CommitPolicyArg,

        /// Cap on the number of validation errors surfaced under
        /// `--commit-policy reject`. Ignored under `cascade`.
        #[arg(long, value_name = "N", default_value_t = 100)]
        max_violations: usize,

        /// Tombstone these IRIs as part of the commit (repeatable).
        /// Applied to the user-layer builder before retroactive
        /// validation; under `--commit-policy cascade` they combine
        /// with cascade-inferred tombstones. D41 §10.1.
        #[arg(long = "explicit-tombstone", value_name = "IRI")]
        explicit_tombstones: Vec<String>,
    },

    /// Validate an Eigon-JSON file without loading
    Validate {
        /// Path to Eigon-JSON file
        #[arg(value_name = "FILE")]
        file: String,
    },

    /// Execute an EigenQL query
    Query {
        /// EigenQL query string
        #[arg(value_name = "QUERY")]
        query: String,

        /// Optional Eigon-JSON file to load before querying
        #[arg(long, value_name = "FILE")]
        file: Option<String>,

        /// Evaluate the query against a specific LayerId (hex-encoded SHA-256)
        /// instead of the session's active top (D21 §3.6). Useful for
        /// reaching a forked task result layer. Remote mode only.
        #[arg(long, value_name = "LAYER_ID", conflicts_with = "branch")]
        at_layer: Option<String>,

        /// Pin reads to this branch's current head. Mutually exclusive
        /// with --at-layer. Remote mode only.
        #[arg(long, value_name = "BRANCH")]
        branch: Option<String>,

        /// Retroactive-validation policy for the optional `--file` load
        /// step (D41 §3.3). `reject` (default) fails the commit on any
        /// violation; `cascade` tombstones violating lower-layer IRIs
        /// iteratively to fixpoint. Ignored when `--file` is omitted.
        #[arg(long, value_enum, default_value_t = CommitPolicyArg::Reject)]
        commit_policy: CommitPolicyArg,

        /// Cap on validation errors surfaced under `--commit-policy reject`.
        #[arg(long, value_name = "N", default_value_t = 100)]
        max_violations: usize,

        /// Tombstone these IRIs as part of the `--file` load step
        /// (repeatable). D41 §10.1.
        #[arg(long = "explicit-tombstone", value_name = "IRI")]
        explicit_tombstones: Vec<String>,
    },

    /// Type-check a program
    ProgramValidate {
        /// Path to program Eigon-JSON file
        #[arg(value_name = "PROGRAM_FILE")]
        program_file: String,

        /// Optional ontology file to load first
        #[arg(long, value_name = "FILE")]
        ontology: Option<String>,
    },

    /// Execute a program
    /// Execute a program (requires --endpoint)
    Run {
        /// Path to program file (Eigon-JSON or ESL)
        #[arg(value_name = "PROGRAM_FILE")]
        program_file: String,

        /// Path to input file (Eigon-JSON or ESL)
        #[arg(value_name = "INPUT_FILE")]
        input_file: String,

        /// Branch the trace layer commits into (defaults to "main").
        #[arg(long, value_name = "BRANCH")]
        branch: Option<String>,
    },

    /// Print a resource by IRI
    Inspect {
        /// IRI of the resource to inspect
        #[arg(value_name = "IRI")]
        iri: String,

        /// Resolve against a specific LayerId (hex-encoded SHA-256)
        /// instead of the session's active top (D21 §3.6). Remote
        /// mode only.
        #[arg(long, value_name = "LAYER_ID", conflicts_with = "branch")]
        at_layer: Option<String>,

        /// Pin reads to this branch's current head. Mutually exclusive
        /// with --at-layer. Remote mode only.
        #[arg(long, value_name = "BRANCH")]
        branch: Option<String>,
    },

    /// Start the gRPC server
    Serve {
        /// gRPC port
        #[arg(long, default_value = "50051")]
        port: u16,

        /// Orchestrator endpoint for IO component dispatch
        #[arg(long, env = "EIGENIUS_ORCHESTRATOR_ENDPOINT")]
        orchestrator: Option<String>,

        /// Path to a RocksDB directory for persistent state. When omitted,
        /// the server runs in-memory and loses all state on exit.
        /// See D13 — Durable Kernel State.
        #[arg(long, env = "EIGENIUS_DB", value_name = "PATH")]
        db: Option<String>,

        /// Resource-cache budget: max resource entries held in memory before
        /// eviction (D23 §5.3). Cold reads page from the backend on demand,
        /// so this caps resident memory for large graphs / bulk loads without
        /// bounding what the kernel can serve. Default 250k.
        #[arg(
            long,
            env = "EIGENIUS_CACHE_BUDGET",
            value_name = "ENTRIES",
            default_value_t = 250_000
        )]
        cache_budget: u64,

        /// WordNet dict directory for the Morphy lemmatizer used by the `ParseSentence` RPC
        /// (surface→lemma, e.g. `events`→`event`). Defaults to the in-repo path; if it can't be
        /// loaded the server falls back to the no-op `Identity` lemmatizer. (D63/GH#97 Lever 1 —
        /// will move to orchestrator-owned lexicon provisioning.)
        #[arg(
            long,
            env = "EIGENIUS_MORPHY_DICT",
            value_name = "PATH",
            default_value = "references/WordNet-3.0/dict"
        )]
        morphy_dict: String,
    },

    /// Database administration
    Db {
        #[command(subcommand)]
        command: DbCommands,
    },

    /// Compile an ESL file to Eigon-JSON
    Compile {
        /// Path to ESL (.esl) file
        #[arg(value_name = "FILE")]
        file: String,
    },

    /// Decompile Eigon-JSON back to ESL source
    Decompile {
        /// Path to an Eigon-JSON (.json) file
        #[arg(value_name = "FILE")]
        file: String,

        /// Re-compile the printed ESL and check it yields the same terms (alpha-equal)
        #[arg(long)]
        verify: bool,

        /// Indent expression trees across lines instead of emitting each term on one line
        #[arg(long)]
        pretty: bool,
    },

    /// Record a reasoning trace
    Reflect {
        /// Path to trace file (Eigon-JSON or ESL)
        #[arg(value_name = "FILE")]
        file: String,
    },

    /// List registered institutions (requires --endpoint)
    ListInstitutions,

    /// Generate JSON Schema for an ontology class (requires --endpoint)
    GetSchema {
        /// IRI of the class
        #[arg(value_name = "CLASS_IRI")]
        class_iri: String,
    },

    /// Manage capabilities (components and institutions)
    Capability {
        #[command(subcommand)]
        command: CapabilityCommands,
    },

    /// Manage runtime package mirrors (D31 §3, Phase 19a.5.a)
    Mirror {
        #[command(subcommand)]
        command: MirrorCommands,
    },

    /// Manage runtime environments (D31 §4.2, Phase 19a.5.b)
    Env {
        #[command(subcommand)]
        command: EnvCommands,
    },

    /// Publish, list, inspect, and run runtime scripts (D26 §10)
    Script {
        #[command(subcommand)]
        command: ScriptCommands,
    },

    /// Attach, list, and inspect external data files (D53)
    Data {
        #[command(subcommand)]
        command: DataCommands,
    },

    /// Lexicon tooling — the kernel-side felicity gate for the D62 prose→trees
    /// engine (D62 §8.6)
    Lexicon {
        #[command(subcommand)]
        command: LexiconCommands,
    },

    /// Manage external institutions (D31 §5, Phase 19a.5.e)
    Institution {
        #[command(subcommand)]
        command: InstitutionCommands,
    },

    /// Inspect and control persisted tasks (D21). Remote mode only.
    Tasks {
        #[command(subcommand)]
        command: TaskCommands,
    },

    /// Manage branch refs (Phase 14g). Remote mode only — branches
    /// require a persistent backend.
    Branch {
        #[command(subcommand)]
        command: BranchCommands,
    },

    /// Show version and build info
    Version,
}

#[derive(Subcommand)]
enum BranchCommands {
    /// List every branch ref with its current head
    List,

    /// Show a single branch's current head
    Show {
        /// Branch name
        #[arg(value_name = "NAME")]
        name: String,
    },

    /// Create a new branch pointing at an existing layer
    Create {
        /// Branch name (must match [A-Za-z0-9_-]+)
        #[arg(value_name = "NAME")]
        name: String,

        /// Hex-encoded LayerId to start the branch from
        #[arg(long, value_name = "LAYER_ID")]
        from: String,
    },

    /// Delete a branch ref. Layers reachable only through this branch
    /// are reclaimed by the next GC pass.
    Delete {
        /// Branch name
        #[arg(value_name = "NAME")]
        name: String,

        /// Skip the safety check that refuses to prune a branch whose
        /// head matches an active task pin
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum TaskCommands {
    /// List all tasks in the session
    List,

    /// Show a task's status and metadata
    Status {
        /// Task UUID
        #[arg(value_name = "TASK_ID")]
        task_id: String,
    },

    /// Request cooperative cancellation of a task
    Cancel {
        /// Task UUID
        #[arg(value_name = "TASK_ID")]
        task_id: String,
    },
}

#[derive(Subcommand)]
enum CapabilityCommands {
    /// List registered components and institutions
    List,

    /// Inspect a registered capability by IRI
    Inspect {
        /// IRI of the component or institution
        #[arg(value_name = "IRI")]
        iri: String,
    },

    /// Invoke a registered capability with test input
    Test {
        /// IRI of the capability to test
        #[arg(value_name = "IRI")]
        iri: String,

        /// Input file (Eigon-JSON or ESL)
        #[arg(long, value_name = "FILE")]
        input: String,

        /// For institutions: dispatch as fiber query (default) or discover-morphisms
        #[arg(long, value_name = "MODE", default_value = "query")]
        mode: String,
    },
}

#[derive(Subcommand)]
enum MirrorCommands {
    /// Generate a mirror against a layer and commit the RuntimePackageMirror to the chain.
    Create {
        /// IRI of the layer the mirror anchors to.
        #[arg(long, value_name = "LAYER_IRI")]
        layer: String,

        /// Inline EigenQL query selecting class IRIs (mutually exclusive with --filter-file).
        #[arg(long, value_name = "EIGENQL", conflicts_with = "filter_file")]
        filter: Option<String>,

        /// Path to a file containing the EigenQL filter query.
        #[arg(long, value_name = "FILE")]
        filter_file: Option<String>,

        /// Target language: julia (other languages are planned per D31 §7).
        #[arg(long, value_name = "LANG", default_value = "julia")]
        language: String,

        /// Output directory the source files will be written to (D31 §3.2).
        #[arg(long, value_name = "DIR")]
        output: String,

        /// Optional path to an institution declaration file (the same
        /// file passed to `eigenius institution install`). When set,
        /// the seed is augmented with every class referenced by the
        /// file's `RuntimeMethodSignature.input_types` / `output_type`
        /// declarations. Lets notebook authors omit cross-institution
        /// return classes (e.g. `OptimisationProblem` in a Symbolics
        /// mirror seed) — the closure walker discovers them
        /// automatically from the institution's signature contracts.
        ///
        /// Reads the file directly rather than querying the chain
        /// because mirror generation runs *before* the institution is
        /// installed (the env image bakes the mirror in, and the
        /// institution declaration references the env IRI). The file
        /// is the source of truth at this stage of the pipeline.
        #[arg(long, value_name = "FILE")]
        institution_file: Option<String>,
    },

    /// Retrieve a previously-created mirror's source files (D31 §3.5). No commit.
    Get {
        /// IRI of the RuntimePackageMirror to fetch.
        #[arg(long, value_name = "MIRROR_IRI")]
        iri: String,

        /// Output directory the source files will be written to.
        #[arg(long, value_name = "DIR")]
        output: String,
    },

    /// List committed mirrors.
    List {
        /// Optional language filter.
        #[arg(long, value_name = "LANG")]
        language: Option<String>,
    },

    /// Inspect a mirror's metadata.
    Inspect {
        /// IRI of the RuntimePackageMirror.
        #[arg(value_name = "MIRROR_IRI")]
        iri: String,
    },
}

#[derive(Subcommand)]
enum EnvCommands {
    /// Build an env image from the handler package in the working
    /// directory. Reads `./Project.toml` + `./src/` (or
    /// `--package-path` if set), fetches the named mirror from the
    /// chain, and runs the substrate's `build_environment_image` with
    /// the handler package + mirror baked in. Prints the resulting
    /// `sha256:` image digest. Pass that digest to `env create` to
    /// commit the `RuntimeEnvironment` resource.
    Build {
        /// Target language: `julia` for v1.
        #[arg(long, value_name = "LANG", default_value = "julia")]
        language: String,

        /// Handler package directory (defaults to current working directory).
        /// The directory must contain `Project.toml` and `src/`.
        #[arg(long, value_name = "DIR")]
        package_path: Option<String>,

        /// IRI of a previously-committed `RuntimePackageMirror` to bake in.
        /// Required for `julia`; optional for `r` (the R image bakes the
        /// pinned `RImagePlan` packages — limma/fgsea/lme4 — and has no
        /// mirror until the P4 S4 generator lands).
        #[arg(long, value_name = "MIRROR_IRI")]
        mirror: Option<String>,

        /// Override the language's default base image. Pin by digest in
        /// production (e.g. `julia@sha256:...`) so builds stay
        /// reproducible. For `r` the default is the Bioconductor base.
        #[arg(long, value_name = "REF", default_value = "julia:1.12-bookworm")]
        base_image: String,

        /// (R only) Path to the `EigeniusRWorker.R` driver. Defaults to
        /// `crates/eigenius-r-worker/r/EigeniusRWorker.R` in the workspace.
        #[arg(long, value_name = "FILE")]
        r_driver: Option<String>,

        /// (R only) Path to the `libeigenius_r_worker.so` cdylib. Defaults
        /// to `target/{release,debug}/libeigenius_r_worker.so`. Build it
        /// first with `cargo build -p eigenius-r-worker --release`.
        #[arg(long, value_name = "FILE")]
        r_cdylib: Option<String>,

        /// (R only) Bioconductor/CRAN package to bake into the image
        /// (repeatable). When given, this explicit list drives the build
        /// instead of the compiled-in `RImagePlan::default`. The set MUST
        /// match the orchestrator's compiled default, or the worker's boot
        /// cross-check (D26 §9.3) rejects the image. Empty ⇒ use the default.
        #[arg(long = "r-package", value_name = "PKG")]
        r_package: Vec<String>,

        /// (R only) Bioconductor release the `--r-package`s install from
        /// (e.g. `3.18`). Defaults to the `RImagePlan::default` release.
        #[arg(long, value_name = "VERSION")]
        bioc_version: Option<String>,

        /// Path to the Julia worker's project directory (must contain
        /// `Project.toml`, `Manifest.toml`, `src/JuliaWorker.jl`).
        /// Defaults to `julia/runtime-worker/` resolved against
        /// `$EIGENIUS_HOME` (or, in dev, the workspace root the CLI
        /// was built under). The worker source is what the substrate
        /// stages as the env image's PID 1; production deployments
        /// should pin a specific worker version.
        #[arg(long, value_name = "DIR")]
        worker_source_dir: Option<String>,

        /// Build context / depot path. Builds materialise the
        /// Dockerfile + COPYs under this directory; `buildah` reads
        /// from here. Defaults to a fresh temp directory.
        #[arg(long, value_name = "DIR")]
        depot: Option<String>,
    },

    /// Build a runtime environment image and commit a RuntimeEnvironment resource.
    Create {
        /// Target language: julia (others planned per D31 §7).
        #[arg(long, value_name = "LANG", default_value = "julia")]
        language: String,

        /// Path to the developer's handler-package directory (D31 §4.1).
        #[arg(long, value_name = "DIR")]
        handler_package: String,

        /// IRI of a previously-committed RuntimePackageMirror to bake in.
        #[arg(long, value_name = "MIRROR_IRI")]
        mirror: String,

        /// Extra package directories to bake in as path-deps (repeatable).
        #[arg(long, value_name = "DIR")]
        include_package: Vec<String>,

        /// IRI to commit the RuntimeEnvironment under.
        #[arg(long, value_name = "ENV_IRI")]
        as_iri: String,

        /// Override the language's default base image (e.g. `julia:1.12-bookworm@sha256:...`).
        #[arg(long, value_name = "REF")]
        base_image: Option<String>,

        /// Image digest of a pre-built env image (`sha256:...`). v1 of
        /// `env create` commits the RuntimeEnvironment resource against
        /// this digest; the integrated image-build path lands in a
        /// follow-up milestone (D31 §4.2 / Phase 19a.5.b proper).
        #[arg(long, value_name = "DIGEST")]
        image_digest: String,

        /// Exact runtime version pinned in the image (e.g. `1.12.1`
        /// for Julia, `3.12.2` for Python). `eigenius env build`
        /// captures this from the built image and prints it; pass that
        /// value through. Required by the chain ontology on
        /// `RuntimeEnvironment.runtime_version`.
        #[arg(long, value_name = "VERSION")]
        runtime_version: String,
    },

    /// List committed environments.
    List {
        /// Optional language filter.
        #[arg(long, value_name = "LANG")]
        language: Option<String>,
    },

    /// Inspect a runtime environment's metadata.
    Inspect {
        /// IRI of the RuntimeEnvironment.
        #[arg(value_name = "ENV_IRI")]
        iri: String,
    },
}

#[derive(Subcommand)]
enum ScriptCommands {
    /// Publish a script as a content-addressed RuntimeScript resource.
    /// Cheap — just a graph commit. The language is inferred from the
    /// file extension (.r/.jl/.py/.lean) unless `--lang` is given.
    Publish {
        /// Path to the script source file.
        #[arg(value_name = "FILE")]
        file: String,

        /// IRI of the RuntimeEnvironment the script declares as compatible.
        #[arg(long, value_name = "ENV_IRI")]
        env: String,

        /// Override the inferred language identifier (e.g. `r`, `julia`).
        #[arg(long, value_name = "LANG")]
        lang: Option<String>,

        /// Declared entry-point name. Optional — omit for a top-level
        /// script (the common RunRuntimeScript case); set it only when
        /// the script exposes a typed entry point.
        #[arg(long, value_name = "NAME")]
        entry_point: Option<String>,

        /// Human-readable description.
        #[arg(long, value_name = "TEXT")]
        description: Option<String>,
    },

    /// List published runtime scripts.
    List {
        /// Optional language filter.
        #[arg(long, value_name = "LANG")]
        lang: Option<String>,
    },

    /// Inspect a published script's metadata and source.
    Inspect {
        /// IRI of the RuntimeScript.
        #[arg(value_name = "SCRIPT_IRI")]
        iri: String,
    },

    /// Run a published script against a graph-resident input resource.
    /// The kernel resolves the script's source and environment from the
    /// graph at execution (D26 §6.2).
    Run {
        /// IRI of the published RuntimeScript.
        #[arg(value_name = "SCRIPT_IRI")]
        iri: String,

        /// Comma-separated input resource IRIs. v1 takes exactly one.
        #[arg(long, value_name = "IRI", value_delimiter = ',')]
        inputs: Vec<String>,

        /// Branch the trace layer commits into (defaults to "main").
        #[arg(long, value_name = "BRANCH")]
        branch: Option<String>,
    },
}

/// `lexicon` subcommands — the kernel-side, **trusted half** of the D62
/// prose→trees engine. An *untrusted* tool (WordNet/VerbNet → LLM) drafts
/// categorial lexical entries as ESL / Eigon-JSON; these subcommands admit or
/// reject them against the kernel, the felicity oracle.
#[derive(Subcommand)]
enum LexiconCommands {
    /// Run the felicity gate over every `lexicon:LexicalEntry` in a file: for
    /// each entry, check `⟦cat⟧ ≡ sem_type` and that its `sem` actually inhabits
    /// `⟦cat⟧`. Fail-closed — any rejection exits non-zero.
    Gate {
        /// One or more ESL (`.esl`) / Eigon-JSON files. All load into one layer
        /// over the bootstrap chain, so entries may reference a schema / domain
        /// in an earlier file; every `lexicon:LexicalEntry` across them is gated.
        #[arg(value_name = "FILE", num_args = 1..)]
        files: Vec<String>,
    },
    /// Parse a natural-language sentence against the served lexicon (D63/D65),
    /// printing the typed parse forest. With `--endpoint` this calls the kernel's
    /// `ParseSentence` RPC over the committed chain; locally it builds the index over
    /// the bootstrap chain plus any `--file` domain layers.
    Parse {
        /// The sentence to parse, e.g. "every Werner syndrome affects HeLa".
        #[arg(value_name = "SENTENCE")]
        sentence: String,
        /// Restrict the parse to these `lexicon:Lexicon` IRIs (repeatable). Order is
        /// resolution precedence (earlier ranks first). None = whole chain, unscoped.
        #[arg(long = "scope", value_name = "LEXICON_IRI")]
        scope: Vec<String>,
        /// A `lexicon:LexiconProfile` IRI naming an ordered scope (mutually exclusive
        /// with `--scope`).
        #[arg(long, value_name = "PROFILE_IRI")]
        profile: Option<String>,
        /// (Local mode only) ESL/Eigon-JSON domain files to load over bootstrap as a
        /// chain before parsing — e.g. a domain lexicon + the demo verbs.
        #[arg(long = "file", value_name = "FILE")]
        files: Vec<String>,
    },
}

#[derive(Subcommand)]
enum DataCommands {
    /// Attach an external file as a content-addressed PinnedExternalFile.
    /// Computes the hash from the bytes (a local path / `file://`, or an
    /// `oxen://` reference the CLI fetches once) and commits the typed node;
    /// the bytes stay off-chain. The IRI is content-addressed (D53 §3).
    Attach {
        /// A local file path, a `file://` URL, or an `oxen://repo@rev/path`
        /// reference. For `oxen://` the CLI downloads once to compute the hash.
        #[arg(value_name = "FILE_OR_REF")]
        file: String,

        /// Durable backend locator stored on the node (the substrate fetches
        /// from this later). Defaults to a `file://` URL of the absolute path
        /// for local files, or the `oxen://` reference itself.
        #[arg(long, value_name = "REFERENCE")]
        reference: Option<String>,

        /// Override the inferred media type (e.g. `text/csv`).
        #[arg(long, value_name = "MEDIA_TYPE")]
        media_type: Option<String>,

        /// Override the short name (defaults to the file name).
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
    },

    /// List attached external files.
    List {
        /// Optional media-type filter.
        #[arg(long, value_name = "MEDIA_TYPE")]
        media_type: Option<String>,
    },

    /// Inspect a pinned external file's metadata.
    Inspect {
        /// IRI of the PinnedExternalFile.
        #[arg(value_name = "DATA_IRI")]
        iri: String,
    },

    /// Verify an attached file: fetch it by its `reference`, recompute the
    /// content hash, and check it against the pinned `content_hash`
    /// (fail closed — D53 §5). Proves the off-chain bytes still match.
    Verify {
        /// IRI of the PinnedExternalFile to verify.
        #[arg(value_name = "DATA_IRI")]
        iri: String,
    },

    /// Validate the bound DatasetSchema against the file's actual columns —
    /// the D53 §4.1 checkable layout gate. Materializes (content-verifies) the
    /// file and header-scans it (CSV/TSV; columnar formats defer to the worker).
    Validate {
        /// IRI of the PinnedExternalFile to validate.
        #[arg(value_name = "DATA_IRI")]
        iri: String,
    },

    /// Provision a PinnedExternalFile into the local content-addressed cache
    /// the kernel reads for native file-backed SampleSet recompute (D53 §6.1
    /// / §7). Fetches + content-verifies into `<cache>/<hash>/<name>`. Run on
    /// the host whose depot the kernel reads.
    Provision {
        /// IRI of the PinnedExternalFile to provision.
        #[arg(value_name = "DATA_IRI")]
        iri: String,

        /// Cache root (the depot's extfile-cache). Defaults to
        /// `$EIGENIUS_EXTFILE_CACHE_DIR`.
        #[arg(long, value_name = "DIR")]
        cache_root: Option<String>,
    },
}

#[derive(Subcommand)]
enum InstitutionCommands {
    /// Install an institution by submitting its definition to the chain.
    Install {
        /// Path to the Eigon-JSON / ESL definition file (D31 §5.2).
        #[arg(long, value_name = "FILE")]
        definition: String,
    },

    /// List installed institutions.
    List,

    /// Inspect an institution's declarations.
    Inspect {
        /// IRI of the Institution resource.
        #[arg(value_name = "IRI")]
        iri: String,
    },
}

#[derive(Subcommand)]
enum DbCommands {
    /// Print storage statistics
    Stats {
        /// RocksDB path
        #[arg(value_name = "PATH")]
        path: String,
    },
    /// Trigger manual compaction
    Compact {
        /// RocksDB path
        #[arg(value_name = "PATH")]
        path: String,
    },
    /// Export all resources as Eigon-JSON
    Export {
        /// RocksDB path
        #[arg(value_name = "DB_PATH")]
        db_path: String,
        /// Output directory
        #[arg(value_name = "OUTPUT_PATH")]
        output_path: String,
    },
    /// Collapse `[from..to]` on a branch into one resolve-equivalent layer (D25).
    ///
    /// Requires `--endpoint` — consolidation serialises with branch
    /// updates via the running kernel's branch lock, so it must be
    /// invoked against the live server holding the same DB.
    ///
    /// When `to` equals the branch's current head, the consolidation
    /// advances the branch ref to the new layer (the 17a "at-head"
    /// path). When `to` is strictly below the head, a resolve
    /// redirect is installed at `to` (D25 §12.8 / Phase 17f) and the
    /// branch stays put.
    Consolidate {
        /// `<from-hex>..<to-hex>` — the inclusive consolidation range,
        /// matching the spec language in D25 §5.3.
        #[arg(value_name = "FROM..TO")]
        range: String,
        /// Branch to consolidate. Defaults to `main`.
        #[arg(long, default_value = "main")]
        branch: String,
        /// Override the cost cap. Defaults to the kernel value
        /// (`5_000_000`).
        #[arg(long)]
        max_walk_entries: Option<u64>,
        /// Run `EstimateConsolidation` instead of `ConsolidateChain` —
        /// reports the cost and the predicted consolidated layer's
        /// id without committing.
        #[arg(long)]
        dry_run: bool,
        /// Below-head consolidation only: keep the pre-consolidation
        /// history alive (GC won't reclaim the source range).
        /// Time-travel reads against intermediate layers in the range
        /// continue to resolve. Default reclaim mode is the
        /// at-head-equivalent contract. See D25 §12.8.1(b).
        #[arg(long)]
        preserve_history: bool,
    },
    /// Reconcile a diverged head with a branch (D20). `preview`
    /// computes the cascade impact without committing; `resolve`
    /// applies the resolutions and CAS-advances the branch ref.
    Merge {
        #[command(subcommand)]
        command: MergeCommands,
    },
}

#[derive(Subcommand)]
enum MergeCommands {
    /// Non-mutating dry-run: show the cascade items the resolutions
    /// would generate without applying anything. Print each item's
    /// stable id so the caller can pipe it into `resolve
    /// --acknowledge`.
    Preview {
        /// Branch whose current head is one side of the merge.
        #[arg(long, default_value = "main")]
        branch: String,
        /// Hex-encoded candidate head: the layer the caller built
        /// that diverged from `branch`'s current tip.
        #[arg(long, value_name = "LAYER_ID")]
        candidate: String,
        /// Path to a JSON file containing the tentative resolutions
        /// (array of `{conflict_id, kind, ...}` objects). See
        /// `docs/cli/merge-resolution-format.md` for the schema, or
        /// the source of `parse_resolution_file` for the variants.
        #[arg(long, value_name = "PATH")]
        resolutions: std::path::PathBuf,
    },
    /// Apply resolutions, commit the merge layer, and CAS-advance
    /// the branch ref. Every cascade item produced by the preview
    /// must be acknowledged via repeated `--acknowledge`.
    Resolve {
        /// Branch whose ref will be advanced on success.
        #[arg(long, default_value = "main")]
        branch: String,
        /// Hex-encoded candidate head.
        #[arg(long, value_name = "LAYER_ID")]
        candidate: String,
        /// Path to a JSON file containing the resolutions to apply.
        #[arg(long, value_name = "PATH")]
        resolutions: std::path::PathBuf,
        /// Cascade item id to acknowledge. Repeat for each item.
        /// `merge preview` prints the ids to pass here.
        #[arg(long = "acknowledge", value_name = "ITEM_ID")]
        acknowledgments: Vec<String>,
    },
}

#[tokio::main]
async fn main() {
    // Install the structured-logging subscriber before anything else
    // so initialization events from the kernel land in the configured
    // format. Read `RUST_LOG` for level filter, `EIGENIUS_LOG_FORMAT`
    // for `json` vs `pretty` (defaults: pretty on TTY, json otherwise).
    eigenius_kernel::observability::init();

    let cli = Cli::parse();

    // Remote mode: delegate to gRPC client
    if let Some(ref endpoint) = cli.endpoint {
        match cli.command {
            Commands::Inspect {
                iri,
                at_layer,
                branch,
            } => {
                remote_inspect(
                    endpoint,
                    &iri,
                    at_layer.as_deref(),
                    branch.as_deref(),
                    cli.json,
                )
                .await
            }
            Commands::Query {
                query,
                file: _,
                at_layer,
                branch,
                commit_policy: _,
                max_violations: _,
                explicit_tombstones: _,
            } => {
                // Remote query doesn't surface the --file load path, so
                // the commit-policy flags are accepted but ignored. The
                // remote `Query` RPC has no LoadRequest.
                remote_query(
                    endpoint,
                    &query,
                    at_layer.as_deref(),
                    branch.as_deref(),
                    cli.json,
                )
                .await
            }
            Commands::Run {
                program_file,
                input_file,
                branch,
            } => {
                remote_run(
                    endpoint,
                    &program_file,
                    &input_file,
                    branch.as_deref(),
                    cli.json,
                )
                .await
            }
            Commands::Load {
                file,
                branch,
                commit_policy,
                max_violations,
                explicit_tombstones,
            } => {
                remote_load(
                    endpoint,
                    &file,
                    branch.as_deref(),
                    commit_policy.to_proto(max_violations),
                    &explicit_tombstones,
                    cli.json,
                )
                .await
            }
            Commands::Reflect { file } => remote_reflect(endpoint, &file, cli.json).await,
            Commands::ListInstitutions => remote_list_institutions(endpoint, cli.json).await,
            Commands::GetSchema { class_iri } => {
                remote_get_schema(endpoint, &class_iri, cli.json).await
            }
            Commands::Capability { command } => {
                remote_capability(endpoint, command, cli.json).await
            }
            Commands::Mirror { command } => remote_mirror(endpoint, command, cli.json).await,
            Commands::Env { command } => remote_env(endpoint, command, cli.json).await,
            Commands::Script { command } => remote_script(endpoint, command, cli.json).await,
            Commands::Data { command } => remote_data(endpoint, command, cli.json).await,
            Commands::Institution { command } => {
                remote_institution(endpoint, command, cli.json).await
            }
            Commands::Tasks { command } => remote_tasks(endpoint, command, cli.json).await,
            Commands::Branch { command } => remote_branch(endpoint, command, cli.json).await,
            Commands::Lexicon { command } => match command {
                LexiconCommands::Parse {
                    sentence,
                    scope,
                    profile,
                    files: _,
                } => remote_parse(endpoint, &sentence, &scope, profile.as_deref(), cli.json).await,
                LexiconCommands::Gate { .. } => {
                    eprintln!(
                        "'lexicon gate' is a local-only operation; drop --endpoint and pass files"
                    );
                    std::process::exit(1);
                }
            },
            Commands::Db { command } => match command {
                DbCommands::Consolidate {
                    range,
                    branch,
                    max_walk_entries,
                    dry_run,
                    preserve_history,
                } => {
                    remote_db_consolidate(
                        endpoint,
                        &range,
                        &branch,
                        max_walk_entries,
                        dry_run,
                        preserve_history,
                        cli.json,
                    )
                    .await
                }
                DbCommands::Merge { command } => match command {
                    MergeCommands::Preview {
                        branch,
                        candidate,
                        resolutions,
                    } => {
                        remote_db_merge_preview(
                            endpoint,
                            &branch,
                            &candidate,
                            &resolutions,
                            cli.json,
                        )
                        .await
                    }
                    MergeCommands::Resolve {
                        branch,
                        candidate,
                        resolutions,
                        acknowledgments,
                    } => {
                        remote_db_merge_resolve(
                            endpoint,
                            &branch,
                            &candidate,
                            &resolutions,
                            &acknowledgments,
                            cli.json,
                        )
                        .await
                    }
                },
                _ => {
                    eprintln!(
                        "this 'db' subcommand is a local-only operation; drop --endpoint and pass \
                         the DB path directly"
                    );
                    std::process::exit(1);
                }
            },
            Commands::Serve { .. } => {
                eprintln!("Cannot use --endpoint with serve");
                std::process::exit(1);
            }
            _ => {
                eprintln!("Remote mode not yet supported for this command");
                std::process::exit(1);
            }
        }
        return;
    }

    // Local mode: embedded kernel
    match cli.command {
        Commands::Load {
            file,
            branch: _,
            commit_policy,
            max_violations,
            explicit_tombstones,
        } => cmd_load(
            &file,
            commit_policy.to_lattice(max_violations),
            &explicit_tombstones,
            cli.json,
        ),
        Commands::Validate { file } => cmd_validate(&file, cli.json),
        Commands::Query {
            query,
            file,
            at_layer: _,
            branch: _,
            commit_policy,
            max_violations,
            explicit_tombstones,
        } => cmd_query(
            &query,
            file.as_deref(),
            commit_policy.to_lattice(max_violations),
            &explicit_tombstones,
            cli.json,
        ),
        Commands::ProgramValidate {
            program_file,
            ontology,
        } => cmd_program_validate(&program_file, ontology.as_deref(), cli.json),
        Commands::Run { .. } => {
            eprintln!("'run' requires --endpoint (connect to a running kernel+orchestrator)");
            eprintln!("  eigenius --endpoint http://localhost:50051 run program.json input.json");
            std::process::exit(1);
        }
        Commands::Inspect { iri, .. } => cmd_inspect(&iri, cli.json),
        Commands::Serve {
            port,
            orchestrator,
            db,
            cache_budget,
            morphy_dict,
        } => {
            cmd_serve(
                port,
                orchestrator.as_deref(),
                db.as_deref(),
                cache_budget,
                &morphy_dict,
            )
            .await
        }
        Commands::Compile { file } => cmd_compile(&file, cli.json),
        Commands::Decompile {
            file,
            verify,
            pretty,
        } => cmd_decompile(&file, verify, pretty),
        Commands::Lexicon { command } => match command {
            LexiconCommands::Gate { files } => cmd_lexicon_gate(&files, cli.json),
            LexiconCommands::Parse {
                sentence,
                scope,
                profile,
                files,
            } => cmd_lexicon_parse(&sentence, &scope, profile.as_deref(), &files, cli.json),
        },
        Commands::Reflect { file } => cmd_reflect(&file, cli.json),
        Commands::ListInstitutions => {
            eprintln!("'list-institutions' requires --endpoint");
            std::process::exit(1);
        }
        Commands::GetSchema { .. } => {
            eprintln!("'get-schema' requires --endpoint");
            std::process::exit(1);
        }
        Commands::Capability { .. } => {
            eprintln!("'capability' commands require --endpoint");
            std::process::exit(1);
        }
        Commands::Mirror { .. } => {
            eprintln!("'mirror' commands require --endpoint");
            std::process::exit(1);
        }
        Commands::Env { .. } => {
            eprintln!("'env' commands require --endpoint");
            std::process::exit(1);
        }
        Commands::Script { .. } => {
            eprintln!("'script' commands require --endpoint");
            std::process::exit(1);
        }
        Commands::Data { .. } => {
            eprintln!("'data' commands require --endpoint");
            std::process::exit(1);
        }
        Commands::Institution { .. } => {
            eprintln!("'institution' commands require --endpoint");
            std::process::exit(1);
        }
        Commands::Tasks { .. } => {
            eprintln!("'tasks' commands require --endpoint");
            std::process::exit(1);
        }
        Commands::Branch { .. } => {
            eprintln!("'branch' commands require --endpoint");
            std::process::exit(1);
        }
        Commands::Db { command } => cmd_db(command),
        Commands::Version => {
            println!("eigenius {}", env!("CARGO_PKG_VERSION"));
        }
    }
}

fn cmd_load(
    file: &str,
    policy: lattice::CommitPolicy,
    explicit_tombstones: &[String],
    json_output: bool,
) {
    // Bootstrap
    let (mut ctx, backend) = match bootstrap_local() {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("Bootstrap failed: {e}");
            std::process::exit(1);
        }
    };

    // Read and parse file (auto-detects ESL vs JSON)
    let resources = load_resources_from_file(file);
    let count = resources.len();

    // Add resources to context
    for resource in resources {
        if let Err(e) = ctx.add_resource(resource) {
            eprintln!("Error adding resource: {e}");
            std::process::exit(1);
        }
    }

    // Commit (validates and builds layer) through the D41 pipeline,
    // applying the user-supplied policy + explicit tombstones.
    match commit_and_advance_with_policy(
        &mut ctx,
        backend.as_ref(),
        "loaded",
        policy,
        explicit_tombstones,
    ) {
        Ok(outcome) => {
            let cascade_count = outcome.cascade_tombstones.len();
            let cascade_iters = outcome.cascade_iterations;
            if json_output {
                println!(
                    "{{\"success\":true,\"resource_count\":{count},\"layer_id\":\"{}\",\"branch\":\"main\",\"branch_advanced\":true,\"cascade_tombstones\":{cascade_count},\"cascade_iterations\":{cascade_iters},\"total_violations\":0}}",
                    outcome.layer.id()
                );
            } else {
                println!(
                    "Loaded {count} resource(s) into layer {}",
                    outcome.layer.id()
                );
                println!("Validation passed.");
                if cascade_count > 0 {
                    println!(
                        "Cascade tombstoned {cascade_count} IRI(s) in {cascade_iters} iteration(s)."
                    );
                }
            }
        }
        Err(e) => {
            if json_output {
                eprintln!("{{\"success\":false,\"message\":\"{e}\"}}");
            } else {
                eprintln!("Load failed: {e}");
            }
            std::process::exit(1);
        }
    }
}

fn cmd_query(
    query_str: &str,
    file: Option<&str>,
    policy: lattice::CommitPolicy,
    explicit_tombstones: &[String],
    json_output: bool,
) {
    // Bootstrap
    let (mut ctx, backend) = match bootstrap_local() {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("Bootstrap failed: {e}");
            std::process::exit(1);
        }
    };

    // Optionally load a file first, applying the user-supplied
    // commit policy + explicit tombstones.
    if let Some(file_path) = file {
        let content = match std::fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Failed to read '{file_path}': {e}");
                std::process::exit(1);
            }
        };
        let resources = match eigon_json::parse_document(&content) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Parse error: {e}");
                std::process::exit(1);
            }
        };
        for resource in resources {
            if let Err(e) = ctx.add_resource(resource) {
                eprintln!("Error adding resource: {e}");
                std::process::exit(1);
            }
        }
        if let Err(e) = commit_and_advance_with_policy(
            &mut ctx,
            backend.as_ref(),
            "loaded",
            policy,
            explicit_tombstones,
        ) {
            eprintln!("Load failed: {e}");
            std::process::exit(1);
        }
    }

    // Execute query — returns the full result document (Property resources,
    // row Class, ResultSet with embedded rows) per D2 Appendix A.
    match eigenius_kernel::query::execute(query_str, ctx.head()) {
        Ok(document) => {
            if json_output {
                let json_results: Vec<serde_json::Value> = document
                    .iter()
                    .map(eigon_json::serialize_resource)
                    .collect();
                println!("{}", serde_json::to_string(&json_results).unwrap());
            } else {
                println!("{} resource(s) in result document:", document.len());
                for resource in &document {
                    let json = eigon_json::serialize_resource(resource);
                    println!("{}", serde_json::to_string_pretty(&json).unwrap());
                }
            }
        }
        Err(errors) => {
            if json_output {
                eprintln!("{{\"status\":\"error\",\"error_count\":{}}}", errors.len());
            } else {
                eprintln!("Query failed with {} error(s):", errors.len());
                for e in &errors {
                    eprintln!("  {e}");
                }
            }
            std::process::exit(1);
        }
    }
}

fn cmd_program_validate(program_file: &str, ontology: Option<&str>, json_output: bool) {
    let (mut ctx, backend) = match bootstrap_local() {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("Bootstrap failed: {e}");
            std::process::exit(1);
        }
    };

    // Load ontology if provided
    if let Some(ont_file) = ontology {
        load_file_into_context(&mut ctx, backend.as_ref(), ont_file);
    }

    // Read and parse program
    let content = match std::fs::read_to_string(program_file) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to read '{program_file}': {e}");
            std::process::exit(1);
        }
    };

    let resources = match eigon_json::parse_document(&content) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Parse error: {e}");
            std::process::exit(1);
        }
    };

    let program = match resources.into_iter().next() {
        Some(r) => r,
        None => {
            eprintln!("No resources in program file");
            std::process::exit(1);
        }
    };

    // Parse and type-check
    match eigenius_kernel::program::expr::parse_program(&program, ctx.head()) {
        Ok((_term, typ)) => {
            // Validate output schemas (bijectivity check, D8 §4)
            let schema_errors =
                eigenius_kernel::program::schema::validate_output_schemas(&program, ctx.head());
            if !schema_errors.is_empty() {
                eprintln!("Schema validation failed:");
                for e in &schema_errors {
                    eprintln!("  {e}");
                }
                std::process::exit(1);
            }

            if json_output {
                println!("{{\"status\":\"ok\",\"type\":\"{typ:?}\"}}");
            } else {
                println!("Program type-checks successfully.");
                println!("Type: {typ:?}");
            }
        }
        Err(e) => {
            eprintln!("Program validation failed: {e}");
            std::process::exit(1);
        }
    }
}

fn load_file_into_context(ctx: &mut ExecutionContext, backend: &dyn PersistentBackend, file: &str) {
    let resources = load_resources_from_file(file);
    for resource in resources {
        if let Err(e) = ctx.add_resource(resource) {
            eprintln!("Error loading '{file}': {e}");
            std::process::exit(1);
        }
    }
    if let Err(e) = commit_and_advance(ctx, backend, "loaded") {
        eprintln!("Commit failed for '{file}': {e}");
        std::process::exit(1);
    }
}

fn cmd_validate(file: &str, json_output: bool) {
    // Bootstrap
    let ctx = match bootstrap::bootstrap() {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("Bootstrap failed: {e}");
            std::process::exit(1);
        }
    };

    // Read and parse file (auto-detects ESL vs JSON)
    let resources = load_resources_from_file(file);
    let count = resources.len();

    // Build a temporary layer for validation
    let mut builder = LayerBuilder::new("validate", Some(Arc::clone(ctx.head())));
    for resource in resources {
        if let Err(e) = builder.add_resource(resource) {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
    let layer =
        std::sync::Arc::new(builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));

    // Validate
    let validator = Validator::new(std::sync::Arc::clone(&layer));
    let errors = validator.validate();

    if errors.is_empty() {
        if json_output {
            println!("{{\"status\":\"ok\",\"resources\":{count}}}");
        } else {
            println!("Validated {count} resource(s). No errors.");
        }
    } else {
        if json_output {
            eprintln!("{{\"status\":\"error\",\"error_count\":{}}}", errors.len());
        } else {
            eprintln!("Validation found {} error(s):", errors.len());
            for e in &errors {
                eprintln!("  {e}");
            }
        }
        std::process::exit(1);
    }
}

fn cmd_inspect(iri_str: &str, json_output: bool) {
    // Bootstrap
    let ctx = match bootstrap::bootstrap() {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("Bootstrap failed: {e}");
            std::process::exit(1);
        }
    };

    let iri = match Iri::parse(iri_str) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("Invalid IRI '{iri_str}': {e}");
            std::process::exit(1);
        }
    };

    match ctx.resolve(&iri) {
        Some(resource) => {
            let json = eigon_json::serialize_resource(&resource);
            if json_output {
                println!("{}", serde_json::to_string(&json).unwrap());
            } else {
                println!("{}", serde_json::to_string_pretty(&json).unwrap());
            }
        }
        None => {
            eprintln!("Resource not found: {iri_str}");
            std::process::exit(1);
        }
    }
}

fn cmd_compile(file: &str, json_output: bool) {
    let source = std::fs::read_to_string(file).unwrap_or_else(|e| {
        eprintln!("Failed to read file: {e}");
        std::process::exit(1);
    });

    // Against a bootstrapped layer, not bare: constructor short names resolve through the chain's
    // ctor table (`collect_ctors_from_layer`), so a file citing `reasoning:JustifiedBy`'s ctors
    // compiles here rather than only inside a running server. Seeding only ADDS resolvable names.
    let ctx = bootstrap::bootstrap().unwrap_or_else(|e| {
        eprintln!("Bootstrap failed: {e}");
        std::process::exit(1);
    });
    let resources = eigenius_kernel::esl::compile_against_layer(&source, ctx.head())
        .unwrap_or_else(|errors| {
            for e in &errors {
                eprintln!("{file}: {e}");
            }
            std::process::exit(1);
        });

    // Output as Eigon-JSON array
    let json_values: Vec<serde_json::Value> = resources
        .iter()
        .map(eigon_json::serialize_resource)
        .collect();
    let output = serde_json::Value::Array(json_values);

    if json_output {
        println!("{}", serde_json::to_string(&output).unwrap());
    } else {
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
    }
}

/// Print an Eigon-JSON document back as ESL source — the inverse of [`cmd_compile`].
///
/// `--verify` closes the loop: re-compile the printed source and check every D47 term is
/// alpha-equal to the one in the input, under the same normalisation the witness index uses. A
/// mismatch exits non-zero rather than emitting source that would commit a different object.
fn cmd_decompile(file: &str, verify: bool, pretty: bool) {
    let text = std::fs::read_to_string(file).unwrap_or_else(|e| {
        eprintln!("Failed to read file: {e}");
        std::process::exit(1);
    });
    let doc: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| {
        eprintln!("{file}: not valid JSON: {e}");
        std::process::exit(1);
    });
    let layout = if pretty {
        eigenius_kernel::esl::print::Layout::Pretty
    } else {
        eigenius_kernel::esl::print::Layout::Flat
    };
    let source =
        eigenius_kernel::esl::print::print_document_with(&doc, layout).unwrap_or_else(|e| {
            eprintln!("{file}: cannot decompile: {e}");
            std::process::exit(1);
        });

    if verify {
        // Against a bootstrapped layer: constructor short names resolve through the chain's ctor
        // table, which is where `reasoning:JustifiedBy`'s constructors live.
        let ctx = bootstrap::bootstrap().unwrap_or_else(|e| {
            eprintln!("Bootstrap failed: {e}");
            std::process::exit(1);
        });
        let resources = eigenius_kernel::esl::compile_against_layer(&source, ctx.head())
            .unwrap_or_else(|errors| {
                eprintln!("{file}: decompiled source does not compile:");
                for e in &errors {
                    eprintln!("  {e}");
                }
                std::process::exit(1);
            });
        let back = serde_json::Value::Array(
            resources
                .iter()
                .map(eigon_json::serialize_resource)
                .collect(),
        );
        let mismatches = compare_terms(&doc, &back);
        if !mismatches.is_empty() {
            eprintln!(
                "{file}: {} term(s) changed under round trip:",
                mismatches.len()
            );
            for m in &mismatches {
                eprintln!("  {m}");
            }
            std::process::exit(1);
        }
        eprintln!("verified: every term is alpha-equal after recompiling");
    }

    println!("{source}");
}

/// Compare the D47 terms of two documents by `@id` + property, alpha-canonically.
fn compare_terms(a: &serde_json::Value, b: &serde_json::Value) -> Vec<String> {
    use eigenius_kernel::witness::alpha_canonicalize_proposition_json;
    fn terms(v: &serde_json::Value) -> std::collections::BTreeMap<String, serde_json::Value> {
        let mut out = std::collections::BTreeMap::new();
        let rs = match v {
            serde_json::Value::Array(a) => a.clone(),
            other => vec![other.clone()],
        };
        for r in rs {
            let Some(o) = r.as_object() else { continue };
            let id = o.get("@id").and_then(|x| x.as_str()).unwrap_or("<anon>");
            for (k, val) in o {
                if val.get("ctor").is_some() {
                    out.insert(format!("{id} :: {k}"), val.clone());
                }
            }
        }
        out
    }
    let (ta, tb) = (terms(a), terms(b));
    let mut bad = Vec::new();
    for (k, va) in &ta {
        match tb.get(k) {
            None => bad.push(format!("{k}: absent after round trip")),
            Some(vb) => {
                if alpha_canonicalize_proposition_json(va)
                    != alpha_canonicalize_proposition_json(vb)
                {
                    bad.push(format!("{k}: not alpha-equal"));
                }
            }
        }
    }
    bad
}

/// Run the D62 felicity gate (`eigenius_kernel::dcg::gate_entry`) over every
/// `lexicon:LexicalEntry` in `file`. The kernel is the felicity oracle: an entry
/// is admitted iff `⟦cat⟧ ≡ sem_type` *and* its `sem` inhabits `⟦cat⟧`. This is
/// the trusted endpoint a WordNet/VerbNet → LLM proposer's drafts pass through
/// (D62 §8.6). Fail-closed: any rejection — or no entries at all — exits non-zero.
fn cmd_lexicon_gate(files: &[String], json_output: bool) {
    use eigenius_kernel::dcg::gate_entry;
    use eigenius_kernel::ontology::resource::Value;

    let ctx = match bootstrap::bootstrap() {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("Bootstrap failed: {e}");
            std::process::exit(1);
        }
    };

    let entry_class = Iri::parse("urn:eigenius:lexicon:LexicalEntry").unwrap();
    let form_prop = Iri::parse("urn:eigenius:lexicon:form").unwrap();

    // Load files as a CHAIN: each compiles AGAINST the layer the prior files
    // built (`compile_against_layer` seeds the compiler's ctor table from the
    // chain), so an entries file can reference a `lexicon:Cat` constructor or
    // domain class declared in an earlier schema file. Standalone-compiling each
    // file would leave those cross-file ctor references unresolved.
    let mut layer = Arc::clone(ctx.head());
    let mut entries: Vec<(Iri, Arc<eigenius_kernel::ontology::resource::Resource>)> = Vec::new();
    for (idx, file) in files.iter().enumerate() {
        let resources = load_resources_against_layer(file, &layer);
        let mut builder =
            LayerBuilder::new(&format!("lexicon-gate-{idx}"), Some(Arc::clone(&layer)));
        for resource in resources {
            if let Err(e) = builder.add_resource(resource) {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
        layer = Arc::new(builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));
        // Collect this file's own entries (not the chain below it).
        for (id, resource) in layer.iter_resources() {
            if resource.is_instance_of(&entry_class) {
                entries.push((id, resource));
            }
        }
    }
    // Gate every entry against the FINAL layer — it sees the whole chain, so a
    // reference from any file resolves.
    let final_layer = layer;
    let mut admitted: Vec<(String, String)> = Vec::new(); // (id, form)
    let mut rejected: Vec<(String, String, String)> = Vec::new(); // (id, form, reason)
    for (id, resource) in &entries {
        let form = match resource.get(&form_prop) {
            Some(Value::String(s)) => s.clone(),
            _ => String::new(),
        };
        match gate_entry(&final_layer, resource) {
            Ok(_denoted) => admitted.push((id.as_str().to_string(), form)),
            Err(reason) => rejected.push((id.as_str().to_string(), form, reason)),
        }
    }
    let total = admitted.len() + rejected.len();

    if json_output {
        let rejected_json: Vec<serde_json::Value> = rejected
            .iter()
            .map(|(id, form, reason)| {
                serde_json::json!({ "entry": id, "form": form, "reason": reason })
            })
            .collect();
        let report = serde_json::json!({
            "status": if rejected.is_empty() && total > 0 { "ok" } else { "error" },
            "gated": total,
            "admitted": admitted.len(),
            "rejected": rejected_json,
        });
        let out = serde_json::to_string(&report).unwrap();
        if rejected.is_empty() && total > 0 {
            println!("{out}");
        } else {
            eprintln!("{out}");
        }
    } else {
        for (id, form) in &admitted {
            println!("  ADMIT   {form:<18}  ({id})");
        }
        for (id, form, reason) in &rejected {
            eprintln!("  REJECT  {form:<18}  ({id})  — {reason}");
        }
        if total == 0 {
            eprintln!("No lexicon:LexicalEntry resources found in the given file(s).");
        } else {
            eprintln!(
                "Gated {total} entr{}: {} admitted, {} rejected.",
                if total == 1 { "y" } else { "ies" },
                admitted.len(),
                rejected.len()
            );
        }
    }

    if !rejected.is_empty() || total == 0 {
        std::process::exit(1);
    }
}

/// One projected parse, shared by the local and remote `lexicon parse` renderers:
/// `(category, sem, is_sentence, lexicon_order, sense_rank)`.
type ParseRow = (String, String, bool, u32, u32);

/// Render a parse forest (human table or JSON). Exits non-zero on an empty forest —
/// "no felicitous parse" is fail-closed, like the gate.
fn print_parse_forest(sentence: &str, parses: &[ParseRow], json_output: bool) {
    if json_output {
        let arr: Vec<serde_json::Value> = parses
            .iter()
            .map(|(cat, sem, is_s, lo, sr)| {
                serde_json::json!({
                    "category": cat, "sem": sem, "is_sentence": is_s,
                    "lexicon_order": lo, "sense_rank": sr,
                })
            })
            .collect();
        let report = serde_json::json!({
            "status": if parses.is_empty() { "error" } else { "ok" },
            "sentence": sentence,
            "parses": arr,
        });
        let out = serde_json::to_string(&report).unwrap();
        if parses.is_empty() {
            eprintln!("{out}");
        } else {
            println!("{out}");
        }
    } else if parses.is_empty() {
        eprintln!("No felicitous parse for: {sentence:?}");
    } else {
        println!(
            "{} parse{} for {sentence:?}:",
            parses.len(),
            if parses.len() == 1 { "" } else { "s" }
        );
        for (i, (cat, sem, is_s, lo, sr)) in parses.iter().enumerate() {
            let tag = if *is_s { "S" } else { "·" };
            println!("  [{i}] {tag} rank=({lo},{sr})  {cat}");
            println!("      {sem}");
        }
    }
    if parses.is_empty() {
        std::process::exit(1);
    }
}

/// Local `lexicon parse`: build the `Parser` over the bootstrap chain plus any
/// `--file` domain layers (loaded as a chain), then parse `sentence` (optionally
/// scoped). The kernel is the parse oracle; this is the offline sibling of the
/// `ParseSentence` RPC.
fn cmd_lexicon_parse(
    sentence: &str,
    scope: &[String],
    profile: Option<&str>,
    files: &[String],
    json_output: bool,
) {
    use eigenius_kernel::dcg::{is_ctor, pretty_term, resolve_lexicon_profile, Parser};
    use eigenius_kernel::nbe::{env::Rho, eval::eval, readback::readback_val};

    let ctx = match bootstrap::bootstrap() {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("Bootstrap failed: {e}");
            std::process::exit(1);
        }
    };

    // Chain-load the domain files (each compiles against the layer below it).
    let mut layer = Arc::clone(ctx.head());
    for (idx, file) in files.iter().enumerate() {
        let resources = load_resources_against_layer(file, &layer);
        let mut builder =
            LayerBuilder::new(&format!("parse-domain-{idx}"), Some(Arc::clone(&layer)));
        for resource in resources {
            if let Err(e) = builder.add_resource(resource) {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
        layer = Arc::new(builder.build(eigenius_kernel::layer::LayerStorage::in_memory()));
    }

    if !scope.is_empty() && profile.is_some() {
        eprintln!("--scope and --profile are mutually exclusive");
        std::process::exit(1);
    }
    let scope_iris: Option<Vec<Iri>> = if !scope.is_empty() {
        Some(
            scope
                .iter()
                .map(|s| {
                    Iri::parse(s).unwrap_or_else(|e| {
                        eprintln!("invalid scope IRI {s:?}: {e:?}");
                        std::process::exit(1);
                    })
                })
                .collect(),
        )
    } else if let Some(p) = profile {
        let piri = Iri::parse(p).unwrap_or_else(|e| {
            eprintln!("invalid profile IRI {p:?}: {e:?}");
            std::process::exit(1);
        });
        Some(resolve_lexicon_profile(&layer, &piri).unwrap_or_else(|| {
            eprintln!("lexicon profile {p} not found in the chain");
            std::process::exit(1);
        }))
    } else {
        None
    };

    // Same parse config as the served RPC (D63/GH#97 Lever 1): cap + beam + the Morphy lemmatizer
    // (default in-repo dict), so local `lexicon parse` matches the server. The contextual reranker
    // is wired here too under `--features use-llm` (+ ANTHROPIC_API_KEY), for parity.
    let pc = build_parse_config("references/WordNet-3.0/dict");
    let mut index = Parser::build(Arc::clone(&layer));
    if let Some(n) = pc.sense_cap {
        index = index.with_sense_cap(n);
    }
    if let Some(m) = pc.cell_beam {
        index = index.with_cell_beam(m);
    }
    #[cfg(feature = "use-llm")]
    if pc.use_ranker {
        if let Some(r) = eigenius_kernel::dcg::AnthropicSenseRanker::from_env() {
            index = index.with_sense_ranker(Box::new(r));
        }
    }
    let forest = index.parse_scoped(sentence, &*pc.lemmatizer, scope_iris.as_deref());
    let rows: Vec<ParseRow> = forest
        .iter()
        .map(|item| {
            let sem = match eval(item.sem(), &Rho::Nil) {
                Ok(v) => pretty_term(&readback_val(0, &v)),
                Err(_) => pretty_term(item.sem()),
            };
            (
                pretty_term(item.cat()),
                sem,
                is_ctor(item.cat(), "cat_s").is_some(),
                item.cost().lexicon_order,
                item.cost().sense_rank,
            )
        })
        .collect();
    print_parse_forest(sentence, &rows, json_output);
}

/// Remote `lexicon parse`: call the kernel's `ParseSentence` RPC over the committed
/// chain. The kernel builds the (lazy) `Parser` server-side and returns the forest.
async fn remote_parse(
    endpoint: &str,
    sentence: &str,
    scope: &[String],
    profile: Option<&str>,
    json_output: bool,
) {
    let mut client = connect_client(endpoint).await;
    let request = eigenius_kernel::server::proto::ParseSentenceRequest {
        sentence: sentence.to_string(),
        scope: scope.to_vec(),
        profile: profile.unwrap_or("").to_string(),
        at_layer: String::new(),
        branch: String::new(),
    };
    match client.parse_sentence(request).await {
        Ok(response) => {
            let rows: Vec<ParseRow> = response
                .into_inner()
                .parses
                .into_iter()
                .map(|p| {
                    (
                        p.category,
                        p.sem,
                        p.is_sentence,
                        p.lexicon_order,
                        p.sense_rank,
                    )
                })
                .collect();
            print_parse_forest(sentence, &rows, json_output);
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

/// Like `load_resources_from_file`, but ESL is compiled AGAINST `layer` so
/// references to constructors / classes declared in earlier (parent) layers
/// resolve — the chain-load path the `lexicon gate` subcommand needs.
fn load_resources_against_layer(
    file: &str,
    layer: &eigenius_kernel::layer::Layer,
) -> Vec<eigenius_kernel::ontology::resource::Resource> {
    let data = std::fs::read_to_string(file).unwrap_or_else(|e| {
        eprintln!("Failed to read file: {e}");
        std::process::exit(1);
    });
    if file.ends_with(".esl") {
        eigenius_kernel::esl::compile_against_layer(&data, layer).unwrap_or_else(|errors| {
            for e in &errors {
                eprintln!("{file}: {e}");
            }
            std::process::exit(1);
        })
    } else {
        eigon_json::parse_document(&data).unwrap_or_else(|e| {
            eprintln!("Failed to parse {file}: {e}");
            std::process::exit(1);
        })
    }
}

/// Load resources from a file, auto-detecting ESL (.esl) vs Eigon-JSON.
fn load_resources_from_file(file: &str) -> Vec<eigenius_kernel::ontology::resource::Resource> {
    let data = std::fs::read_to_string(file).unwrap_or_else(|e| {
        eprintln!("Failed to read file: {e}");
        std::process::exit(1);
    });

    if file.ends_with(".esl") {
        eigenius_kernel::esl::compile(&data).unwrap_or_else(|errors| {
            for e in &errors {
                eprintln!("{file}: {e}");
            }
            std::process::exit(1);
        })
    } else {
        eigon_json::parse_document(&data).unwrap_or_else(|e| {
            eprintln!("Failed to parse {file}: {e}");
            std::process::exit(1);
        })
    }
}

fn cmd_reflect(file: &str, json_output: bool) {
    let (mut ctx, backend) = match bootstrap_local() {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("Bootstrap failed: {e}");
            std::process::exit(1);
        }
    };

    let resources = load_resources_from_file(file);
    let count = resources.len();

    if resources.is_empty() {
        eprintln!("No trace resources found in file");
        std::process::exit(1);
    }

    let trace_iri = resources[0]
        .id()
        .map(|i| i.as_str().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    for resource in resources {
        if let Err(e) = ctx.add_resource(resource) {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }

    if let Err(e) = commit_and_advance(&mut ctx, backend.as_ref(), "reflect") {
        eprintln!("Commit failed: {e}");
        std::process::exit(1);
    }

    if json_output {
        println!("{{\"success\":true,\"trace_iri\":\"{trace_iri}\",\"resource_count\":{count}}}");
    } else {
        println!("Recorded {count} trace resource(s). Trace IRI: {trace_iri}");
    }
}

fn cmd_db(command: DbCommands) {
    use eigenius_kernel::storage::ResourceBackend;

    match command {
        DbCommands::Stats { path } => {
            let store = eigenius_storage_rocksdb::RocksStore::open(std::path::Path::new(&path))
                .unwrap_or_else(|e| {
                    eprintln!("Failed to open database: {e}");
                    std::process::exit(1);
                });

            // Phase 14a: layers are enumerated via the in-memory
            // topology built from the backend's `topo:<id>` entries.
            let topology = PersistentBackend::load_topology(&store).unwrap_or_else(|e| {
                eprintln!("Failed to load topology: {e}");
                std::process::exit(1);
            });

            println!("Database: {path}");
            println!("Layers: {}", topology.layer_count());

            let mut total_resources = 0;
            for handle in topology.iter_layers() {
                let iris = ResourceBackend::list_layer_iris(&store, &handle.id).unwrap_or_default();
                total_resources += iris.len();
                println!("  Layer {}: {} resources", handle.id, iris.len());
            }
            println!("Total resources: {total_resources}");

            // Phase 14g: branches replace the single-head pointer.
            // List all known branches and their heads.
            match PersistentBackend::list_branches(&store) {
                Ok(branches) if branches.is_empty() => println!("Branches: (none)"),
                Ok(branches) => {
                    println!("Branches:");
                    for (name, head) in branches {
                        println!("  {name}: {head}");
                    }
                }
                Err(e) => println!("Branches: error ({e})"),
            }
        }
        DbCommands::Compact { path } => {
            let store = eigenius_storage_rocksdb::RocksStore::open(std::path::Path::new(&path))
                .unwrap_or_else(|e| {
                    eprintln!("Failed to open database: {e}");
                    std::process::exit(1);
                });
            store.compact();
            println!("Compaction complete.");
        }
        DbCommands::Export {
            db_path,
            output_path,
        } => {
            let store = eigenius_storage_rocksdb::RocksStore::open(std::path::Path::new(&db_path))
                .unwrap_or_else(|e| {
                    eprintln!("Failed to open database: {e}");
                    std::process::exit(1);
                });

            let topology = PersistentBackend::load_topology(&store).unwrap_or_else(|e| {
                eprintln!("Failed to load topology: {e}");
                std::process::exit(1);
            });

            std::fs::create_dir_all(&output_path).unwrap_or_else(|e| {
                eprintln!("Failed to create output directory: {e}");
                std::process::exit(1);
            });

            // Export each layer's own content (post-D41 the backend
            // doesn't materialise `Arc<Layer>` itself; we read the IRI
            // set + per-resource bodies directly via `ResourceBackend`).
            for handle in topology.iter_layers() {
                let iris =
                    ResourceBackend::list_layer_iris(&store, &handle.id).unwrap_or_else(|e| {
                        eprintln!("Failed to list IRIs for layer {}: {e}", handle.id);
                        std::process::exit(1);
                    });
                let resources: Vec<serde_json::Value> = iris
                    .iter()
                    .filter_map(|iri| ResourceBackend::load_resource(&store, &handle.id, iri))
                    .map(|r| eigon_json::serialize_resource(&r))
                    .collect();

                let json = serde_json::to_string_pretty(&resources).unwrap();
                let file_path =
                    std::path::Path::new(&output_path).join(format!("{}.json", handle.id));
                std::fs::write(&file_path, json).unwrap_or_else(|e| {
                    eprintln!("Failed to write {}: {e}", file_path.display());
                    std::process::exit(1);
                });
                println!(
                    "Exported layer {} ({} resources) → {}",
                    handle.id,
                    iris.len(),
                    file_path.display()
                );
            }
        }
        DbCommands::Consolidate { .. } => {
            eprintln!(
                "'db consolidate' requires --endpoint — consolidation must run against the live \
                 kernel server holding the same DB so the branch lock serialises with concurrent \
                 commits"
            );
            std::process::exit(1);
        }
        DbCommands::Merge { .. } => {
            eprintln!(
                "'db merge' requires --endpoint — merge resolution must run against the live \
                 kernel server so the branch lock serialises with concurrent commits"
            );
            std::process::exit(1);
        }
    }
}

/// Build the `ParseSentence` [`ParseConfig`] (D63/GH#97 Lever 1): load WordNet's Morphy from
/// `morphy_dict` (falling back to the no-op `Identity` lemmatizer if it can't be loaded), keep the
/// cap+beam defaults (the full-lexicon OOM defense), and enable the contextual LLM reranker iff the
/// binary was built `--features use-llm` (the kernel still requires `ANTHROPIC_API_KEY` at runtime).
fn build_parse_config(morphy_dict: &str) -> eigenius_kernel::server::ParseConfig {
    use eigenius_kernel::dcg::{Identity, Lemmatizer};
    let lemmatizer: std::sync::Arc<dyn Lemmatizer + Send + Sync> =
        match eigenius_wordnet::lemmatizer::MorphyLemmatizer::load(std::path::Path::new(
            morphy_dict,
        )) {
            Ok(m) => {
                eprintln!("Parse lemmatizer: Morphy ({morphy_dict})");
                std::sync::Arc::new(m)
            }
            Err(e) => {
                eprintln!(
                    "Parse lemmatizer: Identity (no-op) — Morphy dict {morphy_dict} not loadable: {e}"
                );
                std::sync::Arc::new(Identity)
            }
        };
    eigenius_kernel::server::ParseConfig {
        lemmatizer,
        use_ranker: cfg!(feature = "use-llm"),
        ..Default::default()
    }
}

async fn cmd_serve(
    port: u16,
    orchestrator: Option<&str>,
    db: Option<&str>,
    cache_budget: u64,
    morphy_dict: &str,
) {
    // Set the process-wide resource-cache budget before any persistent storage is
    // constructed (set-once; D23 §5.3). Bounds resident memory; cold reads page from
    // the backend on demand.
    eigenius_kernel::layer::set_cache_budget(cache_budget);
    println!("Resource-cache budget: {cache_budget} entries");

    let backend: Option<std::sync::Arc<dyn eigenius_kernel::storage::PersistentBackend>> = match db
    {
        Some(path) => {
            match eigenius_storage_rocksdb::RocksStore::open(std::path::Path::new(path)) {
                Ok(store) => {
                    println!("Opened persistent backend at {path}");
                    Some(std::sync::Arc::new(store))
                }
                Err(e) => {
                    eprintln!("Failed to open --db {path}: {e}");
                    std::process::exit(1);
                }
            }
        }
        None => None,
    };

    // 20a.4 + D39 Phase 8 + D52 Phase 5: link in-process institutions
    // the kernel binary ships. `start_server` registers each before
    // rebuilding the institution index, so AutoOnLoad QueryClasses
    // declared on the bootstrapped chain dispatch into the matching
    // Rust impl as a direct function call (per D28 §2.3 / §10.2 for
    // Lean, D39 §4.3 / D14 for Reasoning, D52 §6 for Statistics).
    let in_process_institutions: Vec<
        std::sync::Arc<dyn eigenius_kernel::institution::runtime::Institution>,
    > = vec![
        eigenius_lean::LeanInstitution::arc(),
        eigenius_reasoning::ReasoningInstitution::arc(),
        eigenius_statistics::StatisticsInstitution::arc(),
    ];

    // D43 §5.2 — load eigenius.toml's `[embedder]` section and
    // construct any built-in embedders it names. Empty config
    // means no embedders, no sweep coordinator; vector retrieval
    // queries return errors (or no-ops) but the rest of the kernel
    // works. `Loader::load` is the layered defaults→file→env→
    // overrides path; failures here are configuration errors and
    // the right move is to refuse to start (vs. silently dropping
    // user-declared embedders).
    let cfg = match eigenius_config::Loader::new().load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("config load failed: {e}");
            std::process::exit(1);
        }
    };
    let embedder_cfg = build_embedder_startup(&cfg.embedder).unwrap_or_else(|e| {
        eprintln!("[embedder] config error: {e}");
        std::process::exit(1);
    });

    let parse_config = build_parse_config(morphy_dict);

    if let Err(e) = eigenius_kernel::server::start_server(
        port,
        orchestrator,
        backend,
        in_process_institutions,
        embedder_cfg,
        parse_config,
    )
    .await
    {
        eprintln!("Server error: {e}");
        std::process::exit(1);
    }
}

/// Build the kernel's [`EmbedderStartupConfig`] from a loaded
/// [`eigenius_config::EmbedderConfig`]. Each entry in
/// `cfg.enabled` names a built-in embedder kind; unknown names
/// surface as an error so a typo in the config doesn't silently
/// produce a service that lacks the embedders the operator asked
/// for. Build-time device support is checked at the same time:
/// `device = "cuda"` against a CPU-only binary errors immediately
/// with a fix hint.
///
/// Currently supported kinds: `"bge-small-en-v1.5"` — BGE-small
/// via Candle (D43 §5.4 / [eigenius-embedder-candle]).
fn build_embedder_startup(
    cfg: &eigenius_config::EmbedderConfig,
) -> Result<eigenius_kernel::server::EmbedderStartupConfig, String> {
    use eigenius_config::DeviceSelection;
    // Reject `device = "cuda"` / `"metal"` when the binary lacks
    // the corresponding feature, before the embedder constructor's
    // `Device::new_cuda` call fails confusingly deep inside Candle.
    match cfg.device {
        DeviceSelection::Cuda if !cfg!(feature = "cuda") => {
            return Err("device = \"cuda\" requires building eigenius-cli with \
                 `--features cuda` (or `just build-gpu`)"
                .into());
        }
        DeviceSelection::Metal if !cfg!(feature = "metal") => {
            return Err(
                "device = \"metal\" requires building eigenius-cli with `--features metal`".into(),
            );
        }
        _ => {}
    }

    let mut embedders: Vec<std::sync::Arc<dyn eigenius_kernel::program::embedder::Embedder>> =
        Vec::new();
    for kind in &cfg.enabled {
        match kind.as_str() {
            "bge-small-en-v1.5" | "bge-small" => {
                let e = eigenius_embedder_candle::CandleEmbedder::new_bge_small()
                    .map_err(|e| format!("loading bge-small embedder: {e}"))?;
                embedders.push(std::sync::Arc::new(e));
            }
            other => {
                return Err(format!(
                    "unknown embedder kind {other:?} in [embedder].enabled — \
                     supported: [\"bge-small-en-v1.5\"]"
                ));
            }
        }
    }
    Ok(eigenius_kernel::server::EmbedderStartupConfig {
        embedders,
        batch_size: cfg.batch_size,
        fail_fast_on_missing_model: cfg.fail_fast_on_missing_model,
    })
}

// --- Remote mode (gRPC client) ---

/// Read a file, compiling ESL to Eigon-JSON if needed. Returns JSON bytes.
fn read_as_json(file: &str) -> Vec<u8> {
    if file.ends_with(".esl") {
        let source = std::fs::read_to_string(file).unwrap_or_else(|e| {
            eprintln!("Failed to read {file}: {e}");
            std::process::exit(1);
        });
        let resources = eigenius_kernel::esl::compile(&source).unwrap_or_else(|errors| {
            for e in &errors {
                eprintln!("{file}: {e}");
            }
            std::process::exit(1);
        });
        let json_values: Vec<serde_json::Value> = resources
            .iter()
            .map(eigon_json::serialize_resource)
            .collect();
        serde_json::to_vec(&json_values).unwrap()
    } else {
        std::fs::read(file).unwrap_or_else(|e| {
            eprintln!("Failed to read {file}: {e}");
            std::process::exit(1);
        })
    }
}

fn content_type_for_file(file: &str) -> String {
    if file.ends_with(".esl") {
        "application/esl".to_string()
    } else if file.ends_with(".cbor") {
        "application/cbor".to_string()
    } else {
        "application/eigon+json".to_string()
    }
}

use eigenius_kernel::server::proto::eigenius_kernel_client::EigeniusKernelClient;

pub(crate) async fn connect_client(
    endpoint: &str,
) -> EigeniusKernelClient<tonic::transport::Channel> {
    let channel = tonic::transport::Endpoint::from_shared(endpoint.to_string())
        .unwrap_or_else(|e| {
            eprintln!("Invalid endpoint '{endpoint}': {e}");
            std::process::exit(1);
        })
        .connect()
        .await
        .unwrap_or_else(|e| {
            eprintln!("Failed to connect to {endpoint}: {e}");
            std::process::exit(1);
        });
    // Raise gRPC message size limits to 128 MB to accommodate large
    // layer-load batches and query result sets (which can be multiple MB).
    EigeniusKernelClient::new(channel)
        .max_decoding_message_size(128 * 1024 * 1024)
        .max_encoding_message_size(128 * 1024 * 1024)
}

async fn remote_inspect(
    endpoint: &str,
    iri_str: &str,
    at_layer: Option<&str>,
    branch: Option<&str>,
    json_output: bool,
) {
    let mut client = connect_client(endpoint).await;

    let request = eigenius_kernel::server::proto::InspectRequest {
        at_layer: at_layer.unwrap_or("").to_string(),
        iri: iri_str.to_string(),
        branch: branch.unwrap_or("").to_string(),
    };

    match client.inspect(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            if resp.found {
                let resource =
                    eigenius_kernel::ontology::eigon_cbor::parse_resource(&resp.resource)
                        .unwrap_or_else(|e| {
                            eprintln!("Failed to parse response: {e}");
                            std::process::exit(1);
                        });
                let json = eigon_json::serialize_resource(&resource);
                if json_output {
                    println!("{}", serde_json::to_string(&json).unwrap());
                } else {
                    println!("{}", serde_json::to_string_pretty(&json).unwrap());
                }
            } else {
                eprintln!("Resource not found: {iri_str}");
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

async fn remote_query(
    endpoint: &str,
    eigenql: &str,
    at_layer: Option<&str>,
    branch: Option<&str>,
    json_output: bool,
) {
    let mut client = connect_client(endpoint).await;

    let request = eigenius_kernel::server::proto::QueryRequest {
        at_layer: at_layer.unwrap_or("").to_string(),
        eigenql: eigenql.to_string(),
        branch: branch.unwrap_or("").to_string(),
    };

    match client.query(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            if !resp.success {
                eprintln!("Query failed: {}", resp.error);
                std::process::exit(1);
            }
            let document = eigenius_kernel::ontology::eigon_cbor::parse_document(&resp.document)
                .unwrap_or_else(|e| {
                    eprintln!("Failed to parse result document: {e}");
                    std::process::exit(1);
                });
            if json_output {
                let arr: Vec<serde_json::Value> = document
                    .iter()
                    .map(eigon_json::serialize_resource)
                    .collect();
                println!("{}", serde_json::to_string(&arr).unwrap());
            } else {
                println!("{} resource(s) in result document:", document.len());
                for r in &document {
                    let json = eigon_json::serialize_resource(r);
                    println!("{}", serde_json::to_string_pretty(&json).unwrap());
                }
            }
            // Only meaningful when the query committed something via
            // FIBER ... INTO. A query without INTO clauses reports
            // branch_advanced=false because nothing was committed, not
            // because of a cache hit — so we gate the note on
            // `output_resource_iris` being non-empty.
            if !resp.branch_advanced && !resp.output_resource_iris.is_empty() {
                eprintln!(
                    "Note: FIBER INTO results reused from anchored-commit cache (branch unchanged)."
                );
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

async fn remote_run(
    endpoint: &str,
    program_file: &str,
    input_file: &str,
    branch: Option<&str>,
    json_output: bool,
) {
    let mut client = connect_client(endpoint).await;

    // Compile ESL files client-side since program and input may have different formats
    let program_data = read_as_json(program_file);
    let input_data = read_as_json(input_file);

    let request = eigenius_kernel::server::proto::RunProgramRequest {
        program: program_data,
        input: input_data,
        content_type: "application/eigon+json".to_string(),
        branch: branch.unwrap_or("").to_string(),
    };

    match client.run_program(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            if resp.success {
                let resource =
                    eigenius_kernel::ontology::eigon_cbor::parse_resource_lenient(&resp.output)
                        .unwrap_or_else(|e| {
                            eprintln!("Failed to parse output: {e}");
                            std::process::exit(1);
                        });
                let json = eigon_json::serialize_resource(&resource);
                if json_output {
                    println!("{}", serde_json::to_string(&json).unwrap());
                } else {
                    println!("{}", serde_json::to_string_pretty(&json).unwrap());
                }
                if !resp.branch_advanced {
                    // Anchored-commit cache hit (D33 §6) — the trace
                    // layer for this run already exists canonically
                    // elsewhere in the DAG; the branch ref did not
                    // move. Note on stderr so the data output stays
                    // pipe-clean.
                    eprintln!(
                        "Note: trace layer reused from anchored-commit cache (branch unchanged)."
                    );
                }
            } else {
                eprintln!("Program execution failed:");
                for err in &resp.errors {
                    eprintln!("  {}: {}", err.rule, err.message);
                }
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

async fn remote_load(
    endpoint: &str,
    file: &str,
    branch: Option<&str>,
    policy: Option<eigenius_kernel::server::proto::CommitPolicy>,
    explicit_tombstones: &[String],
    json_output: bool,
) {
    let mut client = connect_client(endpoint).await;

    let data = std::fs::read(file).unwrap_or_else(|e| {
        eprintln!("Failed to read file: {e}");
        std::process::exit(1);
    });

    let content_type = content_type_for_file(file);
    let request = eigenius_kernel::server::proto::LoadRequest {
        resources: data,
        content_type,
        auto_commit: true,
        branch: branch.unwrap_or("").to_string(),
        policy,
        explicit_tombstones: explicit_tombstones.to_vec(),
    };

    match client.load(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            // Find the user-layer outcome to extract cascade info; the
            // orchestrator's `committed_layers` may also carry an audit
            // Sibling (`verdict_provenance`) and an `institution_classes`
            // follow-up. The user layer's role enum value is `1`
            // (LAYER_ROLE_USER per proto/eigenius.proto).
            let user_layer = resp
                .committed_layers
                .iter()
                .find(|l| l.role == eigenius_kernel::server::proto::LayerRole::User as i32);
            let cascade_tombs = user_layer.map(|l| l.cascade_tombstones.len()).unwrap_or(0);
            let cascade_iters = user_layer.map(|l| l.cascade_iterations).unwrap_or(0);
            if resp.success {
                if json_output {
                    println!(
                        "{{\"success\":true,\"resource_count\":{},\"layer_id\":\"{}\",\"branch\":\"{}\",\"branch_advanced\":{},\"cascade_tombstones\":{cascade_tombs},\"cascade_iterations\":{cascade_iters},\"total_violations\":0}}",
                        resp.resource_count,
                        resp.layer_id,
                        resp.branch,
                        resp.branch_advanced,
                    );
                } else if resp.branch_advanced {
                    println!(
                        "Loaded {} resource(s) into branch {}. Layer: {}",
                        resp.resource_count, resp.branch, resp.layer_id
                    );
                    if cascade_tombs > 0 {
                        println!(
                            "Cascade tombstoned {cascade_tombs} IRI(s) in {cascade_iters} iteration(s)."
                        );
                    }
                } else {
                    // Anchored-commit cache hit at a different
                    // position — the canonical layer for this content
                    // already lives elsewhere in the DAG; the branch
                    // ref did not move.
                    println!(
                        "Cached: {} resource(s) already canonical at layer {} (branch {} unchanged)",
                        resp.resource_count, resp.layer_id, resp.branch
                    );
                }
            } else {
                if json_output {
                    println!(
                        "{{\"success\":false,\"total_violations\":{},\"errors\":{}}}",
                        resp.total_violations,
                        resp.errors.len()
                    );
                } else {
                    eprintln!("Load failed:");
                    for err in &resp.errors {
                        eprintln!("  {}: {}", err.rule, err.message);
                    }
                    if (resp.total_violations as usize) > resp.errors.len() {
                        eprintln!(
                            "(Showing first {} of {} total violations.)",
                            resp.errors.len(),
                            resp.total_violations
                        );
                    }
                }
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

async fn remote_reflect(endpoint: &str, file: &str, json_output: bool) {
    let mut client = connect_client(endpoint).await;

    let data = read_as_json(file);

    let request = eigenius_kernel::server::proto::ReflectRequest {
        trace: data,
        content_type: "application/eigon+json".to_string(),
        branch: String::new(),
    };

    match client.reflect(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            if resp.success {
                if json_output {
                    println!(
                        "{{\"success\":true,\"trace_iri\":\"{}\",\"branch_advanced\":{}}}",
                        resp.trace_iri, resp.branch_advanced
                    );
                } else if resp.branch_advanced {
                    println!("Recorded trace: {}", resp.trace_iri);
                } else {
                    println!(
                        "Trace already canonical: {} (anchored-commit cache hit, branch unchanged)",
                        resp.trace_iri
                    );
                }
            } else {
                eprintln!("Reflect failed");
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

async fn remote_list_institutions(endpoint: &str, json_output: bool) {
    let mut client = connect_client(endpoint).await;

    match client
        .list_institutions(eigenius_kernel::server::proto::ListInstitutionsRequest {
            at_layer: String::new(),
        })
        .await
    {
        Ok(response) => {
            let resp = response.into_inner();
            if json_output {
                let json: Vec<serde_json::Value> = resp
                    .institutions
                    .iter()
                    .map(|i| {
                        serde_json::json!({
                            "iri": i.iri,
                            "name": i.name,
                            "query_types": i.query_types,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string(&json).unwrap());
            } else if resp.institutions.is_empty() {
                println!("No institutions registered.");
            } else {
                println!("Registered institutions:");
                for inst in &resp.institutions {
                    println!("  {} ({})", inst.name, inst.iri);
                    for qt in &inst.query_types {
                        println!("    query:    {qt}");
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

async fn remote_get_schema(endpoint: &str, class_iri: &str, _json_output: bool) {
    let mut client = connect_client(endpoint).await;

    match client
        .get_schema(eigenius_kernel::server::proto::GetSchemaRequest {
            at_layer: String::new(),
            class_iri: class_iri.to_string(),
        })
        .await
    {
        Ok(response) => {
            let resp = response.into_inner();
            if resp.success {
                println!("{}", resp.json_schema);
            } else {
                eprintln!("Schema generation failed: {}", resp.error);
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

// --- Capability subcommand ---

async fn remote_tasks(endpoint: &str, command: TaskCommands, json: bool) {
    match command {
        TaskCommands::List => remote_tasks_list(endpoint, json).await,
        TaskCommands::Status { task_id } => remote_task_status(endpoint, &task_id, json).await,
        TaskCommands::Cancel { task_id } => remote_task_cancel(endpoint, &task_id, json).await,
    }
}

async fn remote_tasks_list(endpoint: &str, json_output: bool) {
    let mut client = connect_client(endpoint).await;
    let request = eigenius_kernel::server::proto::ListTasksRequest {};
    match client.list_tasks(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            if json_output {
                let items: Vec<serde_json::Value> = resp
                    .tasks
                    .iter()
                    .map(|t| {
                        serde_json::json!({
                            "task_id": t.task_id,
                            "program_iri": t.program_iri,
                            "status": t.status,
                            "layer_head": t.layer_head,
                            "step_seq": t.step_seq,
                            "result_layer_head": t.result_layer_head,
                            "created_at_ms": t.created_at_ms,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&items).unwrap());
            } else if resp.tasks.is_empty() {
                println!("No tasks.");
            } else {
                println!("{:<36}  {:<12}  PROGRAM", "TASK ID", "STATUS");
                for t in &resp.tasks {
                    println!("{:<36}  {:<12}  {}", t.task_id, t.status, t.program_iri);
                }
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

async fn remote_task_status(endpoint: &str, task_id: &str, json_output: bool) {
    let mut client = connect_client(endpoint).await;
    let request = eigenius_kernel::server::proto::GetTaskStatusRequest {
        task_id: task_id.to_string(),
    };
    match client.get_task_status(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            if !resp.found {
                eprintln!("Task not found: {task_id}");
                std::process::exit(1);
            }
            let t = resp.task.unwrap();
            if json_output {
                let j = serde_json::json!({
                    "task_id": t.task_id,
                    "session_id": t.session_id,
                    "program_iri": t.program_iri,
                    "input_iri": t.input_iri,
                    "status": t.status,
                    "layer_head": t.layer_head,
                    "step_seq": t.step_seq,
                    "latest_trace_seq": t.latest_trace_seq,
                    "last_checkpoint_step": t.last_checkpoint_step,
                    "result_layer_head": t.result_layer_head,
                    "created_at_ms": t.created_at_ms,
                    "updated_at_ms": t.updated_at_ms,
                    "retain_forever": t.retain_forever,
                });
                println!("{}", serde_json::to_string_pretty(&j).unwrap());
            } else {
                println!("Task:         {}", t.task_id);
                println!("Status:       {}", t.status);
                println!("Program:      {}", t.program_iri);
                println!("Input:        {}", t.input_iri);
                println!("Layer head:   {}", t.layer_head);
                println!("Step seq:     {}", t.step_seq);
                println!("Last ckpt:    {}", t.last_checkpoint_step);
                if !t.result_layer_head.is_empty() {
                    println!("Result layer: {}", t.result_layer_head);
                }
                println!(
                    "Created:      {} ms (unix epoch)\nUpdated:      {} ms",
                    t.created_at_ms, t.updated_at_ms
                );
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

async fn remote_task_cancel(endpoint: &str, task_id: &str, json_output: bool) {
    let mut client = connect_client(endpoint).await;
    let request = eigenius_kernel::server::proto::CancelTaskRequest {
        task_id: task_id.to_string(),
    };
    match client.cancel_task(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            if json_output {
                let j = serde_json::json!({
                    "success": resp.success,
                    "status": resp.status,
                    "error": resp.error,
                });
                println!("{}", serde_json::to_string_pretty(&j).unwrap());
            } else if resp.success {
                println!("Task {task_id}: {}", resp.status);
            } else {
                eprintln!("Cancel failed: {}", resp.error);
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

// --- Branch subcommand ---

async fn remote_branch(endpoint: &str, command: BranchCommands, json: bool) {
    match command {
        BranchCommands::List => remote_branch_list(endpoint, json).await,
        BranchCommands::Show { name } => remote_branch_show(endpoint, &name, json).await,
        BranchCommands::Create { name, from } => {
            remote_branch_create(endpoint, &name, &from, json).await
        }
        BranchCommands::Delete { name, force } => {
            remote_branch_delete(endpoint, &name, force, json).await
        }
    }
}

async fn remote_branch_list(endpoint: &str, json_output: bool) {
    let mut client = connect_client(endpoint).await;
    let request = eigenius_kernel::server::proto::ListBranchesRequest {};
    match client.list_branches(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            if json_output {
                let items: Vec<serde_json::Value> = resp
                    .branches
                    .iter()
                    .map(|b| serde_json::json!({"name": b.name, "head_layer": b.head_layer}))
                    .collect();
                println!("{}", serde_json::to_string_pretty(&items).unwrap());
            } else if resp.branches.is_empty() {
                println!("No branches.");
            } else {
                println!("{:<32}  HEAD", "NAME");
                for b in &resp.branches {
                    println!("{:<32}  {}", b.name, b.head_layer);
                }
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

async fn remote_branch_show(endpoint: &str, name: &str, json_output: bool) {
    let mut client = connect_client(endpoint).await;
    let request = eigenius_kernel::server::proto::GetBranchRequest {
        name: name.to_string(),
    };
    match client.get_branch(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            if json_output {
                let j = serde_json::json!({
                    "found": resp.found,
                    "name": name,
                    "head_layer": resp.head_layer,
                });
                println!("{}", serde_json::to_string_pretty(&j).unwrap());
            } else if resp.found {
                println!("{name}: {}", resp.head_layer);
            } else {
                eprintln!("Branch not found: {name}");
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

async fn remote_branch_create(endpoint: &str, name: &str, from: &str, json_output: bool) {
    let mut client = connect_client(endpoint).await;
    let request = eigenius_kernel::server::proto::CreateBranchRequest {
        name: name.to_string(),
        from_layer: from.to_string(),
    };
    match client.create_branch(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            if json_output {
                let j = serde_json::json!({
                    "success": resp.success,
                    "name": name,
                    "head_layer": resp.head_layer,
                    "error": resp.error,
                });
                println!("{}", serde_json::to_string_pretty(&j).unwrap());
            } else if resp.success {
                println!("Created branch {name} at {}", resp.head_layer);
            } else {
                eprintln!("Create failed: {}", resp.error);
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

async fn remote_branch_delete(endpoint: &str, name: &str, force: bool, json_output: bool) {
    let mut client = connect_client(endpoint).await;
    let request = eigenius_kernel::server::proto::DeleteBranchRequest {
        name: name.to_string(),
        force,
    };
    match client.delete_branch(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            if json_output {
                let j = serde_json::json!({
                    "success": resp.success,
                    "deleted": resp.deleted,
                    "name": name,
                    "previous_head": resp.previous_head,
                    "error": resp.error,
                });
                println!("{}", serde_json::to_string_pretty(&j).unwrap());
            } else if !resp.success {
                eprintln!("Delete failed: {}", resp.error);
                std::process::exit(1);
            } else if resp.deleted {
                println!("Deleted branch {name} (was at {})", resp.previous_head);
            } else {
                println!("Branch {name} did not exist");
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

/// Parse a `<from-hex>..<to-hex>` range argument into its two LayerId hex strings.
/// Lightweight — we forward the strings to the server which does
/// authoritative validation.
fn parse_consolidate_range(range: &str) -> Result<(String, String), String> {
    let mut iter = range.splitn(2, "..");
    let from = iter
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "range must be '<from-hex>..<to-hex>'".to_string())?;
    let to = iter
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "range must be '<from-hex>..<to-hex>'".to_string())?;
    Ok((from.to_string(), to.to_string()))
}

async fn remote_db_consolidate(
    endpoint: &str,
    range: &str,
    branch: &str,
    max_walk_entries: Option<u64>,
    dry_run: bool,
    preserve_history: bool,
    json_output: bool,
) {
    let (from_layer, to_layer) = match parse_consolidate_range(range) {
        Ok(parts) => parts,
        Err(e) => {
            eprintln!("Invalid range: {e}");
            std::process::exit(1);
        }
    };
    let max_walk_entries = max_walk_entries.unwrap_or(0);
    let mut client = connect_client(endpoint).await;

    if dry_run {
        let req = eigenius_kernel::server::proto::EstimateConsolidationRequest {
            branch: branch.to_string(),
            from_layer,
            to_layer,
            max_walk_entries,
            trace_pin_policy: String::new(),
            preserve_history,
        };
        match client.estimate_consolidation(req).await {
            Ok(response) => {
                let resp = response.into_inner();
                if json_output {
                    let j = serde_json::json!({
                        "success": resp.success,
                        "dry_run": true,
                        "predicted_consolidated_layer": resp.predicted_consolidated_layer,
                        "collapsed_layer_count": resp.collapsed_layer_count,
                        "predicted_walk_entries": resp.predicted_walk_entries,
                        "actual_walk_entries": resp.actual_walk_entries,
                        "error_kind": resp.error_kind,
                        "error": resp.error,
                        "error_layer": resp.error_layer,
                        "error_count": resp.error_count,
                    });
                    println!("{}", serde_json::to_string_pretty(&j).unwrap());
                } else if resp.success {
                    println!(
                        "Dry-run: would consolidate {} layers on {branch}",
                        resp.collapsed_layer_count
                    );
                    println!(
                        "  Predicted walk entries: {} (actual {} after dedup)",
                        resp.predicted_walk_entries, resp.actual_walk_entries
                    );
                    println!(
                        "  Predicted consolidated layer: {}",
                        resp.predicted_consolidated_layer
                    );
                } else {
                    eprintln!("Consolidation refused: {}", resp.error);
                    std::process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("gRPC error: {e}");
                std::process::exit(1);
            }
        }
    } else {
        let req = eigenius_kernel::server::proto::ConsolidateChainRequest {
            branch: branch.to_string(),
            from_layer,
            to_layer,
            max_walk_entries,
            trace_pin_policy: String::new(),
            preserve_history,
        };
        match client.consolidate_chain(req).await {
            Ok(response) => {
                let resp = response.into_inner();
                if json_output {
                    let j = serde_json::json!({
                        "success": resp.success,
                        "consolidated_layer": resp.consolidated_layer,
                        "collapsed_layer_count": resp.collapsed_layer_count,
                        "head_advanced": resp.head_advanced,
                        "error_kind": resp.error_kind,
                        "error": resp.error,
                        "error_layer": resp.error_layer,
                        "error_count": resp.error_count,
                    });
                    println!("{}", serde_json::to_string_pretty(&j).unwrap());
                } else if resp.success {
                    if resp.head_advanced {
                        println!(
                            "Consolidated {} layers on {branch}; branch advanced to {}",
                            resp.collapsed_layer_count, resp.consolidated_layer
                        );
                    } else {
                        // Below-head: redirect installed; branch ref unchanged.
                        println!(
                            "Consolidated {} layers below the head of {branch}; \
                             redirect installed at the range tip → {}{}",
                            resp.collapsed_layer_count,
                            resp.consolidated_layer,
                            if preserve_history {
                                " (preserve_history mode — source range stays alive)"
                            } else {
                                ""
                            }
                        );
                    }
                } else {
                    eprintln!("Consolidation refused: {}", resp.error);
                    std::process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("gRPC error: {e}");
                std::process::exit(1);
            }
        }
    }
}

/// Read and parse a JSON file of merge resolutions into the wire
/// `MergeResolutionWire` shape. The file is an array of objects, one
/// per resolution; each carries `conflict_id`, `kind`, and the
/// variant-specific fields. Schema:
///
/// ```text
/// [
///   { "conflict_id": "...",
///     "kind": "witness",
///     "comorphism_iri": "..." },
///
///   { "conflict_id": "...",
///     "kind": "rename",
///     "side": "a"|"b",
///     "old_iri": "...",
///     "new_iri": "..." },
///
///   { "conflict_id": "...",
///     "kind": "schema_quotient",
///     "quotient": "keep_both"|"keep_one"|"keep_neither",
///     "winner": "a"|"b"   // only for keep_one
///   },
///
///   { "conflict_id": "...",
///     "kind": "restructure",
///     "affected_class": "urn:project:Dog",
///     "new_parent": "urn:project:Animal",
///     // Inline Eigon-JSON Class definition; omit when
///     // `new_parent` already exists in the chain.
///     "new_parent_def": {
///       "@id": "urn:project:Animal",
///       "urn:eigenius:core:is_a": ["urn:eigenius:core:Class"],
///       "urn:eigenius:core:short_name": "Animal",
///       "urn:eigenius:core:description": "..."
///     },
///     "classes_under_new": ["urn:project:Mammal", "urn:project:Reptile"],
///     "affected_class_under_new": true
///   }
/// ]
/// ```
///
/// Returns the parsed wire vec on success or a human-readable
/// diagnostic on shape failure.
fn parse_resolution_file(
    path: &std::path::Path,
) -> Result<Vec<eigenius_kernel::server::proto::MergeResolutionWire>, String> {
    use eigenius_kernel::server::proto;

    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let array: Vec<serde_json::Value> = serde_json::from_str(&raw)
        .map_err(|e| format!("{} is not a JSON array of resolutions: {e}", path.display()))?;

    let mut out = Vec::with_capacity(array.len());
    for (idx, entry) in array.iter().enumerate() {
        let obj = entry
            .as_object()
            .ok_or_else(|| format!("resolutions[{idx}] is not a JSON object"))?;
        let conflict_id = obj
            .get("conflict_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("resolutions[{idx}]: missing string `conflict_id`"))?
            .to_string();
        let kind = obj
            .get("kind")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("resolutions[{idx}]: missing string `kind`"))?;

        let strategy = match kind {
            "witness" => {
                let comorphism_iri = obj
                    .get("comorphism_iri")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        format!("resolutions[{idx}]: witness needs string `comorphism_iri`")
                    })?
                    .to_string();
                proto::merge_resolution_wire::Strategy::Witness(proto::WitnessStrategy {
                    comorphism_iri,
                })
            }
            "rename" => {
                let side = parse_side_str(obj.get("side"), idx)?;
                let old_iri = obj
                    .get("old_iri")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| format!("resolutions[{idx}]: rename needs string `old_iri`"))?
                    .to_string();
                let new_iri = obj
                    .get("new_iri")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| format!("resolutions[{idx}]: rename needs string `new_iri`"))?
                    .to_string();
                proto::merge_resolution_wire::Strategy::Rename(proto::RenameStrategy {
                    side: side as i32,
                    old_iri,
                    new_iri,
                })
            }
            "schema_quotient" => {
                let quotient_str =
                    obj.get("quotient")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            format!("resolutions[{idx}]: schema_quotient needs string `quotient`")
                        })?;
                let kind = match quotient_str {
                    "keep_both" => proto::MergeQuotientKind::KeepBoth,
                    "keep_one" => proto::MergeQuotientKind::KeepOne,
                    "keep_neither" => proto::MergeQuotientKind::KeepNeither,
                    other => {
                        return Err(format!("resolutions[{idx}]: unknown quotient {other:?}"));
                    }
                };
                let winner = if matches!(kind, proto::MergeQuotientKind::KeepOne) {
                    parse_side_str(obj.get("winner"), idx)?
                } else {
                    proto::MergeSide::Unspecified
                };
                proto::merge_resolution_wire::Strategy::Quotient(proto::QuotientStrategy {
                    kind: kind as i32,
                    winner: winner as i32,
                })
            }
            "restructure" => {
                let affected_class = obj
                    .get("affected_class")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        format!("resolutions[{idx}]: restructure needs string `affected_class`")
                    })?
                    .to_string();
                let new_parent = obj
                    .get("new_parent")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        format!("resolutions[{idx}]: restructure needs string `new_parent`")
                    })?
                    .to_string();
                // `new_parent_def` is an inline Eigon-JSON resource
                // when the parent is new; serialize it back to a
                // string for the wire so the kernel re-parses with
                // `eigon_json::parse_embedded`. Empty when the
                // parent already exists.
                let new_parent_def_json = match obj.get("new_parent_def") {
                    None | Some(serde_json::Value::Null) => String::new(),
                    Some(value) => serde_json::to_string(value).map_err(|e| {
                        format!("resolutions[{idx}].new_parent_def: cannot serialize as JSON: {e}")
                    })?,
                };
                let classes_under_new = match obj.get("classes_under_new") {
                    None => Vec::new(),
                    Some(serde_json::Value::Array(arr)) => arr
                        .iter()
                        .enumerate()
                        .map(|(j, v)| {
                            v.as_str().map(|s| s.to_string()).ok_or_else(|| {
                                format!(
                                    "resolutions[{idx}].classes_under_new[{j}] must be a string"
                                )
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    Some(_) => {
                        return Err(format!(
                            "resolutions[{idx}].classes_under_new must be an array of strings"
                        ));
                    }
                };
                let affected_class_under_new = obj
                    .get("affected_class_under_new")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                proto::merge_resolution_wire::Strategy::Restructure(proto::RestructureStrategy {
                    affected_class,
                    new_parent,
                    new_parent_def_json,
                    classes_under_new,
                    affected_class_under_new,
                })
            }
            other => {
                return Err(format!(
                    "resolutions[{idx}]: unknown kind {other:?}; expected witness, rename, schema_quotient, restructure"
                ));
            }
        };
        out.push(proto::MergeResolutionWire {
            conflict_id,
            strategy: Some(strategy),
        });
    }
    Ok(out)
}

fn parse_side_str(
    value: Option<&serde_json::Value>,
    idx: usize,
) -> Result<eigenius_kernel::server::proto::MergeSide, String> {
    use eigenius_kernel::server::proto::MergeSide;
    let s = value
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("resolutions[{idx}]: missing string `side` (\"a\" or \"b\")"))?;
    match s {
        "a" | "A" => Ok(MergeSide::A),
        "b" | "B" => Ok(MergeSide::B),
        other => Err(format!(
            "resolutions[{idx}]: side must be \"a\" or \"b\", got {other:?}"
        )),
    }
}

async fn remote_db_merge_preview(
    endpoint: &str,
    branch: &str,
    candidate: &str,
    resolutions_path: &std::path::Path,
    json_output: bool,
) {
    let resolutions = match parse_resolution_file(resolutions_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let mut client = connect_client(endpoint).await;
    let req = eigenius_kernel::server::proto::PreviewCascadeRequest {
        branch: branch.to_string(),
        candidate_head: candidate.to_string(),
        resolutions,
        witness_search_branches: Vec::new(),
    };
    match client.preview_cascade(req).await {
        Ok(response) => {
            let resp = response.into_inner();
            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "success": resp.success,
                        "error": resp.error,
                        "error_kind": resp.error_kind,
                        "items": resp.items.iter().map(cascade_item_to_json).collect::<Vec<_>>(),
                    }))
                    .unwrap()
                );
            } else if resp.success {
                if resp.items.is_empty() {
                    println!("No cascade items — resolutions are self-contained.");
                } else {
                    println!("{} cascade item(s):", resp.items.len());
                    for item in &resp.items {
                        println!("  {}", item.item_id);
                        print_cascade_item_body(item);
                    }
                    println!();
                    println!(
                        "Acknowledge with: eigenius db merge resolve --acknowledge <ITEM_ID> [...]"
                    );
                }
            } else {
                eprintln!("Preview failed ({}): {}", resp.error_kind, resp.error);
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

async fn remote_db_merge_resolve(
    endpoint: &str,
    branch: &str,
    candidate: &str,
    resolutions_path: &std::path::Path,
    acknowledgments: &[String],
    json_output: bool,
) {
    let resolutions = match parse_resolution_file(resolutions_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let acks: Vec<eigenius_kernel::server::proto::CascadeAckWire> = acknowledgments
        .iter()
        .map(|id| eigenius_kernel::server::proto::CascadeAckWire {
            item_id: id.clone(),
        })
        .collect();
    let mut client = connect_client(endpoint).await;
    let req = eigenius_kernel::server::proto::SubmitResolutionRequest {
        branch: branch.to_string(),
        candidate_head: candidate.to_string(),
        resolutions,
        acknowledgments: acks,
        witness_search_branches: Vec::new(),
    };
    match client.submit_resolution(req).await {
        Ok(response) => {
            let resp = response.into_inner();
            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "success": resp.success,
                        "error": resp.error,
                        "error_kind": resp.error_kind,
                        "merge_layer_id": resp.merge_layer_id,
                        "branch_tip": resp.branch_tip,
                        "missing_acknowledgments": resp.missing_acknowledgments,
                    }))
                    .unwrap()
                );
            } else if resp.success {
                println!("Merge committed on {branch}: layer {}", resp.merge_layer_id);
                if resp.branch_tip != resp.merge_layer_id {
                    println!("  branch tip: {}", resp.branch_tip);
                }
            } else {
                eprintln!("Resolution failed ({}): {}", resp.error_kind, resp.error);
                if !resp.missing_acknowledgments.is_empty() {
                    eprintln!("Missing acknowledgments — pass each with --acknowledge:");
                    for id in &resp.missing_acknowledgments {
                        eprintln!("  --acknowledge {id}");
                    }
                }
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

fn cascade_item_to_json(
    item: &eigenius_kernel::server::proto::CascadeItemWire,
) -> serde_json::Value {
    use eigenius_kernel::server::proto::cascade_item_wire::Kind;
    let mut body = serde_json::json!({ "item_id": item.item_id });
    if let Some(kind) = &item.kind {
        let body_obj = body.as_object_mut().unwrap();
        match kind {
            Kind::OrphanedReference(r) => {
                body_obj.insert("kind".to_string(), "orphaned_reference".into());
                body_obj.insert("resource".to_string(), r.resource.clone().into());
                body_obj.insert(
                    "dropped_target".to_string(),
                    r.dropped_target.clone().into(),
                );
                body_obj.insert(
                    "property_path".to_string(),
                    serde_json::Value::Array(
                        r.property_path
                            .iter()
                            .map(|s| serde_json::Value::String(s.clone()))
                            .collect(),
                    ),
                );
            }
            Kind::OrphanedTyping(t) => {
                body_obj.insert("kind".to_string(), "orphaned_typing".into());
                body_obj.insert("class".to_string(), t.class.clone().into());
                body_obj.insert(
                    "affected_resources".to_string(),
                    serde_json::Value::Array(
                        t.affected_resources
                            .iter()
                            .map(|s| serde_json::Value::String(s.clone()))
                            .collect(),
                    ),
                );
            }
            Kind::InvalidatedSignature(s) => {
                body_obj.insert("kind".to_string(), "invalidated_signature".into());
                body_obj.insert("program".to_string(), s.program.clone().into());
                body_obj.insert(
                    "signature_problem".to_string(),
                    s.signature_problem.clone().into(),
                );
            }
            Kind::InvalidatedTrace(t) => {
                body_obj.insert("kind".to_string(), "invalidated_trace".into());
                body_obj.insert("trace".to_string(), t.trace.clone().into());
                body_obj.insert("reason".to_string(), t.reason.clone().into());
            }
        }
    }
    body
}

fn print_cascade_item_body(item: &eigenius_kernel::server::proto::CascadeItemWire) {
    use eigenius_kernel::server::proto::cascade_item_wire::Kind;
    let Some(kind) = &item.kind else {
        return;
    };
    match kind {
        Kind::OrphanedReference(r) => {
            let path = if r.property_path.is_empty() {
                "<root>".to_string()
            } else {
                r.property_path.join("/")
            };
            println!(
                "    OrphanedReference: {} → {} (at {})",
                r.resource, r.dropped_target, path
            );
        }
        Kind::OrphanedTyping(t) => {
            println!(
                "    OrphanedTyping: class {} ({} affected resources)",
                t.class,
                t.affected_resources.len()
            );
            for r in &t.affected_resources {
                println!("      {r}");
            }
        }
        Kind::InvalidatedSignature(s) => {
            println!(
                "    InvalidatedSignature: program {} ({})",
                s.program, s.signature_problem
            );
        }
        Kind::InvalidatedTrace(t) => {
            println!("    InvalidatedTrace: {} ({})", t.trace, t.reason);
        }
    }
}

async fn remote_mirror(endpoint: &str, command: MirrorCommands, json: bool) {
    match command {
        MirrorCommands::Create {
            layer,
            filter,
            filter_file,
            language,
            output,
            institution_file,
        } => {
            mirror::mirror_create(
                endpoint,
                &layer,
                filter.as_deref(),
                filter_file.as_deref(),
                &language,
                &output,
                institution_file.as_deref(),
                json,
            )
            .await
        }
        MirrorCommands::Get { iri, output } => {
            mirror::mirror_get(endpoint, &iri, &output, json).await
        }
        MirrorCommands::List { language } => {
            mirror::mirror_list(endpoint, language.as_deref(), json).await
        }
        MirrorCommands::Inspect { iri } => mirror::mirror_inspect(endpoint, &iri, json).await,
    }
}

async fn remote_env(endpoint: &str, command: EnvCommands, json: bool) {
    match command {
        EnvCommands::Build {
            language,
            package_path,
            mirror,
            base_image,
            worker_source_dir,
            depot,
            r_driver,
            r_cdylib,
            r_package,
            bioc_version,
        } => {
            env::env_build(
                endpoint,
                &language,
                package_path.as_deref(),
                mirror.as_deref(),
                &base_image,
                worker_source_dir.as_deref(),
                depot.as_deref(),
                r_driver.as_deref(),
                r_cdylib.as_deref(),
                &r_package,
                bioc_version.as_deref(),
                json,
            )
            .await
        }
        EnvCommands::Create {
            language,
            handler_package,
            mirror,
            include_package,
            as_iri,
            base_image,
            image_digest,
            runtime_version,
        } => {
            env::env_create(
                endpoint,
                &language,
                &handler_package,
                &mirror,
                &include_package,
                &as_iri,
                base_image.as_deref(),
                &image_digest,
                &runtime_version,
                json,
            )
            .await
        }
        EnvCommands::List { language } => env::env_list(endpoint, language.as_deref(), json).await,
        EnvCommands::Inspect { iri } => env::env_inspect(endpoint, &iri, json).await,
    }
}

async fn remote_data(endpoint: &str, command: DataCommands, json: bool) {
    match command {
        DataCommands::Attach {
            file,
            reference,
            media_type,
            name,
        } => {
            data::data_attach(
                endpoint,
                &file,
                reference.as_deref(),
                media_type.as_deref(),
                name.as_deref(),
                json,
            )
            .await
        }
        DataCommands::List { media_type } => {
            data::data_list(endpoint, media_type.as_deref(), json).await
        }
        DataCommands::Inspect { iri } => data::data_inspect(endpoint, &iri, json).await,
        DataCommands::Verify { iri } => data::data_verify(endpoint, &iri, json).await,
        DataCommands::Validate { iri } => data::data_validate(endpoint, &iri, json).await,
        DataCommands::Provision { iri, cache_root } => {
            data::data_provision(endpoint, &iri, cache_root.as_deref(), json).await
        }
    }
}

async fn remote_script(endpoint: &str, command: ScriptCommands, json: bool) {
    match command {
        ScriptCommands::Publish {
            file,
            env,
            lang,
            entry_point,
            description,
        } => {
            scripts::script_publish(
                endpoint,
                &file,
                &env,
                lang.as_deref(),
                entry_point.as_deref(),
                description.as_deref(),
                json,
            )
            .await
        }
        ScriptCommands::List { lang } => {
            scripts::script_list(endpoint, lang.as_deref(), json).await
        }
        ScriptCommands::Inspect { iri } => scripts::script_inspect(endpoint, &iri, json).await,
        ScriptCommands::Run {
            iri,
            inputs,
            branch,
        } => scripts::script_run(endpoint, &iri, &inputs, branch.as_deref(), json).await,
    }
}

async fn remote_institution(endpoint: &str, command: InstitutionCommands, json: bool) {
    match command {
        InstitutionCommands::Install { definition } => {
            institutions::institution_install(endpoint, &definition, json).await
        }
        InstitutionCommands::List => institutions::institution_list(endpoint, json).await,
        InstitutionCommands::Inspect { iri } => {
            institutions::institution_inspect(endpoint, &iri, json).await
        }
    }
}

async fn remote_capability(endpoint: &str, command: CapabilityCommands, json: bool) {
    match command {
        CapabilityCommands::List => remote_capability_list(endpoint, json).await,
        CapabilityCommands::Inspect { iri } => {
            remote_capability_inspect(endpoint, &iri, json).await
        }
        CapabilityCommands::Test { iri, input, mode } => {
            remote_capability_test(endpoint, &iri, &input, &mode, json).await
        }
    }
}

async fn remote_capability_list(endpoint: &str, json: bool) {
    let mut client = connect_client(endpoint).await;

    // Find all Component resources
    let components_query = r#"
        MATCH "urn:eigenius:program:Component"(?c) {
            "urn:eigenius:core:short_name": ?name
        }
        RETURN [] { iri: ?c, name: ?name }
    "#;

    // Find all Institution resources
    let institutions_query = r#"
        MATCH "urn:eigenius:institution:Institution"(?i) {
            "urn:eigenius:institution:institution_name": ?name
        }
        RETURN [] { iri: ?i, name: ?name }
    "#;

    let components = run_query(&mut client, components_query).await;
    let institutions = run_query(&mut client, institutions_query).await;

    if json {
        println!(
            "{{\"components\":{},\"institutions\":{}}}",
            serde_json::to_string(&components).unwrap(),
            serde_json::to_string(&institutions).unwrap()
        );
    } else {
        println!("Components:");
        if components.is_empty() {
            println!("  (none registered)");
        } else {
            for r in &components {
                let iri = r.get("iri").and_then(|v| v.as_str()).unwrap_or("?");
                let name = r.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                println!("  {name} ({iri})");
            }
        }
        println!();
        println!("Institutions:");
        if institutions.is_empty() {
            println!("  (none registered)");
        } else {
            for r in &institutions {
                let iri = r.get("iri").and_then(|v| v.as_str()).unwrap_or("?");
                let name = r.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                println!("  {name} ({iri})");
            }
        }
    }
}

/// Run a remote EigenQL query and materialise its rows into plain JSON
/// objects keyed by the RETURN clause's short names.
///
/// Walks the response document per D2 Appendix A:
///   1. Parse the response bytes as an Eigon document.
///   2. Find the ResultSet (has `is_a` including `urn:eigenius:query:ResultSet`).
///   3. Find the row class (`result_class` IRI points at a Class resource
///      in the same document).
///   4. For each Property IRI listed on the class, read its `short_name`
///      and build a short_name → property IRI map.
///   5. For each embedded row in `rows`, emit a JSON object keyed by
///      short name.
///
/// Callers access values by the short name they put in the RETURN clause
/// — e.g. `row.get("iri")` when the query said `RETURN [] { iri: ?c }`.
pub(crate) async fn run_query(
    client: &mut eigenius_kernel::server::proto::eigenius_kernel_client::EigeniusKernelClient<
        tonic::transport::Channel,
    >,
    eigenql: &str,
) -> Vec<serde_json::Value> {
    use eigenius_kernel::ontology::eigon_cbor;
    use eigenius_kernel::ontology::iri::Iri;
    use eigenius_kernel::ontology::resource::{Resource, Value as RValue};
    use eigenius_kernel::ontology::well_known as wk;
    use eigenius_kernel::query::document as qdoc;

    let resp = match client
        .query(eigenius_kernel::server::proto::QueryRequest {
            at_layer: String::new(),
            eigenql: eigenql.to_string(),
            branch: String::new(),
        })
        .await
    {
        Ok(r) => r.into_inner(),
        Err(e) => {
            eprintln!("Query failed: {e}");
            return Vec::new();
        }
    };
    if !resp.success {
        eprintln!("Query failed: {}", resp.error);
        return Vec::new();
    }

    let document = match eigon_cbor::parse_document(&resp.document) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to parse result document: {e}");
            return Vec::new();
        }
    };

    // Index the document by IRI for class/property lookup.
    let by_iri: std::collections::BTreeMap<String, &Resource> = document
        .iter()
        .filter_map(|r| r.id().map(|iri| (iri.as_str().to_string(), r)))
        .collect();

    // Locate the ResultSet.
    let is_a_iri = Iri::parse(wk::IS_A).unwrap();
    let rs_class = qdoc::RESULT_SET_CLASS;
    let result_set = document.iter().find(|r| match r.get(&is_a_iri) {
        Some(RValue::Array(a)) => a.iter().any(|v| s_as_str(v) == rs_class),
        _ => false,
    });
    let result_set = match result_set {
        Some(r) => r,
        None => return Vec::new(),
    };

    // Walk to the row class.
    let row_class_iri = match result_set.get(&Iri::parse(qdoc::RESULT_CLASS_PROP).unwrap()) {
        Some(RValue::String(s)) => s.clone(),
        Some(RValue::ResourceRef(i)) => i.as_str().to_string(),
        _ => return Vec::new(),
    };
    let row_class = match by_iri.get(&row_class_iri) {
        Some(c) => *c,
        None => return Vec::new(),
    };

    // Build short_name → property IRI map from the class's property list.
    let properties_prop = Iri::parse("urn:eigenius:core:properties").unwrap();
    let short_name_prop = Iri::parse(wk::SHORT_NAME).unwrap();
    let mut short_to_iri: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    if let Some(RValue::Array(props)) = row_class.get(&properties_prop) {
        for p in props {
            let prop_iri = match p {
                RValue::String(s) => s.clone(),
                RValue::ResourceRef(i) => i.as_str().to_string(),
                _ => continue,
            };
            let Some(prop_res) = by_iri.get(&prop_iri) else {
                continue;
            };
            if let Some(RValue::String(short)) = prop_res.get(&short_name_prop) {
                short_to_iri.insert(short.clone(), prop_iri);
            }
        }
    }

    // Iterate rows (embedded inside the ResultSet) and project each into a
    // JSON object keyed by short name.
    let mut out = Vec::new();
    if let Some(RValue::Array(rows)) = result_set.get(&Iri::parse(qdoc::ROWS_PROP).unwrap()) {
        for row_val in rows {
            let row = match row_val {
                RValue::Embedded(r) => r.as_ref(),
                _ => continue,
            };
            let mut obj = serde_json::Map::new();
            for (short, iri_str) in &short_to_iri {
                let Ok(iri) = Iri::parse(iri_str) else {
                    continue;
                };
                if let Some(v) = row.get(&iri) {
                    if let Some(json) = value_to_json(v) {
                        obj.insert(short.clone(), json);
                    }
                }
            }
            out.push(serde_json::Value::Object(obj));
        }
    }
    out
}

fn s_as_str(v: &eigenius_kernel::ontology::resource::Value) -> &str {
    use eigenius_kernel::ontology::resource::Value;
    match v {
        Value::String(s) => s.as_str(),
        Value::ResourceRef(i) => i.as_str(),
        _ => "",
    }
}

fn value_to_json(v: &eigenius_kernel::ontology::resource::Value) -> Option<serde_json::Value> {
    use eigenius_kernel::ontology::resource::Value;
    Some(match v {
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::ResourceRef(i) => serde_json::Value::String(i.as_str().to_string()),
        Value::Integer(n) => serde_json::Value::Number((*n).into()),
        Value::Float(f) => serde_json::Number::from_f64(*f).map(serde_json::Value::Number)?,
        Value::Boolean(b) => serde_json::Value::Bool(*b),
        _ => return None,
    })
}

async fn remote_capability_inspect(endpoint: &str, iri: &str, json: bool) {
    let mut client = connect_client(endpoint).await;

    let request = eigenius_kernel::server::proto::InspectRequest {
        at_layer: String::new(),
        iri: iri.to_string(),
        branch: String::new(),
    };

    match client.inspect(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            if !resp.found {
                eprintln!("Capability not found: {iri}");
                std::process::exit(1);
            }
            let resource =
                match eigenius_kernel::ontology::eigon_cbor::parse_resource(&resp.resource) {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("Failed to parse resource: {e}");
                        std::process::exit(1);
                    }
                };
            if json {
                let v = eigenius_kernel::ontology::eigon_json::serialize_resource(&resource);
                println!("{}", serde_json::to_string_pretty(&v).unwrap());
            } else {
                print_capability_human(&resource);
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

fn print_capability_human(resource: &eigenius_kernel::ontology::resource::Resource) {
    use eigenius_kernel::ontology::iri::Iri;
    use eigenius_kernel::ontology::resource::Value;

    let get = |key: &str| -> Option<&Value> { resource.get(&Iri::parse(key).unwrap()) };
    let get_str =
        |key: &str| -> Option<String> { get(key).and_then(|v| v.as_str()).map(|s| s.to_string()) };

    if let Some(id) = resource.id() {
        println!("IRI:             {}", id.as_str());
    }
    if let Some(name) = get_str("urn:eigenius:core:short_name") {
        println!("Name:            {name}");
    }
    if let Some(desc) = get_str("urn:eigenius:core:description") {
        println!("Description:     {desc}");
    }

    // is_a
    let is_a_iris = resource.is_a();
    if !is_a_iris.is_empty() {
        let classes: Vec<String> = is_a_iris.iter().map(|i| i.as_str().to_string()).collect();
        println!("Classes:         {}", classes.join(", "));
    }

    // Component-specific
    if let Some(impl_) = get_str("urn:eigenius:program:component:implementation") {
        println!("Implementation:  {impl_}");
    }
    if let Some(cap) = get_str("urn:eigenius:program:component:capability_level") {
        println!("Capability:      {cap}");
    }
    if let Some(input) = get_str("urn:eigenius:program:component:input_type") {
        println!("Input type:      {input}");
    }
    if let Some(output) = get_str("urn:eigenius:program:component:output_type") {
        println!("Output type:     {output}");
    }
    if let Some(arg) = get_str("urn:eigenius:program:component:argument_type") {
        println!("Argument type:   {arg}");
    }

    // Institution-specific
    if let Some(impl_) = get_str("urn:eigenius:institution:implementation") {
        println!("Implementation:  {impl_}");
    }
    if let Some(inst_iri) = get_str("urn:eigenius:institution:institution_iri") {
        println!("Institution IRI: {inst_iri}");
    }
}

async fn remote_capability_test(
    endpoint: &str,
    iri: &str,
    input_file: &str,
    mode: &str,
    json: bool,
) {
    let mut client = connect_client(endpoint).await;

    // Detect institution-hood via ListInstitutions (the authoritative view —
    // institutions may register under a binary-declared IRI that differs from
    // the ontology resource's @id).
    let institutions = client
        .list_institutions(eigenius_kernel::server::proto::ListInstitutionsRequest {
            at_layer: String::new(),
        })
        .await
        .map(|r| r.into_inner().institutions)
        .unwrap_or_default();

    let is_institution = institutions.iter().any(|i| i.iri == iri);

    let input_json = read_as_json(input_file);

    if is_institution {
        // There is no per-institution dispatch RPC. Per-RPC
        // FiberQuery / DiscoverMorphisms primitives from the D10 era
        // were retired in Phase 12. To exercise an institution's
        // QueryClasses, write an EigenQL FIBER query and submit it via
        // the Query RPC (D2 v2 §3.5). `eigenius capability test` no
        // longer supports direct institution invocation.
        let _ = mode;
        eprintln!(
            "`capability test` cannot directly invoke an institution.\n\
             Write an EigenQL FIBER query against one of this institution's QueryClasses\n\
             and submit it via `eigenius query` instead — see D2 v2 §3.5.\n\
             Institution: {iri}"
        );
        std::process::exit(1);
    } else {
        // Component: wrap in a trivial program that applies the component to input
        let program_json = format!(
            r#"{{
                "@id": "urn:eigenius:cli:capability_test_program",
                "urn:eigenius:core:is_a": ["urn:eigenius:program:Program"],
                "urn:eigenius:program:input_type": "urn:eigenius:core:Class",
                "urn:eigenius:program:output_type": "urn:eigenius:core:Class",
                "urn:eigenius:program:body": {{
                    "urn:eigenius:core:is_a": ["urn:eigenius:program:Apply"],
                    "urn:eigenius:program:function": "{iri}",
                    "urn:eigenius:program:argument": {{
                        "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
                        "urn:eigenius:program:name": "input"
                    }}
                }}
            }}"#
        );

        match client
            .run_program(eigenius_kernel::server::proto::RunProgramRequest {
                program: program_json.into_bytes(),
                input: input_json,
                content_type: "application/eigon+json".to_string(),
                branch: String::new(),
            })
            .await
        {
            Ok(response) => {
                let resp = response.into_inner();
                if resp.success {
                    print_test_result(&resp.output, json);
                } else {
                    eprintln!("Component execution failed:");
                    for err in &resp.errors {
                        eprintln!("  {}: {}", err.rule, err.message);
                    }
                    std::process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("gRPC error: {e}");
                std::process::exit(1);
            }
        }
    }
}

fn print_test_result(cbor_bytes: &[u8], json: bool) {
    if json {
        if let Ok(resource) =
            eigenius_kernel::ontology::eigon_cbor::parse_resource_lenient(cbor_bytes)
        {
            let v = eigenius_kernel::ontology::eigon_json::serialize_resource(&resource);
            println!("{}", serde_json::to_string_pretty(&v).unwrap());
        } else {
            eprintln!("Failed to parse result CBOR");
            std::process::exit(1);
        }
    } else if let Ok(resource) =
        eigenius_kernel::ontology::eigon_cbor::parse_resource_lenient(cbor_bytes)
    {
        let v = eigenius_kernel::ontology::eigon_json::serialize_resource(&resource);
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
    } else {
        eprintln!("Failed to parse result");
        std::process::exit(1);
    }
}
