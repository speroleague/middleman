# Repository conventions

Read this file before GitHub or Git operations. `AGENTS.md` governs task planning,
code structure, review, and validation. The implementation specification defines
product behavior; `docs/tasks/001-phases-0-2-cli-product.md` tracks the current work.

## Version control

- Use Jujutsu (`jj`) with the existing Git-backed repository. Do not initialize a
  second repository or mix Git commits with the established Jujutsu workflow.
- Define a single-line conventional commit message before editing code. Complete
  one coherent, validated slice per commit: `type(scope): imperative description`
  or `type: imperative description`. No body, trailers, or agent attribution.
- Inspect the working-copy diff and commit explicit paths using
  `jj commit <paths> -m "type(scope): description"`. Preserve unrelated changes.
- Verify the resulting description and working copy. Do not push, rewrite shared
  history, or change identity configuration unless the user requests it.
- Jujutsu normally snapshots the working copy even for status commands. In a
  restricted environment, use `jj --ignore-working-copy` for read-only inspection;
  it may show stale data until a snapshot is made. Use the supported tool approval
  path for commands that need to write Git metadata. Do not change filesystem ACLs
  to bypass the execution sandbox.

## Validation

Run focused tests first, then these checks before committing a material Rust slice:

```text
cargo test --workspace --offline
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo fmt --all --check
```

Use offline mode when dependencies are cached. If dependencies are missing,
fetch them through the supported permission path and keep `Cargo.lock` committed.
Fixture repositories are input data: tests may read or copy them, but must not
execute their source, install their dependencies, or run their hooks.

Record actual validation and platform limitations in the task note. A passing
build does not establish product exit criteria; exercise the completed workflows.
