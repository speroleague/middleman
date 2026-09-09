# Handoff: harness integration after Phase 2

## What Phase 2 provides

The local CLI is the stable product boundary. It can initialize a repository,
persist a source-free incremental index, return bounded context packets, track
tasks, validate reviewed durable-memory proposals, and transfer a verified event
log. The generated `AGENTS.md` block describes the portable lifecycle:
`index --changed`, `prepare`, optional task start/finish, then proposal/review.

Middleman does not proxy model traffic, run repository code, or become a user
chat interface. The user continues to choose the harness and model.

## Phase 3: MCP and Pi

Implement a local stdio MCP facade over the CLI/core contract with only
`middleman_prepare`, `middleman_expand`, and `middleman_propose`, plus the
documented small resource surface. It must return a clear unavailable fallback
without attempting network recovery or broad filesystem access.

The Pi adapter should call `index --changed` and prepare after task submission,
inject only the bounded packet, and propose after material work. It must preserve
the user's explicit review gate for apply/reject. Add mocked adapter-contract
tests that check packet bounds, unavailable fallback and no raw prompt/tool-output
persistence.

## Phase 4: Kilo and Cline

Start with the MCP protocol and tested local setup instructions. Add native
automation only where the harness has documented lifecycle hooks. Measure the
same task with and without the broker using rendered initial packet size and
completion/validation quality; do not claim savings from cache work counts alone.

## Phase 5: learning and optional AI

Use locally recorded, explainable retrieval outcomes to make reviewable tuning
suggestions. Optional model use must remain off by default and produce proposals,
never silent durable-memory writes. Core retrieval must remain useful with every
optional feature disabled.

## Deferred work and open validation

Do not add cloud sync, embeddings, generic shell/filesystem MCP tools, request
proxying, or orchestration. Before beginning Phase 3, add the Phase 2 exit test:
two separate CLI/harness-process sessions complete related work with only a
packet and reviewed exported/imported state, without sharing a transcript.
