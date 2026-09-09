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
- [ ] indexer: language + git stage
- [ ] indexer: derived graph + incremental refresh
- [ ] core: classification and scoring
- [ ] packet: Context IR + renderers + budgets
- [ ] cli: prepare, expand, search, explain (spec 16.1 exit)
- [ ] cli: render agents-md, agent-context
- [ ] cli: task start/finish with automatic observations
- [ ] core: proposal validation
- [ ] cli: propose from-git / structured input
- [ ] cli: review, apply, reject
- [ ] cli: export, import, backup, restore
- [ ] docs: agent guide answers + phase 3-5 handoff (spec 16.2 exit)

## Changes made

- `Cargo.toml`, `crates/*/`, `.gitignore`, `Cargo.lock` — workspace scaffold.
- `crates/core` — error types, ULID-based identifiers, blake3 `Hash`, entities/edges/evidence, hash-chained events, pure `project()` fold, TOML `Config`; unit + proptest coverage.
- `crates/store` — SQLite schema + migrations, WAL + busy timeout, single-writer file lock, transactional append with projection rebuild; integration tests incl. crash recovery, corruption refusal, lock retry/timeout, busy retry.
- `crates/cli` — `init`, `status`, `doctor` (clap 4).
- `crates/indexer` — git CLI probe (bounded).

## Validation

- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo fmt --check`
- Pending: verify phase exits 16.1/16.2 live against fixture repositories when the
  corresponding CLI commands exist. They have not been verified yet.

## Next step / handoff

- Continue at the first unchecked slice above. Read the spec (`context-broker-implementation-spec.md`) sections 6-11 for contracts; the crate-level doc comments state boundary rules.

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
- `CONVENTIONS.md` is absent in the repository and checked parent directories.
  `jj status` could not snapshot because `.git/objects` is read-only; no commit
  has been created during this resumption.
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
- Next: language + Git stage, then derived graph/incremental refresh. The current
  Git probe still uses an unbounded subprocess; the next slice must replace it with
  bounded process/output handling before invoking Git against repository data.

```mermaid
flowchart LR
    Rules[Bounded ignore rules] --> Scan[Filesystem scan]
    Scan --> Files[Sorted paths, hashes and bounded text]
    Files --> Parse[Pure document extraction]
    Parse --> Facts[Headings, links, identifiers and routing]
    Facts -. later slice .-> Graph[Evidence-backed graph]
    Graph -. later slice .-> Store[Transactional event append]
```
