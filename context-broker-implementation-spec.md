# Middleman

## Implementation specification

## 1. Product definition

Middleman is a local-first, vendor-neutral context layer for coding agents. It sits between a coding harness and a repository, supplying the smallest reliable context for a task and preserving reviewed project knowledge across tasks and harnesses.

The user continues to use their preferred harness and model: Cline with Qwen, Claude Code, Codex, Pi, Kilo, or a future client. The broker has no model account, API key, or AI inference requirement in its core path.

### Primary outcome

Over time, a project needs fewer fresh input tokens to begin a task because the broker knows:

- repository structure, symbols, dependencies, tests, and Git changes;
- durable decisions, invariants, contracts, ownership, and operational constraints;
- prior task outcomes, validation, open questions, and handoffs; and
- which context was useful for similar work.

### Product principles

- Local-first and private by default. No cloud account, telemetry, query logging, or external AI dependency.
- Harness- and model-neutral. The same repository memory works with every supported client.
- Deterministic core. Code indexing, routing, state transitions, storage, and rendering do not need an LLM.
- Functional core, imperative shell. Immutable project events become validated materialized state; filesystem, Git, harness, and optional AI integration sit at narrow edges.
- Explainable retrieval. Every included context item has a reason; every omitted high-score item is inspectable.
- Earned memory. Durable project facts are reviewed, evidenced, versioned, and reversible. The model does not silently rewrite project truth.
- Token-aware. Stable, compact, task-specific output is the default. The broker never emits a whole-repository summary merely because it can.

### Non-goals for v1

- Replacing a coding harness or code editor.
- Proxying a user’s subscription credentials or intercepting provider traffic.
- Automatic autonomous implementation or deployment.
- Full semantic search, embeddings, vector databases, or a second AI model.
- Replacing source control, issue tracking, documentation, or tests.
- Treating chat transcripts as authoritative project memory.

## 2. User experience

### First-time setup

```bash
middleman init
middleman index
middleman doctor
```

This creates a repository-local `.middleman/` directory, scans the repository under ignore rules, records a baseline, and writes a small `AGENTS.md` integration block.

### Normal task

The user tells their harness:

```text
Implement crawler frontier lease renewal.
```

The harness calls:

```bash
middleman prepare --task "Implement crawler frontier lease renewal" --format markdown
```

The broker returns a compact task packet. The harness reads the named docs, modules, contracts, and tests, then works normally.

At the end of material work, the harness calls:

```bash
middleman propose --from-git --task-id task_01H... --format markdown
```

The broker produces a proposed context diff. The harness or user reviews and accepts it:

```bash
middleman apply proposal_01H...
```

### Important interaction rule

The user should not need to name source files in ordinary prompts. `AGENTS.md` tells the harness to invoke `middleman prepare`; routing and expansion are Middleman's job.

## 3. System architecture

```text
User
  ↕ normal language
Coding harness and selected model
  ↕ MCP tools, CLI, hook, or native adapter
Middleman
  ├── task router
  ├── context packet renderer
  ├── source and Git indexer
  ├── durable-memory event store
  ├── proposal validator
  └── optional adapter host
  ↕
Repository, Git, docs, tests, SQLite database
```

The broker is one local Rust binary. It may expose:

1. a CLI, the universal baseline;
2. a local stdio MCP server, the common cross-harness integration; and
3. optional harness-native adapters for automatic context injection.

Do not begin with a daemon. A short-lived CLI process with SQLite is enough for v1. Add a long-lived local daemon only when indexing or concurrent clients make measured latency unacceptable.

## 4. Repository layout

```text
middleman/
  Cargo.toml
  crates/
    core/             # domain types, pure routing and validation
    store/            # SQLite event store, migrations, snapshots
    indexer/          # filesystem, Git, docs, symbols, tests
    packet/           # Context IR and Markdown/JSON renderers
    cli/              # `middleman` command
    mcp/              # stdio MCP facade
    adapters/         # optional harness-specific adapters
  tests/
    fixtures/
    integration/
  docs/
```

Use current stable Rust, a committed lockfile, SQLite, and ordinary local files. Keep the core free of proprietary services. Use narrow, well-maintained dependencies: `clap`, `serde`, `rusqlite` or `sqlx` with SQLite, `thiserror`, `tracing`, `uuid`, `time`, `blake3`, and a Git library or a carefully bounded Git CLI adapter. Add language parsers behind optional features.

## 5. Local project state

```text
repository/
  AGENTS.md
  .middleman/
    config.toml
    context.sqlite3
    state/
      current.cir
      architecture.md          # generated, optional
    proposals/
    cache/
    ignore
```

`.middleman/context.sqlite3` is local runtime state and should be ignored by Git. The repository may optionally commit human-readable generated files such as `docs/agent-context.md`, decisions, and task notes. Never require committing opaque broker state.

### SQLite concurrency

Use WAL mode, a short busy timeout, one writer transaction at a time, short transactions, and retry for transient `SQLITE_BUSY`. The CLI obtains a file lock around writes. Indexing and reads can remain concurrent. Do not claim multi-writer correctness beyond the explicit single-writer boundary.

## 6. Domain model

### Immutable event log

Every durable mutation is an append-only event. Current state is a materialized projection.

```text
Event
  id
  project_id
  sequence
  occurred_at
  actor             # user | harness:<name> | system
  kind
  payload_json
  evidence_refs
  proposal_id?
  previous_hash
  hash
```

Event kinds:

```text
ProjectInitialized
SourceIndexed
EntityDeclared
EntitySuperseded
EdgeDeclared
TaskStarted
TaskCompleted
TaskAbandoned
DecisionProposed
DecisionAccepted
InvariantProposed
InvariantAccepted
ContractProposed
ContractAccepted
EvidenceAttached
RetrievalObserved
ProposalRejected
```

### Entity types

```text
Module          path, language, responsibility, public surface
Symbol          path, span, kind, signature, visibility
Test            path, scope, command, covered symbols
Document        path, title, kind, authority
Decision        statement, rationale, status, owner, supersedes
Invariant       statement, scope, failure consequence
Contract        inputs, outputs, compatibility, owner
Task            objective, status, scope, validation, handoff
Risk            condition, severity, mitigation, owner
OpenQuestion    question, scope, blocking status
Operation       runbook, environment, recovery path
```

### Edges

```text
OWNS, IMPLEMENTS, CALLS, IMPORTS, TESTS, DOCUMENTS,
CONSTRAINS, SUPERSEDES, DEPENDS_ON, AFFECTS, EVIDENCES,
VALIDATES, RISK_OF
```

### Evidence

Every durable claim must link to one or more evidence references:

```text
GitCommit { sha, paths }
SourceSpan { path, start_line, end_line, content_hash }
TestRun { command, status, timestamp, output_digest }
Document { path, content_hash }
UserApproval { actor, timestamp }
```

## 7. Context IR

Context IR is a compact, stable, human-inspectable interchange format. It is not a clever vowel-stripping language. It uses short keywords, IDs, references, and controlled values so a model can follow it reliably.

Example:

```text
CIR/1
P situer
T task_01HZ lease-renewal
G implement renewal without duplicate active owner
R mod:frontier mod:db con:lease-owner test:frontier-lease dec:time-source
I inv:lease-one-owner inv:pg-time-authoritative
E mod:frontier=>crates/frontier
E con:lease-owner=>docs/contracts/leases.md
V fmt,clippy,frontier-integration
X no-migration,no-production-access
Q stale-worker behavior remains open
```

Rules:

- one statement per line;
- IDs are stable and resolvable locally;
- values contain no secrets or arbitrary raw tool output;
- source locations stay as references, not copied source;
- a compact packet has a hard default budget, initially 2,000 tokens rendered as Markdown or 1,200 tokens rendered as CIR;
- the renderer may produce plain Markdown for harnesses that are less reliable with compact syntax.

Do not prematurely optimize the syntax around a particular tokenizer. Measure rendered token counts across common model tokenizers before shortening fields. Clarity and stable retrieval matter more than shaving characters that tokenize unpredictably.

## 8. Deterministic indexing

### Inputs

- repository path and `.git` metadata;
- `AGENTS.md`, `CLAUDE.md`, README files, architecture docs, ADRs, runbooks, task notes;
- source files not excluded by `.middleman/ignore`, `.gitignore`, and user configuration;
- test files and CI configuration;
- migration directories and package manifests.

### Index stages

1. **Filesystem scan:** classify paths and compute content hashes.
2. **Document scan:** discover headings, links, frontmatter, identifiers, and declared routing tables.
3. **Language scan:** extract declarations, imports, exports, tests, and signatures. Start with regex/lightweight patterns for v1; add Tree-sitter parsers for Rust, PHP, TypeScript, JavaScript, Python, Go, and Elm as optional language packs.
4. **Git scan:** record changed paths, last-touch commits, ownership hints, and co-change edges.
5. **Derived graph:** create Module/Symbol/Test/Document nodes and evidence-backed edges.
6. **Incremental refresh:** re-index only changed hashes and their direct dependency neighborhood.

The indexer must never execute repository code. Parsing untrusted files is bounded by file size, time, and memory limits. Ignore dependencies, build output, caches, generated artifacts, secrets, and user-defined excluded paths.

## 9. Task routing and retrieval

### Task classification

Classify a task by deterministic signals first:

- explicit paths, symbols, issue identifiers, and URLs in the user request;
- task keywords matched against document headings, tags, and routing rules;
- recent Git changes and open task state;
- file/module names, test names, and contract terms;
- user-selected mode when supplied by an adapter.

AI classification is optional and must produce a proposal, never silently change durable routing rules.

### Retrieval scoring

For each candidate node, compute a transparent score:

```text
score =
  exact_path_match * 100
+ exact_symbol_match * 90
+ contract_or_invariant_match * 80
+ explicit_routing_match * 70
+ direct_dependency * 50
+ direct_test_link * 45
+ recent_task_link * 25
+ co-change_link * 15
+ recency_bonus * 5
- ignored_or_stale_penalty
```

Return a packet with:

- top documents, modules, contracts, tests, and open risks;
- the reason each was selected;
- a bounded list of expandable nearby items;
- the recommended validation commands; and
- an explicit statement when the router has low confidence.

### Learning retrieval without AI

Record retrieval outcomes:

```text
retrieved node
expanded node
referenced in agent response or tool request
changed file overlap
test overlap
user accepted/rejected packet
```

Use this to tune weights per repository. The system may propose routing-rule improvements, but changing a durable project rule requires review.

## 10. Context update flow

### Automatic observations

The broker may automatically record non-durable facts such as:

- changed paths and symbols;
- commands run and their exit status;
- test/build result digest;
- task start/end timestamps;
- retrieval outcomes.

### Proposed durable memory

The harness may submit a structured proposal:

```json
{
  "task_id": "task_01HZ",
  "decisions": [{
    "statement": "PostgreSQL time is authoritative for frontier leases.",
    "scope": ["mod:frontier", "mod:db"],
    "evidence": ["source:crates/db/src/lease.rs#L20-L64", "test:frontier-lease"]
  }],
  "invariants": [],
  "open_questions": [],
  "validation": [{"command":"cargo test -p frontier", "status":"passed"}]
}
```

The broker validates schema, evidence existence, duplicates, contradiction with active entities, and affected owners. It writes a proposed diff, not durable state.

Acceptance options:

```bash
middleman review proposal_01H...
middleman apply proposal_01H...
middleman reject proposal_01H... --reason "already covered by decision 014"
```

Policy defaults:

- source/Git observations: automatic;
- task summaries: automatic but editable;
- decisions, invariants, contracts, risk severity, and operational runbooks: review required;
- secrets, raw prompts, raw chat logs, raw request bodies, and personal data: never stored by default.

## 11. CLI interface

```text
middleman init [--name NAME]
middleman index [--full] [--changed]
middleman status
middleman doctor

middleman prepare --task TEXT [--format cir|markdown|json] [--budget N]
middleman expand ID [--format cir|markdown|json]
middleman search TEXT [--type TYPE] [--limit N]
middleman explain packet_ID | node_ID

middleman task start --task TEXT
middleman task finish TASK_ID [--from-git]
middleman propose --task-id ID --from-git [--input FILE]
middleman review PROPOSAL_ID
middleman apply PROPOSAL_ID
middleman reject PROPOSAL_ID --reason TEXT

middleman render agents-md
middleman render agent-context
middleman export [--format jsonl|cir|markdown]
middleman import FILE
middleman backup PATH
middleman restore PATH
```

All machine-facing commands support `--format json`. Errors use stable machine-readable codes and human-readable messages.

## 12. Harness integration

### Universal baseline: CLI plus project instructions

Every supported harness can use the broker through the same repository instruction block:

```md
## Middleman

Before investigating or changing code, call:

`middleman prepare --task "<user request>" --format markdown`

Read the returned packet and follow its selected documentation, modules, contracts, tests, and constraints. Do not scan the repository broadly unless the packet has low confidence or the task cannot be completed safely without expansion.

Use `middleman expand <id>` for additional context. After material work, call `middleman propose --from-git --task-id <id>` and present the proposed durable-memory diff for review. Do not directly rewrite Middleman-managed durable memory.
```

This is portable but cooperative: the model follows the instruction. It does not technically force an external call.

### MCP server

Provide a local stdio MCP server with exactly three initial tools:

```text
middleman_prepare(task, format?, budget?)
middleman_expand(id, format?, budget?)
middleman_propose(task_id, source?)
```

Keep tool descriptions compact. Do not expose a large tool catalog, raw database query access, or generic filesystem tools; those inflate harness prompts and weaken the purpose of the product.

Provide a small optional resource surface:

```text
middleman://project/current
middleman://task/{id}
middleman://node/{id}
```

### Adapter priority

1. **Pi adapter:** first native adapter. Pi extensions can inject context, intercept tool calls, register commands/tools, customize compaction, and persist session state. Use it to automatically call `prepare` after the user submits a task, insert the packet, and propose a context update after a task completes.
2. **Kilo adapter:** second native adapter. Kilo supports local MCP servers and plugins that can add tools, intercept events/tool calls, mutate output, and customize compaction. Start with MCP; add a plugin only for automatic injection and retrieval feedback.
3. **Cline adapter:** MCP/CLI integration first, then a thin hook or extension adapter where it can reliably run a pre-task prepare and post-task proposal. Do not depend on opaque prompt rewriting.
4. **Claude Code adapter:** MCP/CLI integration first, with hook support if it can execute the prepare/propose lifecycle locally.
5. **Codex adapter:** MCP/CLI integration first. `AGENTS.md` is the portable behavioral contract; add native lifecycle automation only where supported and documented.

Native adapters are conveniences. The CLI and MCP protocol are the product contract.

## 13. Configuration

```toml
[project]
name = "example"
language_packs = ["rust", "php", "typescript"]

[privacy]
store_raw_prompts = false
store_raw_tool_output = false
redact_patterns = ["dotenv", "private_key", "bearer_token"]

[routing]
default_packet_budget = 1200
low_confidence_threshold = 0.55
max_initial_documents = 6
max_initial_modules = 8
max_initial_tests = 5

[memory]
durable_changes_require_review = true
task_summary_retention_days = 180

[index]
max_file_bytes = 1048576
respect_gitignore = true
extra_ignore = [".middleman/ignore"]

[optional_ai]
enabled = false
mode = "disabled" # disabled | harness | local | remote
```

## 14. Security and privacy

- Bind local server transports to stdio or loopback only. No network listener by default.
- Never store provider credentials, subscription cookies, or model traffic.
- Do not proxy provider requests in v1. It complicates privacy, authentication, compatibility, and subscription terms while providing little necessary value.
- Apply `.gitignore`, `.middleman/ignore`, and configured sensitive-path deny lists before indexing.
- Redact likely secrets from all stored error messages and diagnostics. Do not persist raw prompt text or raw tool output by default.
- Require explicit user confirmation for any command that writes outside `.middleman/`, modifies tracked documentation, or changes durable state.
- Treat repository files and generated agent proposals as untrusted input. Bound parser work, validate schemas, and escape rendered Markdown.
- Provide `middleman export`, encrypted user-chosen backups, and complete local deletion. No hidden state.

## 15. Tests and observability

### Test classes

- Pure unit tests for scoring, routing, Context IR parsing/rendering, event validation, and projection.
- Property tests for event-log projection determinism and Context IR round trips.
- Fixture repositories: Rust workspace, Laravel app, TypeScript monorepo, Elm frontend, mixed repository, empty repository, and hostile/large files.
- Integration tests for Git incremental indexing, SQLite busy/retry behavior, proposal conflicts, backup/restore, and each MCP tool.
- Golden packet tests: task request → expected selected node IDs and bounded render.
- Adapter contract tests against mocked harness payloads.

### Metrics, stored locally and opt-in for display

```text
packet_tokens_rendered
expanded_tokens_rendered
initial_nodes_selected
retrieval_precision_proxy
fresh_input_estimate
cache_hit_rate_if_reported_by_harness
task_completion_validation_rate
proposal_acceptance_rate
index_duration_ms
```

Never report these remotely by default. The user should be able to see: “this repository used 43% less initial context after 20 completed tasks” and inspect why.

## 16. Delivery plan

### Phase 0: executable skeleton

Deliver:

- Rust workspace, config loading, SQLite migrations, logging, `middleman init`, `status`, and `doctor`.
- Local `.middleman/` layout and conservative ignore behavior.
- Deterministic test harness and fixture repositories.

Exit criteria: a repository can initialize, index files, and recover from an interrupted write without corrupting state.

### Phase 1: deterministic project map

Deliver:

- filesystem, document, manifest, test, and Git indexer;
- Module/Document/Test entities and basic dependency edges;
- `middleman prepare`, `expand`, `search`, and `explain`;
- Markdown and JSON packet renderers;
- generated `docs/agent-context.md` and `AGENTS.md` block.

Exit criteria: in fixture repositories, a task is routed to the right module, direct tests, and docs without full-repository output.

### Phase 2: durable memory and review

Deliver:

- event log, projections, task lifecycle, proposals, review/apply/reject;
- decisions, invariants, contracts, risks, and evidence;
- task-note generation and Git-backed proposed updates;
- export/import/backup/restore.

Exit criteria: two separate harness sessions can complete related work without a chat transcript, using only the broker’s task packet and reviewed state.

### Phase 3: MCP and Pi

Deliver:

- stdio MCP server with the three-tool surface;
- Pi extension for automatic prepare, injection, post-task proposal, and retrieval feedback;
- clear fallbacks when the broker is unavailable.

Exit criteria: Pi can start a normal task with an injected context packet and no manual CLI invocation, while the same repository still works through the universal CLI contract.

### Phase 4: Kilo and Cline

Deliver:

- tested local MCP installation instructions;
- Kilo native plugin for automatic lifecycle behavior;
- Cline adapter where supported, otherwise MCP/CLI workflow;
- end-to-end benchmarks using identical tasks with and without the broker.

Exit criteria: a user can change harnesses without re-indexing or losing project memory.

### Phase 5: learning and optional AI

Deliver:

- deterministic retrieval-weight tuning from outcome signals;
- reviewable suggestions for missing docs, routing rules, and stale decisions;
- opt-in use of the current harness model, local model, or a separate remote model for messy historical summarization.

Exit criteria: optional AI improves retrieval or proposes maintenance, but disabling it leaves all core workflows correct and useful.

## 17. Explicitly defer

Do not add these until real usage justifies them:

- cloud sync or team collaboration;
- embeddings/vector database;
- LLM-powered automatic durable-memory writes;
- code editing or shell execution tools inside the broker;
- agent orchestration/subagents;
- universal request proxying;
- graphical dashboard;
- a new general-purpose programming language.

The compact Context IR is the proving ground. If it becomes expressive, reliable, and valuable enough, a Susumu-based transformation language can later operate over the event log, graph, and task plans. That is a later layer, not the MVP.

## 18. Success criteria

The first public version is successful when it can demonstrate all of the following in a real repository:

1. A new task gets a correct initial packet without a whole-repository scan.
2. A second harness can continue the project without access to the first harness’s chat history.
3. Durable project facts are explainable, evidenced, reviewable, and reversible.
4. The default installation is local, works without extra AI credentials, and does not send repository data anywhere.
5. Measured fresh input context per comparable task falls over time without lowering task completion or validation quality.
