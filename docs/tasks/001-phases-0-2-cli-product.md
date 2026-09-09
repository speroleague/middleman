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
- [ ] fixtures: deterministic test repositories
- [ ] indexer: fs + document stage
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
- Phase exits 16.1/16.2 verified live against fixture repositories, not inferred.

## Next step / handoff

- Continue at the first unchecked slice above. Read the spec (`context-broker-implementation-spec.md`) sections 6-11 for contracts; the crate-level doc comments state boundary rules.
