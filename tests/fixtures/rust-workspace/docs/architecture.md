---
id: frontier-architecture
tags: [frontier, lease, ownership]
---
# Frontier architecture

## Routing

| If the task involves | Read first | Then inspect |
| --- | --- | --- |
| lease renewal | `docs/contracts/leases.md` | `crates/alpha/src/frontier.rs` |
| worker output | `README.md` | `crates/beta/src/main.rs` |

## Ownership

The [lease contract](contracts/leases.md#invariants) governs renewal.

```markdown
# This example is not a document heading
[example](not-a-real-document.md)
```
