# Eigenius — Project Rules

Typed knowledge graph platform. Rust workspace with kernel, storage backends,
CLI, and a Deno/TypeScript orchestration layer. See [`CLAUDE.md`](CLAUDE.md)
for build commands and architectural shape.

## Project posture

Eigenius is the foundation of a platform intended to change how typed
knowledge is represented, processed, and verified. **There is no release
timeline.** Getting the design right matters more than getting it done
quickly. Time pressure is not a valid reason to compromise structural
decisions — there is no time pressure. If a proper fix is multi-session,
multi-day, or multi-week, that is fine; invest the time.

The cost of shipping a wrong shape is paid continuously by everyone who works
on the system afterward, and paid most heavily when the wrong shape has to be
unwound under the pressure of downstream consumers. Avoid that entirely by
building the right shape the first time.

## Workflow ordering (mandatory)

For every task, in this order:

1. **Read first.** [`CLAUDE.md`](CLAUDE.md), then any design doc under
   [`docs/design/`](docs/design/) relevant to the surface you are touching.
   The design docs (`d1`–`d41`+) are authoritative; the code is the
   consequence, not the source of truth.
2. **Structural intent check.** Before substantive coding on a
   design-affecting surface, the relevant `docs/design/d*.md` must exist
   and cover the work. If no doc covers the change, dispatch to the
   `architect` persona to draft or extend it; **the doc lands first, in
   its own PR**. Subsequent code PRs cite the doc. The doc is the source
   of truth and is kept current — if implementation surfaces refinements,
   update the doc in the same PR as the refinement or in a follow-up.
3. **Structure-not-symptoms gate.** Before edits, ask: *"Am I solving the
   problem or hiding it?"* If the immediate fix is a *guard* against bad
   behaviour rather than *eliminating* the bad behaviour, stop. Escalate
   to the overall-lead human; do not paper over with an error message,
   bridge, or compatibility layer.
4. **Worktree, edit, build, test, lint.** Standard implementation loop.
   See the engineer template for the canonical commands.
5. **Doc currency.** Any design-affecting change keeps the relevant
   `docs/design/d*.md` current. Updates may ride in the implementation PR
   or in a sibling PR that merges alongside; the doc must not drift behind
   the code.

## Specific signals you are wedging instead of fixing

- Adding a parser/runtime error to reject malformed input that should be
  expressible (the AST or grammar is wrong, not the input).
- Adding a "bridge" or "compatibility layer" on top of a design you've
  already concluded is wrong, with the intent to "clean up later." Later
  rarely arrives.
- Reaching for "minimal scope" or "additive change" as a justification when
  the foundation itself needs reshaping.
- Filing a follow-up issue immediately after writing code you already know
  is structurally wrong, instead of writing the code right the first time.

When in doubt, ask: *"Am I solving the problem or just hiding it?"* If
hiding, do the harder thing.

## Coding conventions

[`CONVENTIONS.md`](CONVENTIONS.md) is mandatory. Read it before writing
code.

## Architecture and design

- [`docs/design/`](docs/design/) — design docs `d1`–`d41`+; the
  architectural source of truth.
- [`docs/design/architecture-v0.3.md`](docs/design/architecture-v0.3.md) —
  current architecture overview.
- [`docs/design/implementation-plan.md`](docs/design/implementation-plan.md) —
  plan of record.

Read the relevant design doc before working on an issue. If a relevant doc
is missing, that is a signal to author one (step 2 of "Workflow ordering"),
not to proceed without it.

## Build, test, lint

Use the `/build`, `/test`, and `/lint` skills. Each points at the canonical
invocation; CI runs the same commands.

### Before requesting review (mandatory)

1. Branch up to date with `origin/main` (fetch + rebase).
2. `/build`, `/lint`, `/test` all green at the **workspace** root —
   `cargo build`, `cargo test --workspace`,
   `cargo fmt --all -- --check && cargo clippy --workspace --all-targets`.
3. The relevant `docs/design/d*.md` covers the work. If implementation
   revealed refinements, the doc is updated in this PR or in a sibling PR
   that merges alongside.
4. No follow-up issue for known-wrong shape. If you already know the shape
   is wrong, fix it now.
5. TODOs reference a filed issue.
6. PR carries the `eigenius-agents-team` label.

## Agents, sub-agents, concurrent agents

Multiple coding agents work on this codebase simultaneously.

- Every PR developed in a dedicated worktree under
  `$SPRING_WORKSPACE_PATH/worktrees/<task>/`. Never work directly in the
  main clone — other agent processes may be active concurrently.
- Small, focused PRs — one issue per PR.
- Rebase onto `main` before merging.
- When adding to shared files (enums, dispatch tables, registries), append
  at the end to minimise merge conflicts with concurrent work.
