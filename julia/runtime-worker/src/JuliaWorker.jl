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

# `JuliaWorker.jl` — minimal Julia worker for Phase 18d's substrate
# capstone. Speaks the Eigenius substrate's CBOR RPC over a Unix
# domain socket; Phase 19a inherits this as the seed of
# `eigenius-julia`'s production worker.
#
# Wire format mirrors the Rust enums in
# `crates/runtime-substrate/src/rpc/protocol.rs`. The enums are
# `#[serde(tag = "verb", rename_all = "snake_case")]` — internally
# tagged with a `verb` discriminator and snake_case names, with
# variant fields flattened into the same CBOR map:
#
#   - Request::Health → {"verb": "health"}
#   - Request::DispatchMethod{...} → {"verb": "dispatch_method", "invocation_id": ..., "target": <bytes>, "inputs": [...]}
#   - Response::Health(HealthInfo) → {"verb": "health", "manifest_hash_in_image": ..., "env_digest_in_image": ..., "numerical_metadata": {...}}
#   - Response::DispatchOk{...} → {"verb": "dispatch_ok", "invocation_id": ..., "output": <bytes>, "dispatched_to": ...}
#   - Response::Evicted → {"verb": "evicted"}
#
# Length-prefixed framing: 4-byte big-endian length || CBOR body.

using CBOR
using Sockets

const EXIT_CROSS_CHECK_FAILURE = 78
const FRAME_HEADER_BYTES = 4
const DEFAULT_PROVENANCE_DIR = "/etc/eigenius-runtime-env"
const MANIFEST_HASH_FILE = "manifest-hash"

# --- Cross-check (D26 §9.3) -------------------------------------------------

function verify_cross_check()
    env_digest = get(ENV, "EIGENIUS_RUNTIME_ENV_DIGEST", nothing)
    env_hash = get(ENV, "EIGENIUS_RUNTIME_ENV_MANIFEST_HASH", nothing)
    if env_digest === nothing
        cross_check_fail("required env var `EIGENIUS_RUNTIME_ENV_DIGEST` is not set")
    end
    if env_hash === nothing
        cross_check_fail("required env var `EIGENIUS_RUNTIME_ENV_MANIFEST_HASH` is not set")
    end
    prov_dir = get(ENV, "EIGENIUS_RUNTIME_ENV_DIR", DEFAULT_PROVENANCE_DIR)
    file_path = joinpath(prov_dir, MANIFEST_HASH_FILE)
    in_image = try
        strip(read(file_path, String))
    catch e
        cross_check_fail("manifest-hash file at $file_path is unreadable: $e")
    end
    if in_image != env_hash
        cross_check_fail(
            "manifest-hash mismatch: env `$env_hash` vs in-image `$in_image` at $file_path",
        )
    end
end

function cross_check_fail(msg::AbstractString)
    println(stderr, "JuliaWorker: bootstrap cross-check failed: ", msg)
    exit(EXIT_CROSS_CHECK_FAILURE)
end

# --- Length-prefixed framing -----------------------------------------------

function read_frame(io)::Union{Nothing, Any}
    header = read(io, FRAME_HEADER_BYTES)
    if length(header) == 0
        return nothing  # clean EOF
    end
    if length(header) < FRAME_HEADER_BYTES
        error("partial frame header: got $(length(header)) bytes")
    end
    # Big-endian u32 length
    len = UInt32(header[1]) << 24 |
          UInt32(header[2]) << 16 |
          UInt32(header[3]) <<  8 |
          UInt32(header[4])
    body = read(io, Int(len))
    if length(body) != Int(len)
        error("partial frame body: expected $len bytes, got $(length(body))")
    end
    return CBOR.decode(body)
end

function write_frame(io, value)
    body = CBOR.encode(value)
    len = UInt32(length(body))
    header = UInt8[(len >> 24) & 0xff, (len >> 16) & 0xff, (len >> 8) & 0xff, len & 0xff]
    write(io, header)
    write(io, body)
    flush(io)
end

# --- Request handling -------------------------------------------------------

# Substrate-supplied env values, captured once at startup so `Health`
# responses are stable across the worker lifetime.
const ENV_DIGEST = Ref{Union{String, Nothing}}(nothing)
const ENV_HASH = Ref{Union{String, Nothing}}(nothing)

function handle_request(req)
    if !(req isa AbstractDict) || !haskey(req, "verb")
        return Dict(
            "verb" => "dispatch_failed",
            "invocation_id" => "?",
            "error_kind" => "method_signature_mismatch",
            "message" => "request missing `verb` discriminator: $(typeof(req))",
        )
    end
    verb = req["verb"]
    if verb == "health"
        return Dict(
            "verb" => "health",
            "manifest_hash_in_image" => ENV_HASH[],
            "env_digest_in_image" => ENV_DIGEST[],
            "numerical_metadata" => Dict(
                "blas_lib" => nothing,
                "blas_version" => nothing,
                "fma_enabled" => nothing,
                # Reported as "julia-test-runtime" so the capstone
                # test can distinguish this worker from the bash
                # test worker (which reports "test-runtime").
                "host_kernel" => "julia-test-runtime",
                "gpu_vendor" => nothing,
                "gpu_driver_version" => nothing,
            ),
        )
    elseif verb == "evict"
        return Dict("verb" => "evicted")
    elseif verb == "instantiate"
        return Dict("verb" => "instantiated", "ready" => true)
    elseif verb == "register_mirror"
        return Dict("verb" => "mirror_registered", "mirror_iri" => req["mirror_iri"])
    elseif verb == "dispatch_method"
        return dispatch_julia(req["invocation_id"], req["target"])
    end
    return Dict(
        "verb" => "dispatch_failed",
        "invocation_id" => get(req, "invocation_id", "?"),
        "error_kind" => "method_signature_mismatch",
        "message" => "unknown verb: $verb",
    )
end

function dispatch_julia(invocation_id::AbstractString, target_bytes::Vector{UInt8})
    # `target` is a CBOR-encoded String containing the Julia source.
    source = try
        CBOR.decode(target_bytes)
    catch e
        return failure(invocation_id, "method_signature_mismatch",
            "could not decode target as CBOR: $e")
    end
    if !(source isa AbstractString)
        return failure(invocation_id, "method_signature_mismatch",
            "expected target to decode to a String, got $(typeof(source))")
    end
    expr = try
        Meta.parse(source)
    catch e
        return failure(invocation_id, "method_signature_mismatch",
            "Julia parse error: $e")
    end
    # Stringify the eval'd value as the output. The bash worker takes
    # bash *stdout*; Julia is value-returning so the language-natural
    # output is the expression's value — scripts that want to format
    # output explicitly can call `string(...)` themselves. Phase 19a
    # may revisit this when actual Julia method dispatch lands;
    # 18d's capstone scope is "the e2e plumbing works" and the
    # simplest output channel suffices.
    result = try
        Base.eval(Main, expr)
    catch e
        return failure(invocation_id, "runtime_error", "eval failed: $e")
    end
    output_string = string(result)
    output_bytes = CBOR.encode(output_string)
    return Dict(
        "verb" => "dispatch_ok",
        "invocation_id" => invocation_id,
        "output" => output_bytes,
        "dispatched_to" => nothing,
    )
end

function failure(invocation_id, error_kind, message)
    return Dict(
        "verb" => "dispatch_failed",
        "invocation_id" => invocation_id,
        "error_kind" => error_kind,
        "message" => message,
    )
end

# --- Connection / accept loop ----------------------------------------------

@enum ServeOutcome EvictReceived ConnectionClosed

function serve(stream)::ServeOutcome
    while true
        req = read_frame(stream)
        if req === nothing
            return ConnectionClosed
        end
        evict_after = req isa AbstractDict && get(req, "verb", nothing) == "evict"
        resp = handle_request(req)
        write_frame(stream, resp)
        if evict_after
            return EvictReceived
        end
    end
end

function main()
    verify_cross_check()
    ENV_DIGEST[] = ENV["EIGENIUS_RUNTIME_ENV_DIGEST"]
    ENV_HASH[] = ENV["EIGENIUS_RUNTIME_ENV_MANIFEST_HASH"]

    uds_path = get(ENV, "EIGENIUS_TEST_WORKER_UDS", nothing)
    if uds_path === nothing
        println(stderr, "JuliaWorker: EIGENIUS_TEST_WORKER_UDS not set")
        exit(2)
    end
    # Stale socket from a previous worker run blocks `bind`.
    isfile(uds_path) && rm(uds_path)

    server = listen(uds_path)
    # World-rw so any caller UID can connect (substrate may run as a
    # different UID than the container's process — see test_runtime_docker.rs
    # for the same pattern in the bash worker).
    chmod(uds_path, 0o666)

    # Multi-connection loop: substrate may open separate connections
    # for Health and DispatchMethod (Phase 18c.5). Worker exits only
    # on explicit Evict.
    while true
        stream = accept(server)
        outcome = serve(stream)
        outcome == EvictReceived && break
        # ConnectionClosed: loop back and accept the next connection.
    end
    close(server)
end

main()
