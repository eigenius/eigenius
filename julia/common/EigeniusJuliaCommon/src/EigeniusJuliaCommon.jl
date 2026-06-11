"""
    EigeniusJuliaCommon

Shared validation + codec helpers used by the auto-generated mirror
modules emitted by `eigon-julia-gen` (Phase 19a.3 of the Eigenius
runtime substrate). Generated code calls these functions; the
package itself is hand-authored.

## Validation primitives

Each `validate_*` function raises an `ArgumentError` with a clear
message on violation. The Eigenius substrate's mirror generator
emits inline calls inside each struct's inner constructor (D27 §3.3:
"Format violations raise `EigenValidationError`; this matches Julia
style — validation at the boundary"). v1 reuses Julia's standard
`ArgumentError` rather than a custom type so generated code's error
shape is familiar to Julia users; a typed `EigenValidationError`
becomes worthwhile if downstream code needs to dispatch on it.

## Format checks

Format constraints from the ontology (`urn:eigenius:core:formats:date`,
`:datetime`, `:iri`, `:uuid`, `:regex`) map to a single `validate_format`
call with the format-short-name as a `Symbol`. Each format has a
purpose-built check; unknown formats raise rather than silently pass.

## CBOR codec helpers

Generated `decode_*` / `encode_*` functions handle property reads
directly (`m["<iri>"]`); the codec helpers here exist for the small
number of cross-cutting cases where the generator can't inline the
operation — currently empty in v1, kept as a place to grow.
"""
module EigeniusJuliaCommon

# --- Numeric range validation ------------------------------------------

"""
    validate_min_value(field::Symbol, value::Real, min::Real)

Raises `ArgumentError` if `value < min`.
"""
function validate_min_value(field::Symbol, value::Real, min::Real)
    value >= min || throw(ArgumentError(
        "$field must be >= $min, got $value"))
end

"""
    validate_max_value(field::Symbol, value::Real, max::Real)

Raises `ArgumentError` if `value > max`.
"""
function validate_max_value(field::Symbol, value::Real, max::Real)
    value <= max || throw(ArgumentError(
        "$field must be <= $max, got $value"))
end

# --- Length validation -------------------------------------------------

"""
    validate_min_length(field::Symbol, value, n::Integer)

Raises `ArgumentError` if `length(value) < n`. Works for any
collection or string.
"""
function validate_min_length(field::Symbol, value, n::Integer)
    length(value) >= n || throw(ArgumentError(
        "$field must have length >= $n, got $(length(value))"))
end

"""
    validate_max_length(field::Symbol, value, n::Integer)

Raises `ArgumentError` if `length(value) > n`.
"""
function validate_max_length(field::Symbol, value, n::Integer)
    length(value) <= n || throw(ArgumentError(
        "$field must have length <= $n, got $(length(value))"))
end

# --- Pattern + format validation ---------------------------------------

"""
    validate_pattern(field::Symbol, value::AbstractString, pattern::AbstractString)

Raises `ArgumentError` if `value` does not *fully* match the regex
`pattern`. Anchoring is applied by this function — the user-supplied
pattern is wrapped as `^(?:<pattern>)\$` before compilation, matching
the kernel-side validator's semantics in
`kernel/src/validation/mod.rs::check_pattern`. Pinned by D29 §9.4.

Pattern syntax must use the portable subset of ECMA 262 features
supported by both Rust's `regex` crate and Julia's PCRE-derived
`Regex` — see D29 §9.5 for the exact subset and the PCRE-only
features that are NOT portable.
"""
function validate_pattern(field::Symbol, value::AbstractString, pattern::AbstractString)
    # Wrap in `^(?:…)$` so the user's pattern is treated as a full
    # match. A non-capturing group around the user's pattern keeps
    # alternation (`a|b`) anchored as a unit, not just at the leading
    # `a` and trailing `b`.
    anchored = "^(?:" * pattern * ")\$"
    re = try
        Regex(anchored)
    catch e
        throw(ArgumentError(
            "$field has invalid regex pattern \\\"$pattern\\\": $e"))
    end
    occursin(re, value) || throw(ArgumentError(
        "$field must match pattern \\\"$pattern\\\", got $(repr(value))"))
end

"""
    validate_format(field::Symbol, value::AbstractString, fmt::Symbol)

Validates a string against a named format (`:date`, `:datetime`,
`:time`, `:iri`, `:uuid`, `:regex` per the core ontology's
`Format` resources). Unknown formats raise rather than pass.
"""
function validate_format(field::Symbol, value::AbstractString, fmt::Symbol)
    if fmt === :date
        # ISO 8601 date — YYYY-MM-DD.
        occursin(r"^\d{4}-\d{2}-\d{2}$", value) || throw(ArgumentError(
            "$field must be ISO 8601 date (YYYY-MM-DD), got $(repr(value))"))
    elseif fmt === :datetime
        # ISO 8601 datetime with timezone offset or Z. Permissive on
        # fractional seconds and offset shape.
        occursin(
            r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})$",
            value,
        ) || throw(ArgumentError(
            "$field must be ISO 8601 datetime, got $(repr(value))"))
    elseif fmt === :time
        # ISO 8601 time — HH:MM:SS optionally with fractional seconds
        # and a Z / offset.
        occursin(
            r"^\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})?$",
            value,
        ) || throw(ArgumentError(
            "$field must be ISO 8601 time, got $(repr(value))"))
    elseif fmt === :iri
        # Permissive IRI check — RFC 3987 is hard to validate
        # exhaustively in regex. Require a scheme + : + non-empty
        # body. Tighter validation can land if a use case demands it.
        occursin(r"^[A-Za-z][A-Za-z0-9+.\-]*:.+", value) || throw(ArgumentError(
            "$field must be an IRI (RFC 3987), got $(repr(value))"))
    elseif fmt === :uuid
        # RFC 4122 UUID — eight-four-four-four-twelve hex.
        occursin(
            r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$",
            value,
        ) || throw(ArgumentError(
            "$field must be a UUID (RFC 4122), got $(repr(value))"))
    elseif fmt === :regex
        # Verify the value is a valid regex by trying to compile it.
        try
            Regex(value)
        catch e
            throw(ArgumentError(
                "$field must be a valid regex, got $(repr(value)): $(e)"))
        end
    else
        throw(ArgumentError(
            "$field uses unknown format `$fmt` (supported: :date, :datetime, :time, :iri, :uuid, :regex)"))
    end
end

# --- QueryResponse -----------------------------------------------------
#
# Return shape for institution query handlers (AutoOnLoad / Decidable
# / OnDemand QueryClasses) that want to emit per-effect derivations
# alongside the gate Verdict. Authors return a `QueryResponse(verdict,
# results)` instead of a bare verdict; the JuliaWorker detects this
# type at the encoder step, encodes each derivation as its own
# Eigon-CBOR resource, and threads them through the substrate wire
# protocol so the kernel commits them as
# `reflection:InstitutionEmittedDerivation`s under the gated subject
# (D52 §6).
#
# Bare-verdict handlers continue to work unchanged — they return the
# output resource directly and emit zero derivations.

"""
    QueryResponse(output, derivations=[])

Wraps an institution query handler's gate Verdict (`output`) and zero-
or-more side-effect derivation resources. The JuliaWorker recognises
this type and threads both halves across the substrate boundary; the
kernel stamps the `reflection:InstitutionEmittedDerivation` marker and
the `from_subject` / `runtime_invocation` linkage properties on each
derivation before committing.

Each derivation should carry its own `@id` (typically suffixed off the
gated subject, e.g. `{subject_iri}:result:{effect_name}`) and a
`reflection:canonical_proposition` if it should be admitted as an
`IsDerivedAs` witness target.
"""
struct QueryResponse{O,V<:AbstractVector}
    output::O
    derivations::V
end

QueryResponse(output) = QueryResponse(output, Any[])

# --- Exports -----------------------------------------------------------

export validate_min_value,
       validate_max_value,
       validate_min_length,
       validate_max_length,
       validate_pattern,
       validate_format,
       QueryResponse

end # module EigeniusJuliaCommon
