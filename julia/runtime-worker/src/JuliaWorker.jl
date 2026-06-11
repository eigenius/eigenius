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
#   - Response::DispatchOk{...} → {"verb": "dispatch_ok", "invocation_id": ..., "output": <bytes>, "derivations": [<bytes>, ...], "dispatched_to": ...}
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
        # `target_kind` chooses between the eval-source path (default,
        # used by RunRuntimeScript) and the typed-method path
        # (CallRuntimeMethod). Wire format pinned by D26 §8.1 +
        # `runtime-substrate/src/rpc/protocol.rs::TargetKind`.
        target_kind = get(req, "target_kind", "script")
        invocation_id = req["invocation_id"]
        if target_kind == "script"
            return dispatch_julia(invocation_id, req["target"])
        elseif target_kind == "method"
            return dispatch_typed_method(
                invocation_id,
                req["target"],
                get(req, "inputs", Vector{Vector{UInt8}}()),
            )
        else
            return failure(invocation_id, "method_signature_mismatch",
                "unknown target_kind: $target_kind")
        end
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
        "derivations" => Vector{Vector{UInt8}}(),
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

# --- Typed-method dispatch (CallRuntimeMethod path) -----------------------
#
# Wire shape: `target` is a CBOR-encoded MethodInvocation, `inputs`
# are CBOR-encoded mirror struct dicts. The worker:
#   1. Discovers loaded mirror modules' `_eigenius_decoders` /
#      `_eigenius_encoders` registries.
#   2. Decodes each input by the leading entry of its `is_a` list.
#   3. Looks up `function_name` in `Main` (or any `using`-imported
#      module reachable from there).
#   4. Calls the function with the decoded args.
#   5. Captures `which(...)` for `dispatched_to` (D26 §4.2).
#   6. Encodes the result via the encoder registry.

const PROP_IS_A = "urn:eigenius:core:is_a"

"""Walk `Base.loaded_modules` for every module defining
`_eigenius_decoders` and merge their `class_iri → decode_fn` entries.
Built fresh per dispatch so newly-loaded mirror modules show up
immediately. We use `Base.loaded_modules` rather than `names(Main)`
because `using SomeMirror` brings the mirror's *exports* into Main
(the constants themselves, not the module-as-a-named-entity), so
walking Main's name table loses the per-mirror grouping. Iterating
loaded modules directly is the robust way to find every loaded mirror
package's registry."""
function discover_decoders()::Dict{String, Function}
    out = Dict{String, Function}()
    for (_pkg_id, mod) in Base.loaded_modules
        try
            if isdefined(mod, :_eigenius_decoders)
                reg = getfield(mod, :_eigenius_decoders)
                if reg isa AbstractDict
                    for (k, v) in reg
                        out[String(k)] = v
                    end
                end
            end
        catch
            # Some loaded modules (Base, Core stubs, ...) reject
            # `isdefined`/`getfield` calls outside their public surface;
            # silently skip — they aren't mirror modules anyway.
        end
    end
    return out
end

"""Mirror of `discover_decoders` for `_eigenius_encoders`. Keyed on
concrete struct types (not class IRIs) — encoding dispatches on
`typeof(value)`."""
function discover_encoders()::Dict{DataType, Function}
    out = Dict{DataType, Function}()
    for (_pkg_id, mod) in Base.loaded_modules
        try
            if isdefined(mod, :_eigenius_encoders)
                reg = getfield(mod, :_eigenius_encoders)
                if reg isa AbstractDict
                    for (k, v) in reg
                        out[k] = v
                    end
                end
            end
        catch
        end
    end
    return out
end

function dispatch_typed_method(
    invocation_id::AbstractString,
    target_bytes::Vector{UInt8},
    input_byte_arrays,
)
    # 1. Decode the MethodInvocation directive from the target bytes.
    invocation = try
        CBOR.decode(target_bytes)
    catch e
        return failure(invocation_id, "method_signature_mismatch",
            "could not decode target as CBOR MethodInvocation: $e")
    end
    if !(invocation isa AbstractDict)
        return failure(invocation_id, "method_signature_mismatch",
            "MethodInvocation must be a Dict, got $(typeof(invocation))")
    end
    function_name = get(invocation, "function_name", nothing)
    if !(function_name isa AbstractString)
        return failure(invocation_id, "method_signature_mismatch",
            "MethodInvocation.function_name missing or not a string")
    end

    # 2. Decode each input via the mirror modules' registries.
    decoders = discover_decoders()
    decoded_args = []
    for (i, bytes) in enumerate(input_byte_arrays)
        m = try
            CBOR.decode(Vector{UInt8}(bytes))
        catch e
            return failure(invocation_id, "method_signature_mismatch",
                "could not decode input #$i as CBOR: $e")
        end
        if !(m isa AbstractDict)
            return failure(invocation_id, "method_signature_mismatch",
                "input #$i must be a Dict (a CBOR-encoded mirror resource), got $(typeof(m))")
        end
        is_a = get(m, PROP_IS_A, nothing)
        if !(is_a isa AbstractVector) || isempty(is_a)
            return failure(invocation_id, "method_signature_mismatch",
                "input #$i missing or empty `is_a` list — needed to dispatch the decoder")
        end
        decoded = nothing
        for class_iri in is_a
            class_iri_s = String(class_iri)
            if haskey(decoders, class_iri_s)
                try
                    # `invokelatest` so decoders defined in a newer
                    # world (after a setup script's `using`/eval) are
                    # visible from this dispatch frame's older world.
                    decoded = Base.invokelatest(decoders[class_iri_s], m)
                    break
                catch e
                    return failure(invocation_id, "runtime_error",
                        "decoder for class $class_iri_s on input #$i failed: $e")
                end
            end
        end
        if decoded === nothing
            return failure(invocation_id, "method_signature_mismatch",
                "no mirror decoder registered for any class in input #$i.is_a = $(is_a)")
        end
        push!(decoded_args, decoded)
    end

    # 3. Look up `function_name` in Main. `using`-imported names live
    # in Main's binding table, so this finds handlers from mirror or
    # institution-handler modules `using`-loaded by the worker.
    #
    # Bindings added by `using` after `dispatch_typed_method` was
    # first compiled live in a newer world age than the function's
    # compile-time view of `Main`. A direct `isdefined(Main, sym)`
    # / `getfield(Main, sym)` here would miss them. `Core.eval`
    # always evaluates at the current world, so it sees newly-loaded
    # handler exports without an `invokelatest` rabbit hole — the
    # canonical Julia idiom for "look up a symbol added at runtime".
    fn_symbol = Symbol(function_name)
    fn = try
        Core.eval(Main, fn_symbol)
    catch e
        if e isa UndefVarError
            return failure(invocation_id, "method_signature_mismatch",
                "function `$function_name` not defined in Main — handler module not loaded?")
        end
        return failure(invocation_id, "method_signature_mismatch",
            "Core.eval(Main, :$function_name) raised: $e")
    end
    if !(fn isa Function)
        return failure(invocation_id, "method_signature_mismatch",
            "Main.$function_name is not a function (got $(typeof(fn)))")
    end

    # 4. Capture which() for `dispatched_to` BEFORE calling, in case
    # the call panics — this gives the auditor the dispatch attempt
    # even on failure. `which` returns a `Method` object whose `repr`
    # is the standard "Module.f(::T1, ::T2) at file:line" form (D26
    # §4.2).
    arg_types = Tuple{(typeof(a) for a in decoded_args)...}
    dispatched_to_str = try
        string(which(fn, arg_types))
    catch e
        return failure(invocation_id, "method_signature_mismatch",
            "no method matches Main.$function_name for arg types $arg_types: $e")
    end

    # 5. Invoke the handler.
    result = try
        Base.invokelatest(fn, decoded_args...)
    catch e
        return failure(invocation_id, "runtime_error",
            "Main.$function_name dispatch failed: $e")
    end

    # 6. Encode the result. Multiple cases:
    #    - Result is a `QueryResponse` (D52 §6 institution-emitted
    #      derivation shape): split into (output, derivations) and
    #      encode each half via the discovered encoders.
    #    - Result is a mirror struct: dispatch via _eigenius_encoders
    #      (keyed on typeof) to its encode_<C>.
    #    - Result is a primitive: pass through (the caller decodes
    #      based on RuntimeMethodSignature.output_type).
    encoders = discover_encoders()
    encode_one = function (value)
        if haskey(encoders, typeof(value))
            try
                return Base.invokelatest(encoders[typeof(value)], value)
            catch e
                throw(ErrorException("encoder for $(typeof(value)) failed: $e"))
            end
        else
            # Primitive (or anything without an encoder) — emit as-is.
            return value
        end
    end

    # Duck-type detection of a query-response shape: any value with
    # both `output` and `derivations` fields, where `derivations` is
    # iterable. Covers `EigeniusJuliaCommon.QueryResponse` (the
    # canonical author surface), NamedTuple `(output=..., derivations=[...])`,
    # and any other struct following the same shape. The worker
    # intentionally doesn't `using EigeniusJuliaCommon` itself — its
    # Project.toml stays minimal (CBOR + Sockets only) — and detects
    # by structure rather than nominal type.
    output_value = result
    derivation_values = Any[]
    if hasproperty(result, :output) &&
       hasproperty(result, :derivations) &&
       applicable(iterate, getproperty(result, :derivations))
        output_value = getproperty(result, :output)
        derivation_values = collect(getproperty(result, :derivations))
    end

    output_payload = try
        encode_one(output_value)
    catch e
        return failure(invocation_id, "runtime_error", string(e))
    end
    output_bytes = try
        CBOR.encode(output_payload)
    catch e
        return failure(invocation_id, "runtime_error",
            "could not CBOR-encode output: $e")
    end

    derivation_bytes_list = Vector{Vector{UInt8}}()
    for (i, dv) in enumerate(derivation_values)
        local payload
        payload = try
            encode_one(dv)
        catch e
            return failure(invocation_id, "runtime_error",
                "encoder for derivation #$i: $e")
        end
        local bytes
        bytes = try
            CBOR.encode(payload)
        catch e
            return failure(invocation_id, "runtime_error",
                "could not CBOR-encode derivation #$i: $e")
        end
        push!(derivation_bytes_list, bytes)
    end

    return Dict(
        "verb" => "dispatch_ok",
        "invocation_id" => invocation_id,
        "output" => output_bytes,
        "derivations" => derivation_bytes_list,
        "dispatched_to" => dispatched_to_str,
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

"""
Walk the in-image package tree (`/opt/eigenius/packages/*/Project.toml`
and `/opt/eigenius/mirror/Project.toml`) and `using` each baked package
into `Main`. This is the bridge between Pkg.develop'd packages (visible
to the project) and `Main` bindings (visible to `Core.eval(Main, ...)`
during dispatch). Without this, mirror modules' `_eigenius_decoders`
registry stays empty and the dispatcher rejects every input as
"no mirror decoder registered for class X".

Each `using` evaluates against `Main` at the worker's current world;
`Core.eval(Main, fn_symbol)` in `dispatch_typed_method` later picks up
the new bindings naturally.
"""
function load_baked_packages()
    package_roots = [
        "/opt/eigenius/packages",
        "/opt/eigenius/mirror",
    ]
    for root in package_roots
        isdir(root) || continue
        # `/opt/eigenius/mirror/` is a single package; `/opt/eigenius/packages/`
        # contains one subdirectory per package.
        candidates = if isfile(joinpath(root, "Project.toml"))
            [root]
        else
            [joinpath(root, name) for name in readdir(root)
             if isfile(joinpath(root, name, "Project.toml"))]
        end
        for pkg_dir in candidates
            project_toml = joinpath(pkg_dir, "Project.toml")
            name = parse_package_name(project_toml)
            name === nothing && continue
            try
                Core.eval(Main, Meta.parse("using $name"))
            catch e
                println(stderr,
                    "JuliaWorker: failed to `using $name` from $pkg_dir: $e; ",
                    "dispatches that depend on its bindings will fail")
            end
        end
    end
end

"""Parse `name = "..."` from a Julia Project.toml. Tolerant single-line
form; sufficient for the well-formed Project.tomls produced by the
substrate's mirror generator and `eigenius env build`."""
function parse_package_name(project_toml_path::AbstractString)::Union{Nothing, String}
    isfile(project_toml_path) || return nothing
    for line in eachline(project_toml_path)
        m = match(r"^\s*name\s*=\s*\"([^\"]+)\"", line)
        m === nothing && continue
        return m.captures[1]
    end
    return nothing
end

function main()
    verify_cross_check()
    ENV_DIGEST[] = ENV["EIGENIUS_RUNTIME_ENV_DIGEST"]
    ENV_HASH[] = ENV["EIGENIUS_RUNTIME_ENV_MANIFEST_HASH"]

    load_baked_packages()

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
