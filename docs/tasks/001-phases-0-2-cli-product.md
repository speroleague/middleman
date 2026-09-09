# Task: Phases 0-2 — full CLI product

## Objective

Implement spec phases 0-2: a deterministic, local-first `middleman` CLI —
event-sourced state, deterministic indexing and routing, context packets
(CIR/markdown/json), and the durable-memory lifecycle (propose, review,
apply, reject) with export/import/backup/restore.

## Scope

- Files / modules: `crates/core`, `crates/store`, `crates/indexer`, `crates/packet`, `crates/cli`, `tests/fixtures/`, `docs/`
- Explicitly out of scope: MCP facade and adapters (phase 3-4), learning and optional AI (phase 5) — handoff note lands in the final slice; encrypted backups; network listeners of any kind

## Contract and operational impact

- Adds a `.middleman/` runtime layout to any initialized repository (gitignored in middleman's own repo; broker state is never committed in target repos, spec 5).
- Data: `.middleman/context.sqlite3` — append-only hash-chained event log plus materialized projections (spec 5, 6).
- Compatibility: greenfield, none yet. Rollback per target repo: delete `.middleman/`; recovery: `middleman restore PATH`.

## Functional design

- Pure core (`middleman-core`): events, `project()` fold, scoring, validation. No I/O, no clocks, no hidden randomness.
- Edges: `middleman-store` (SQLite WAL, single writer, file lock), `middleman-indexer` (bounded filesystem + git CLI reads, never executes repo code), `middleman-packet` (one pipeline: selected nodes -> typed packet -> renderer).
- One writer rule: every durable state change is an event append plus its projection update in a single transaction.

## Decisions

- rusqlite 0.40 with `bundled` (works on Windows/Linux/macOS with a local C toolchain); bounded git CLI adapter instead of a git library; ULID identifiers matching the spec's `task_01H...` examples; clap 4.6; toml 1.x; proptest for property tests.
- Token estimator (spec 7: no tokenizer tuning in v1): CIR = whitespace-delimited tokens; Markdown = chars/4. Deterministic and surfaced by `explain`.
- Entity id prefixes: `mod:`, `sym:`, `test:`, `doc:`, `dec:`, `inv:`, `con:`, `task:`, `risk:`, `q:`, `op:` for indexer-derived entities; ULID ids with labels for user-declared facts.
- Cross-platform: Windows, Linux, macOS. `PathBuf` everywhere, no POSIX assumptions, no hardcoded separators, no shell strings for file operations.
- VCS: jj. One vertical slice = one commit, single-line conventional message.

## Slice plan

- [x] base docs commit
- [x] workspace scaffold (7 crates, lint config, committed lockfile)
- [x] core: event model with blake3 hash chain and typed payloads
- [x] core: pure `project()` fold to state
- [x] store: schema, migrations, WAL, file lock, busy retry
- [x] cli: init, status, doctor
- [x] fixtures: deterministic test repositories
- [x] indexer: fs + document stage
- [x] indexer: language + git stage
- [x] indexer: derived graph + incremental refresh
- [x] core: classification and scoring
- [x] packet: Context IR + renderers + budgets
- [x] cli: prepare, expand, search, explain (fixture routing verified)
- [x] cli: render agents-md, agent-context
- [x] cli: task start/finish with automatic observations
- [x] core: proposal validation
- [ ] cli: propose from-git / structured input
- [ ] cli: review, apply, reject
- [ ] cli: export, import, backup, restore
- [ ] cli/store: standalone index command and persistent incremental snapshots
- [ ] docs: agent guide answers + phase 3-5 handoff (spec 16.2 exit)

## Changes made

- `Cargo.toml`, `crates/*/`, `.gitignore`, `Cargo.lock` — workspace scaffold.
- `crates/core` — error types, ULID-based identifiers, blake3 `Hash`, entities/edges/evidence, hash-chained events, pure `project()` fold, TOML `Config`; unit + proptest coverage.
- `crates/store` — SQLite schema + migrations, WAL + busy timeout, single-writer file lock, transactional append with projection rebuild; integration tests incl. crash recovery, corruption refusal, lock retry/timeout, busy retry.
- `crates/cli` — `init`, `status`, `doctor` (clap 4).
- `crates/indexer` — bounded filesystem/document/language stages and restricted
  Git observations; `doctor` uses the bounded Git probe.

## Validation

- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo fmt --check`
- Pending: verify phase exits 16.1/16.2 live against fixture repositories when the
  corresponding CLI commands exist. They have not been verified yet.

## Next step / handoff

- Continue at the first unchecked slice above. Read the spec (`context-broker-implementation-spec.md`) sections 6-11 for contracts; the crate-level doc comments state boundary rules.
- Derived graph/refresh is complete; see [the focused handoff](002-derived-graph-refresh.md)
  for the API, cache/evidence contracts, validation and known limits.
- Core classification/scoring is complete; see [the routing handoff](003-classification-scoring.md)
  for signals, ranking, confidence, validation and adapter responsibilities.
  Packet rendering is complete; see [the packet handoff](004-context-packets.md)
  for CIR grammar, budget semantics and validation.
- Retrieval CLI is complete; see [the command handoff](005-retrieval-cli.md)
  for command contracts, prompt-free packet events, validation and current fresh-scan
  behavior. Generated agent guides are complete; see [the guide handoff](006-agent-guides.md)
  for preview/write semantics, preservation and validation. Task lifecycle is complete;
  see [the lifecycle handoff](007-task-lifecycle.md) for atomic batches, observations,
  privacy and validation. Proposal validation is complete; see [the validation handoff]
  (008-proposal-validation.md) for deterministic checks and CLI integration constraints.
  Next: propose, review, apply, and reject. Standalone index/persistent snapshot work
  is explicitly tracked above before final phase-exit validation.

## Language and Git slice

- Planned commit: `feat(indexer): extract language and git observations`.
- Pure language stage: bounded lightweight extraction for Rust, PHP, Elm,
  TypeScript/JavaScript, Python, and Go. Return declarations, imports, visibility,
  signatures and test hints with line references. Unsupported syntax produces no
  invented semantic relationships; this is not a compiler or name resolver.
- Git edge: finite time, output, history and file-count budgets; sanitize errors;
  disable configured clean/process filters, fsmonitor, pagers, external diffs,
  signatures and lazy fetch. No hooks or source code execution, author identities,
  messages, network, or durable writes. Keep parsers separate from process control.
- Caller supplies allowed repository-relative paths from current/previous scans;
  discard other paths. A caller wanting deletion observations must include previous
  accepted paths. History is bounded and exposes truncation, never claims complete
  ownership or complete co-change history. Graph derivation remains the next slice.
- Test process timeout/output overflow with a compiled helper; construct temporary
  Git histories with fixed synthetic identities. Test hostile filter configuration
  by verifying that its marker file is not created.
- Implemented in `crates/indexer/src/language.rs`, `git.rs`, and `process.rs`, with
  focused tests in `tests/language_observations.rs` and `tests/git_observations.rs`
  inside that crate. The process helper runs only through the test executable;
  no helper binary is shipped with the product.
- Validation: `cargo test --workspace --offline` (45 tests),
  `cargo clippy --workspace --all-targets --offline -- -D warnings`, and
  `cargo fmt --all --check` passed on Windows. Git tests cover unchanged index
  bytes, first-parent touch history, deletion/untracked paths, no filter/fsmonitor
  execution, malformed NUL records, truncation, time and output limits.
- Limits: language hints do not cover every syntax form (for example PHP heredocs,
  multiline declarations, macro expansions, or all import/export forms). Git paths
  require UTF-8 without backslashes; unsupported paths fail explicitly. Filters
  are disabled so changes may differ from a user's filtered status. No author
  identity is retained; last-touch commits provide ownership evidence only.
- Compatibility: no database/config migration. `doctor` gains bounded Git process
  handling and sanitized errors. CLI indexing and graph persistence remain pending.
- Next: derived graph and incremental refresh, using these pure observations.

```mermaid
flowchart LR
    Source[Bounded source text] --> Parse[Pure language extraction]
    Allowed[Allowed relative paths] --> Git[Bounded and restricted Git adapter]
    Git --> Observations[Changes and commit touch groups]
    Parse --> Graph[Later graph stage]
    Observations --> Graph
```

## Resumption: filesystem and document indexing

- User authorized edits on 2026-09-09 after scope clarification.
- Planned commits: `test: add deterministic repository fixtures` and
  `feat(indexer): add bounded filesystem and document scanning`.
- Complete the empty Rust fixture files, then implement scanner and pure document
  extraction with focused integration tests. No CLI/persistence wiring in this slice.
- Use the existing typed configuration without changing its serialized contract.
  Add explicit total-byte and entry limits at the scanner boundary. Budget exhaustion
  fails the run; excluded/binary/oversized files have typed skip outcomes.
- Use the `ignore` crate for Git-compatible pattern matching. Load nested rules
  with bounded reads, never traverse links, prune built-in sensitive/build paths
  before reading, and give broker/config exclusions independent precedence.
- Document parsing is a bounded Markdown subset (ATX headings, inline links,
  simple frontmatter, code-span identifiers and routing tables). It does not
  resolve external links or execute repository code.
- Avoid coupling parsing to persistence: owned scan data flows to pure parsers;
  later graph construction and event appends consume those results.
- VCS access resolved: `.git` is not marked read-only on Windows; the execution
  sandbox denied writes. Approved Jujutsu commands successfully snapshot and
  commit without changing ACLs. Follow the new root `CONVENTIONS.md`.
- Completed commits: `4de3bdd5` (fixtures), `0aea6044` (filesystem/document scan).
- Implemented: `crates/indexer/src/scan.rs`, `document.rs`, and
  `crates/indexer/tests/scanning.rs`; completed `tests/fixtures/rust-workspace`.
  `ignore` 0.4.31 is locked in `Cargo.lock`.
- Validation passed on Windows: workspace tests (34 tests, including 8 new
  indexing integration tests and junction rejection), workspace Clippy with
  warnings denied, and workspace format check. The Unix symlink test is
  platform-gated and was not run here. Fixture code is data and was not executed.
- Compatibility: no database migration or CLI behavior change. Markdown extraction
  supports the documented subset, not full CommonMark/YAML. Timing checks are
  cooperative around filesystem reads; this is not a sandbox against concurrent
  malicious filesystem replacement. Default hard-denied names are a conservative
  baseline, not a content-based secret detector.
- Language/Git indexing is now completed above. Continue with derived graph and
  incremental refresh; the Git probe now uses the bounded process adapter.

```mermaid
flowchart LR
    Rules[Bounded ignore rules] --> Scan[Filesystem scan]
    Scan --> Files[Sorted paths, hashes and bounded text]
    Files --> Parse[Pure document extraction]
    Parse --> Facts[Headings, links, identifiers and routing]
    Facts -. later slice .-> Graph[Evidence-backed graph]
    Graph -. later slice .-> Store[Transactional event append]
```
