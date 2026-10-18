# Copyright 2026 The Eigenius Authors
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

# `EigeniusRWorker.R` — R-side driver for the R language runtime (D55).
#
# Loads the `eigenius-r-worker` cdylib, binds the substrate's Unix-domain
# socket, and runs the dispatch loop. The cdylib (shared Rust) owns the
# transport + CBOR; R only drives control flow and runs the computation —
# the same split eigenius-lean-worker uses for Lean.
#
# Inputs (env first — the ServiceSpawner convention — then argv fallback
# for standalone use):
#   EIGENIUS_TEST_WORKER_UDS   socket path to bind  (argv[1])
#   EIGENIUS_R_WORKER_CDYLIB   cdylib to dyn.load   (argv[2])
#
# RequestKind values (mirror crate::RequestKind):
#   0 Health  1 Instantiate  2 RegisterMirror  3 DispatchScript
#   4 DispatchMethod  5 Evict   -1 Closed  -2 TransportError  -3 Malformed
#
# The substrate dials a fresh connection per RPC, so the loop re-accepts
# (`r_accept_next`) when a connection closes (`Closed`).
#
# P1.2 contract: a DispatchScript's source is an R expression whose value
# is the output byte vector (a `raw`); the driver evaluates it and returns
# those bytes. The typed input-matrix / output-DerivedResource Eigon-CBOR
# marshalling lands with the first real recompute (P5).

arg_or_env <- function(env_name, args, idx) {
  v <- Sys.getenv(env_name, unset = NA_character_)
  if (!is.na(v) && nzchar(v)) {
    return(v)
  }
  if (length(args) >= idx) {
    return(args[[idx]])
  }
  stop("missing ", env_name, " (and no argv[", idx, "] fallback)")
}

# Boot cross-check (D26 §9.3): when running against a pinned image, verify
# the in-image manifest-hash matches the digest the substrate recorded, and
# fail closed (exit 78) on mismatch — mirroring JuliaWorker's verify_cross_check
# and the substrate's `prepare_substrate_side`/`verify_in_worker`. Under the
# LocalServiceSpawner dev path there is no pinned environment, so the env var
# is unset and the check is skipped (correct — nothing to verify against).
EXIT_CROSS_CHECK_FAILURE <- 78L

verify_cross_check <- function() {
  env_hash <- Sys.getenv("EIGENIUS_RUNTIME_ENV_MANIFEST_HASH", unset = NA_character_)
  if (is.na(env_hash) || !nzchar(env_hash)) {
    return(invisible()) # no pinned environment (local dev) → nothing to check
  }
  prov_dir <- Sys.getenv("EIGENIUS_RUNTIME_ENV_DIR", unset = "/etc/eigenius-runtime-env")
  file_path <- file.path(prov_dir, "manifest-hash")
  in_image <- tryCatch(
    trimws(paste(readLines(file_path, warn = FALSE), collapse = "\n")),
    error = function(e) NULL
  )
  if (is.null(in_image)) {
    cat("eigenius-r-worker: cross-check: manifest-hash unreadable at", file_path, "\n",
        file = stderr())
    quit(status = EXIT_CROSS_CHECK_FAILURE)
  }
  if (!identical(in_image, env_hash)) {
    cat("eigenius-r-worker: cross-check: manifest-hash mismatch (env vs in-image)\n",
        file = stderr())
    quit(status = EXIT_CROSS_CHECK_FAILURE)
  }
}

args <- commandArgs(trailingOnly = TRUE)
socket_path <- arg_or_env("EIGENIUS_TEST_WORKER_UDS", args, 1L)
cdylib_path <- arg_or_env("EIGENIUS_R_WORKER_CDYLIB", args, 2L)

verify_cross_check()

dyn.load(cdylib_path)

id <- .Call("r_listen", socket_path)
if (id < 0L) {
  stop("r_listen failed (could not bind/accept on ", socket_path, ")")
}

repeat {
  kind <- .Call("r_next_kind", id)

  if (kind == 0L) {
    # Health
    .Call("r_send_health", id)
  } else if (kind == 3L) {
    # DispatchScript — evaluate the R source; its value is the output bytes
    # (typically the CBOR of an Eigon DerivedResource built via the
    # r_eigon_* marshalling helpers). The input resources (CBOR) are bound
    # as `eigenius_inputs` (a list of raw vectors) in the eval environment;
    # the script decodes them with `r_eigon_f64_array` / `r_eigon_str_array`.
    # For a `PinnedExternalFile` input (D53), the substrate has already
    # fetched + content-verified the external file; the script obtains the
    # local path with `.Call("r_eigon_materialized_path", eigenius_inputs[[i]])`
    # and opens it with the appropriate reader (read.csv / arrow::read_parquet).
    src <- .Call("r_script_source", id)
    n <- .Call("r_input_count", id)
    eigenius_inputs <- if (n > 0L) {
      lapply(seq_len(n) - 1L, function(i) .Call("r_input", id, as.integer(i)))
    } else {
      list()
    }
    eval_env <- new.env(parent = globalenv())
    eval_env$eigenius_inputs <- eigenius_inputs
    result <- tryCatch(eval(parse(text = src), envir = eval_env), error = function(e) e)
    if (inherits(result, "error")) {
      .Call("r_send_dispatch_failed", id, "runtime_error", conditionMessage(result))
    } else {
      .Call("r_send_dispatch_ok", id, as.raw(result))
    }
  } else if (kind == 5L) {
    # Evict
    .Call("r_send_evicted", id)
    break
  } else if (kind == -1L) {
    # Connection closed cleanly; wait for the next substrate dial.
    if (.Call("r_accept_next", id) < 0L) {
      break
    }
  } else {
    # 4 (DispatchMethod, P4), -2/-3 (transport/malformed): stop for P1.2/P2.
    break
  }
}

invisible(.Call("r_close", id))
