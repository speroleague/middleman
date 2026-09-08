# Agent context: fill this in once per repository

Keep this file short: ideally 500–1,500 words. It is an index to the system, not a replacement for source code or detailed documentation.

## Purpose

<!-- One paragraph: what this repository does, who it serves, and its main boundary. -->

## Architecture map

| Area | Path | Responsibility | Read when |
| --- | --- | --- | --- |
| Application entrypoint | `path/` | | |
| Core domain | `path/` | | |
| Persistence | `path/` | | |
| API / UI | `path/` | | |
| Tests / fixtures | `path/` | | |
| Operations | `path/` | | |

## Important invariants

- <!-- A rule that must remain true. -->
- <!-- A security, data-integrity, or user-facing guarantee. -->

## Domain model and contracts

| Concept | Canonical representation | Owner | Compatibility / migration notes |
| --- | --- | --- | --- |
| <!-- e.g., account status --> | <!-- typed field / enum / table --> | | |

<!-- List versioned APIs, important error-envelope rules, correlation IDs, pagination limits, or configuration versions here. Link to detailed specifications instead of pasting them. -->

## Effect boundaries

| Effect | Adapter / module | Pure core it serves | Test strategy |
| --- | --- | --- | --- |
| Database | | | |
| HTTP / external API | | | |
| Filesystem / object storage | | | |
| Time, randomness, queue, or process execution | | | |

<!-- Keep domain transformations and state transitions separate from these boundary adapters. -->

## Operational posture

- Rollback / recovery:
- Observability / key signals:
- Performance baseline and known bottlenecks:
- Security-sensitive boundaries:

## Commands

```bash
# setup:
# format:
# lint:
# focused tests:
# full tests:
# build:
# local run:
```

## Decision references

- `docs/...` — <!-- what decision or detail it contains -->

## Working-tree conventions

- <!-- migrations, generated files, fixtures, branching, review, release constraints -->
