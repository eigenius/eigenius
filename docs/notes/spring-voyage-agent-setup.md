# Spring Voyage Agent Setup — Plan and Status

> **What this is.** A working document tracking the setup of the
> [Spring Voyage](https://github.com/cvoya-com/spring-voyage) platform
> as Eigenius's agentic-development framework. Captures the decisions
> we've made, the phases we've completed, the artifacts produced, and
> the work that remains.
>
> **Where this isn't.** This is *not* a structural design doc. It does
> not propose changes to Eigenius's architecture, identifiers, or AST
> shape — it documents how we deploy an external tool. Per
> [`AGENTS.md`](../../AGENTS.md), structural design docs live under
> [`docs/design/`](../design/) and are owned by the `architect` persona.

---

## Goal

Deploy Spring Voyage locally so it can host a team of AI engineering and
program-management agents that work against the `eigenius/eigenius`
repository on GitHub. The agents:

- pick up issues and PRs labelled `eigenius-agents-team`
- read the project's design docs and conventions before changing code
- open PRs through a dedicated bot identity (the GitHub App)
- escalate structural questions rather than papering over them

This dogfoods Eigenius's *design-first* posture
([`AGENTS.md`](../../AGENTS.md) § "Workflow ordering") while accelerating
the throughput of correct, well-scoped contributions.

---

## Phase status at a glance

| Phase | Title | Status |
| --- | --- | --- |
| 0 | Pre-flight checks | ✅ Done |
| 1 | Install Spring Voyage platform | ⏸ Next |
| 2 | Register GitHub App, create label, start webhook forwarder | ⏸ Pending |
| 3 | Build the agent engineer container image | ✅ Done |
| 4 | Author Eigenius anchor docs (AGENTS.md, CONVENTIONS.md, skills) | ✅ Done |
| 5 | Fork and customise the Spring Voyage package YAMLs | ✅ Done |
| 6 | Draft engineer personas | ✅ Done |
| 7 | Install the package, smoke-test, observe | ⏸ Pending |
| 8 | Real work and iteration | ⏸ Pending |

`✅` complete · `🟡` in progress · `⏸` not yet started.

---

## Decisions of record

These shape every downstream artifact; documented here so they survive
future context loss.

| Decision | Choice |
| --- | --- |
| GitHub target | `eigenius/eigenius` (org-owned, public, default branch `main`) |
| Custom package name | `eigenius-agents` |
| Package location in-tree | [`agents/eigenius-agents/`](../../agents/eigenius-agents/) |
| Webhook filter label | `eigenius-agents-team` |
| Starter team size | 1 engineer (`goedel`) + 1 PM (`russell`) |
| Bring-up model | `claude-haiku-4-5` (engineer, PM, leader); swap to `claude-opus-4-7` after Phase 7 |
| Persona dispatch model | `claude-opus-4-7` (design and code quality matters per dispatch) |
| Initiative throttle | `max_level: responsive`, `max_actions_per_hour: 3` (one tier below SV's OSS default) |
| Agent image | `eigenius/agent-engineer:dev`, built locally via docker |
| SV release pin | `0.0.0-rc.20260513.4` (SV has no stable release yet — only pre-releases) |
| GitHub auth mode | App-installation token (per ADR-0047 §11); PAT path available but unused |
| GitHub App owner | The `eigenius` org (user is org owner) |

### Structural surfaces (design-doc-gated)

Issues labelled with any of these `area:*` labels route to an engineer
**with the architect-first directive** (see the leader unit's intent
table at
[`agents/eigenius-agents/units/eigenius-agents/package.yaml`](../../agents/eigenius-agents/units/eigenius-agents/package.yaml)):

- `area:kernel`
- `area:storage`
- `area:orchestrator`
- `area:notebook`
- `area:documentation`
- `area:wasm`
- `area:institution`
- `area:verification-lean4`
- `area:verification-verus`

Non-structural surfaces (direct engineer dispatch, no design-doc gate):
`area:cli`, `area:examples`, `area:tooling`.

---

## Phase 0 — Pre-flight checks ✅

**Goal.** Confirm we have the credentials and tooling needed to run
Spring Voyage's GitHub-integrated flows.

| Check | Status | Notes |
| --- | --- | --- |
| `claude` CLI installed | ✅ | v2.1.126; previously v2.1.98 in agent image |
| GitHub PAT generated (classic, scopes `repo`, `read:org`, `workflow`) | ✅ | Stored in `~/.config/gh/hosts.yml` |
| `gh auth` working stably | ✅ | After WSL2 DNS workaround (below) |
| User confirmed as org-owner on `eigenius` | ✅ | `role: admin`, `state: active` — enables Phase 2 `--org eigenius` App registration |
| Repo admin rights on `eigenius/eigenius` | ✅ | Full perms: admin/maintain/push/triage/pull |
| `eigenius-agents-team` label exists | ❌ | Will be created in Phase 2 |

### WSL2 DNS workaround (lessons learned)

The host hit a WSL2-specific DNS bug while completing Phase 0. Captured
here so it doesn't get rediscovered next time.

**Symptom.** `gh api …` failed intermittently with
`dial tcp: lookup api.github.com on 10.255.255.254:53: no such host`.
`curl`, `getent`, and `dig +short A` all succeeded.

**Root cause.** WSL2 mirrored networking with `dnsTunneling=true` was
returning malformed packets for AAAA (IPv6) DNS queries. Go's pure
resolver issues parallel A+AAAA queries; a malformed AAAA response
poisons the whole lookup, while glibc-based tools (curl, dig)
tolerated the bad AAAA and returned the A answer.

**Fix applied.** Configured WSL to skip auto-generation of
`/etc/resolv.conf` and pinned static nameservers:

`C:\Users\hmwill\.wslconfig`:
```ini
[wsl2]
networkingMode=mirrored
dnsTunneling=false
```

`/etc/wsl.conf` inside WSL:
```ini
[network]
generateResolvConf = false
```

`/etc/resolv.conf` inside WSL (static):
```
nameserver 8.8.8.8
nameserver 1.1.1.1
```

After `wsl --shutdown` from PowerShell, `gh api user` returned
`hmwill` 10 times in a row.

**Anthropic OAuth token.** Not yet generated; produced via
`claude setup-token` during Phase 1 and registered with
`spring secret create --scope tenant anthropic-oauth --value '<token>'`
post-install.

---

## Phase 1 — Install Spring Voyage ⏸ Next

**Goal.** A green `voyage status` showing the platform stack up locally.

### Prereqs

- ✅ Phase 0 complete
- ❌ Podman 4+ installed (`sudo apt install -y podman`; current host has
  docker but SV's installer is podman-only)
- ❌ Ports 80 / 443 free on the host
- ❌ Anthropic OAuth token (interactive: `claude setup-token`)

### Install command

SV currently publishes only pre-releases; `:latest` does not resolve.
Pin to the current pre-release explicitly:

```bash
curl -fSL \
  https://github.com/cvoya-com/spring-voyage/releases/download/spring-voyage-v0.0.0-rc.20260513.4/install-0.0.0-rc.20260513.4.sh \
  | bash
```

Prompts during install:

1. `DEPLOY_HOSTNAME` → `localhost`
2. GitHub App registration → **skip** (deferred to Phase 2)

### Post-install steps

```bash
voyage status    # all containers green
claude setup-token    # interactive; produces an Anthropic OAuth token
spring secret create --scope tenant anthropic-oauth --value 'YOUR_ANTHROPIC_TOKEN'
spring secret list --scope tenant    # verify
```

### Image bridge

We built `eigenius/agent-engineer:dev` against docker. SV's worker runs
images via podman, which has a separate image store. Bridge it:

```bash
docker save eigenius/agent-engineer:dev | podman load
```

(Alternative: rebuild with `podman build`. Slower; the bridge command
is faster.)

---

## Phase 2 — GitHub App and webhook plumbing ⏸ Pending

**Goal.** SV can authenticate to GitHub as the *Eigenius Agents* bot
and receive webhook events for `eigenius/eigenius`.

### Steps

1. **Register the App under the `eigenius` org.** Requires org-owner
   rights (confirmed in Phase 0).
   ```bash
   spring github-app register --org eigenius
   ```
   Opens a device-flow in a browser (Windows). One click to confirm.
   SV writes App credentials to `~/.spring-voyage/spring.env` and
   prints an install URL.

2. **Install the App on `eigenius/eigenius`** via the URL above.
   Capture the numeric **installation ID** from the resulting URL
   (`.../installations/<id>`). Used as `--input
   github_installation_id=<id>` in Phase 7.

3. **Create the team label** on the repo:
   ```bash
   gh label create eigenius-agents-team \
     --description "Issues and PRs the Eigenius Agents team should own" \
     -R eigenius/eigenius
   ```

4. **Start the webhook forwarder** in a separate terminal:
   ```bash
   gh extension install cli/gh-webhook
   voyage gh-webhook-forward --repo eigenius/eigenius
   ```
   Leave running. GitHub tears down the short-lived forwarding hook
   automatically on Ctrl-C.

### Known risk

`voyage gh-webhook-forward` uses `gh webhook forward` internally —
needs `gh` working stably on the host. With the DNS fix in place this
should not be an issue, but it's a real coupling worth knowing about.

---

## Phase 3 — Build the agent engineer image ✅

**Goal.** A container image the SV worker can launch as the agent
runtime, with all the toolchains Eigenius work needs.

### What's done

- Dockerfile at
  [`eng/agent-images/Dockerfile.engineer`](../../eng/agent-images/Dockerfile.engineer).
- Built locally to `eigenius/agent-engineer:dev` (5.56 GB, docker).
- Base: `ghcr.io/cvoya-com/spring-voyage-agent-base:0.0.0-rc.20260513.4`
  (SV's BYOI base image).
- Layers added on top:
  - Claude Code CLI (`claude` v2.1.98)
  - Rust stable toolchain (`cargo`, `clippy`, `rustfmt`) via rustup
  - `protoc` for `kernel/build.rs` (tonic-build)
  - `gh` CLI v2.93.0
  - Deno v2.4.5 for the orchestrator
  - Julia v1.12.6 (direct binary download — juliaup's tight HTTP
    timeout consistently failed on slow paths)
  - elan + Lean 4 v4.29.1 for `eigenius-lean-worker`'s `build.rs` C
    bridge

### Conformance

- BYOI path 1 per [ADR-0027](../../../spring-voyage/docs/decisions/0027-agent-image-conformance-contract.md):
  ENTRYPOINT inherited from base (tini → A2A sidecar bridge); not
  overridden.
- Final `USER agent` (matches base; UID 1000 on this host conveniently
  matches the human user).

### Smoke tests passed

| Build path | Result |
| --- | --- |
| `cargo check -p eigenius-kernel` | ✅ 1m 01s |
| `cargo check -p eigenius-julia` | ✅ 2m 25s (no Julia toolchain needed at build) |
| `cargo check -p eigenius-lean-worker` | ✅ 2m 23s — `lean --print-prefix` via elan resolves `lean.h`, C bridge compiles |
| `cargo build -p eigenius-lean-worker` | ✅ 1m 21s |
| `lake build` in `lean/runtime-worker/` | ✅ 6/6 targets linked, executable produced |
| `deno check orchestration/src/main.ts` | ✅ Typecheck succeeded |

### Outstanding

- ❌ Image not yet bridged from docker → podman (Phase 1 step).
- ❌ Live agent run (BYOI sidecar bridge on `:8999` exercising the A2A
  protocol) not yet observed; Phase 7 will validate.

### Cdylib path coupling — note for Lake builds

The Lake project at
[`lean/runtime-worker/lakefile.lean`](../../lean/runtime-worker/lakefile.lean)
hardcodes `extraLinkArgs := ["-L../../target/debug",
"-leigenius_lean_worker", "-Wl,-rpath,$ORIGIN/../../../../../target/debug"]`
— i.e. the cdylib must be at `<workspace>/target/debug/`. If an agent
overrides `CARGO_TARGET_DIR`, Lake's link will fail. Bake this into
the institution persona prompt so engineers don't introduce that
coupling break. *(Already noted in
[`.claude/agents/institution-engineer.md`](../../.claude/agents/institution-engineer.md).)*

---

## Phase 4 — Anchor docs and skills ✅

Files written and committed:

| File | Purpose |
| --- | --- |
| [`AGENTS.md`](../../AGENTS.md) | Project posture, workflow ordering, design-first rules |
| [`CONVENTIONS.md`](../../CONVENTIONS.md) | Coding conventions (Resource model, IRI scheme, BTreeMap default, thiserror, async, naming) |
| [`.agents/skills/build/SKILL.md`](../../.agents/skills/build/SKILL.md) | `/build` → `cargo build --workspace` |
| [`.agents/skills/test/SKILL.md`](../../.agents/skills/test/SKILL.md) | `/test` → `cargo test --workspace` |
| [`.agents/skills/lint/SKILL.md`](../../.agents/skills/lint/SKILL.md) | `/lint` → `cargo fmt --check` + clippy `-D warnings` |

**Skill scoping.** The three Rust-workspace skills cover the *primary*
toolchain. Per-language commands for Deno (orchestrator) and
Vite/React (notebooks) live in the relevant persona prompts (`/`
namespace stays tight; cross-language `just`-based skills were
considered and rejected).

---

## Phase 5 — Custom package YAMLs ✅

Files written and committed under
[`agents/eigenius-agents/`](../../agents/eigenius-agents/):

```
agents/eigenius-agents/
├── package.yaml                                  ← top-level (name, version, image default)
├── templates/
│   ├── software-engineer/package.yaml            ← engineer AgentTemplate
│   └── program-manager/package.yaml              ← PM AgentTemplate
└── units/
    └── eigenius-agents/package.yaml              ← the unit (leader prompt, members, connector binding)
```

### Notable customisations from SV's `spring-voyage-oss` package

- All names rewritten (`spring-voyage-oss` → `eigenius-agents`,
  `spring-voyage-team` → `eigenius-agents-team`).
- Leader prompt's **intent table** reshaped to Eigenius's structural
  surface set with the architect-first directive on structural areas
  (see "Structural surfaces" above).
- Engineer template prompt rewritten:
  - Reframed as *"generalist that dispatches focused work to
    repository-defined personas under `.claude/agents/`"*
  - Workflow ordering carries the Eigenius design-first sequence:
    read first → structural intent check (dispatch `architect` if
    no design doc covers the work; doc lands first in its own PR) →
    structure-not-symptoms gate → worktree → edit → build/test/lint →
    doc currency → push.
  - Persona dispatch table covers 11 Eigenius personas.
- PM template prompt narrowed for Eigenius:
  - No milestone management (Eigenius isn't sprint-driven)
  - PM is routing-only (triage area + type, hand back to leader for
    engineer dispatch)
  - No `needs-design-doc` label gate (engineer-side architect dispatch
    handles it instead)
- Initiative throttle dropped to `responsive` / 3 per hour.
- Single engineer + single PM in `members:` — scale up after Phase 7.

---

## Phase 6 — Persona files ✅

Eleven persona files written at
[`.claude/agents/`](../../.claude/agents/):

| Persona | Surface |
| --- | --- |
| [`architect`](../../.claude/agents/architect.md) | `docs/design/`, structural decisions, the AST / IRI / layer surfaces |
| [`kernel-engineer`](../../.claude/agents/kernel-engineer.md) | `kernel/` |
| [`storage-engineer`](../../.claude/agents/storage-engineer.md) | `storage/{memory,rocksdb,tikv}` |
| [`wasm-engineer`](../../.claude/agents/wasm-engineer.md) | `crates/wasm-runtime`, `sdk/wasm-sdk`, `wit/` |
| [`institution-engineer`](../../.claude/agents/institution-engineer.md) | Julia + Lean 4 institutions, polyglot |
| [`cli-engineer`](../../.claude/agents/cli-engineer.md) | `cli/` |
| [`orchestration-engineer`](../../.claude/agents/orchestration-engineer.md) | Deno orchestrator at `orchestration/` |
| [`notebook-engineer`](../../.claude/agents/notebook-engineer.md) | React/Vite frontend at `notebooks/` |
| [`qa-engineer`](../../.claude/agents/qa-engineer.md) | Workspace tests, proptest, criterion |
| [`verus-engineer`](../../.claude/agents/verus-engineer.md) | **Advisory only** — no tracked code (spikes are gitignored); methodology opinions feed into the architect persona |
| [`docs-writer`](../../.claude/agents/docs-writer.md) | `docs/` outside `docs/design/` (which the architect owns) |

### Persona model

`opus` for all personas — sub-task quality matters per dispatch. Cost
contained because persona dispatches are bounded tasks invoked from
the engineer's main turn.

### Tool surface

All personas have `Read, Glob, Grep` minimum. Engineers add `Bash,
Write, Edit` for code editing. `architect` adds `Write, Edit, WebFetch`
(can author design docs). `docs-writer` has no `Bash` (text editing
only). `verus-engineer` is read-only (advisory only, never commits).

### Not yet created

- `triage-assistant` — referenced by the PM template but deferred; not
  load-bearing for Phase 7 smoke.
- `.claude/commands/` — slash-command shortcuts (SV has these for
  things like `/adr-new`); not strictly needed since personas dispatch
  via Claude Code's `Task` tool. Can be added later.

---

## Phase 7 — Install package, smoke test ⏸ Pending

### Open question

**How does SV pick up our custom package?** Spring Voyage's catalog
reads from its on-disk `packages/` tree (the bundled
`spring-voyage-oss` package is automatically visible because it ships
in that tree). Our package lives at
[`agents/eigenius-agents/`](../../agents/eigenius-agents/) in this
repo, not under SV's install root.

Three possibilities (to be confirmed once SV is installed):

1. **Symlink:** create a symlink from `~/.spring-voyage/current/packages/eigenius-agents` → `<repo>/agents/eigenius-agents`.
2. **`--from-path` flag** on `spring package install`: install directly
   from a path, no catalog registration.
3. **Copy into the catalog tree.** Brittle (re-copies on every package
   edit) but the most obvious fallback.

Resolved at Phase 7 entry. Not blocking earlier phases.

### Steps once package registration is resolved

```bash
spring package install eigenius-agents \
  --input github_repo=eigenius/eigenius \
  --input github_installation_id=<id>

spring package status <install-id>
spring unit show eigenius-agents
```

### Smoke tests

1. **Direct prompt (no GitHub).** Confirms the unit's mailbox accepts
   messages and the engineer container starts:
   ```bash
   spring message send unit:<unit-id> \
     "Read CLAUDE.md and summarise the project posture in three bullets."
   ```
   Expect: agent clones the repo, reads, replies in the activity feed.

2. **Trivial labelled issue.** Open a bug like *"fix a typo in
   README.md"* on `eigenius/eigenius`, add the
   `eigenius-agents-team` label, watch the activity feed:
   ```bash
   spring activity list --source unit:<unit-id> --limit 20
   ```
   Expect: PM triage event → engineer dispatch → PR opened against
   the repo.

### Exit gate

A real, trivial PR exists from the bot. **STOP HERE** before pressing
into larger work — confirm prompt accuracy, output quality, and labeling
discipline first.

---

## Phase 8 — Real work and iteration ⏸ Pending

Only after Phase 7 succeeds:

- Pick a real, small, well-scoped issue (a bug fix, not a structural
  change). Label it. Observe.
- Tune leader prompt where dispatch misfires.
- Add Verus to the agent image if/when verification work begins in
  earnest (deferred per the Dockerfile's "Deferred additions"
  comment).
- Add personas as scope expands (`triage-assistant`, persona for any
  new sub-discipline).
- Scale `members:` beyond 1+1 only when a single engineer is genuinely
  the bottleneck on parallelisable work.
- Swap haiku → opus on the engineer / PM / leader templates once
  workflow quality matters more than bring-up cost.

### Observation criterion (added during Phase 5 planning)

In the first five PRs the team produces, count how many touched
`docs/design/` in the same PR or alongside on structural surfaces. If
< 60%, the prompts need more work — don't scale up until that ratio
is healthy.

---

## Artifact index

Everything we've authored, in one list:

```
.agents/skills/
  build/SKILL.md
  test/SKILL.md
  lint/SKILL.md

.claude/agents/
  architect.md
  cli-engineer.md
  docs-writer.md
  institution-engineer.md
  kernel-engineer.md
  notebook-engineer.md
  orchestration-engineer.md
  qa-engineer.md
  storage-engineer.md
  verus-engineer.md
  wasm-engineer.md

agents/eigenius-agents/
  package.yaml
  templates/
    program-manager/package.yaml
    software-engineer/package.yaml
  units/
    eigenius-agents/package.yaml

eng/agent-images/
  Dockerfile.engineer

docs/notes/
  spring-voyage-agent-setup.md            ← this file

AGENTS.md
CONVENTIONS.md
```

---

## Known unknowns / risks

| Item | Detail |
| --- | --- |
| **Custom package catalog registration** | How SV discovers our `agents/eigenius-agents/` package — to be resolved at Phase 7. |
| **SV stable release timing** | All references currently pin to pre-release `0.0.0-rc.20260513.4`. When SV cuts a stable release, bump three places: [`Dockerfile.engineer`](../../eng/agent-images/Dockerfile.engineer) base image arg, [`agents/eigenius-agents/package.yaml`](../../agents/eigenius-agents/package.yaml) execution image, [`agents/eigenius-agents/templates/program-manager/package.yaml`](../../agents/eigenius-agents/templates/program-manager/package.yaml) execution image. |
| **Cost discipline** | Five-Opus persistent agents would burn money fast. We're starting with 1+1 on haiku-4-5; review after first real workload before scaling. |
| **`gh-webhook-forward` coupling** | Webhook forwarder needs `gh` working on host. WSL2 DNS is now fixed but the path is fragile; if it breaks again, the team gets webhooks only at GitHub's discretion (no local feedback loop). |
| **Cdylib path break** | If anyone introduces a `CARGO_TARGET_DIR` override that moves builds out of `<repo>/target/debug/`, Lake's hardcoded link path breaks. Documented in [`institution-engineer.md`](../../.claude/agents/institution-engineer.md). |
| **Design-first vs SV velocity bias** | The OSS template originally pushes a "build/test/lint → PR" cadence. We rewrote it to gate on design docs, but the structural integrity of this gate isn't tested until Phase 7 produces a structural PR. Watch closely. |

---

## Resume here

**Next concrete actions (Phase 1):**

1. `sudo apt update && sudo apt install -y podman`
2. `podman --version` (expect 4+)
3. Run the SV installer:
   ```bash
   curl -fSL \
     https://github.com/cvoya-com/spring-voyage/releases/download/spring-voyage-v0.0.0-rc.20260513.4/install-0.0.0-rc.20260513.4.sh \
     | bash
   ```
   - `DEPLOY_HOSTNAME` → `localhost`
   - GitHub App registration → **skip**
4. `voyage status` → all containers green.
5. `claude setup-token` (interactive).
6. `spring secret create --scope tenant anthropic-oauth --value '<token>'`.
7. `docker save eigenius/agent-engineer:dev | podman load`.

Then ping the architect / assistant and move to Phase 2.
