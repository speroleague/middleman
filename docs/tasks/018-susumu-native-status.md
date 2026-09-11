# Task: Susumu native status delegation

## Objective

Recognize repositories already using Susumu and expose its native review
attention summary from Middleman's status command without duplicating Susumu's
scanner, artifact parser, or record model.

## Scope

- Files / modules: `crates/cli/src/susumu.rs`, `crates/cli/src/main.rs`, `README.md`
- Explicitly out of scope: importing `.susu` records, writing Susumu artifacts,
  changing Middleman's index, task, proposal, or persistence contracts.

## Contract and operational impact

- Callers / consumers affected: `middleman status` optionally includes a native
  Susumu summary; `middleman doctor` reports detected Susumu markers.
- Data ownership or schema impact: none. Susumu retains ownership of all
  `.susu` artifacts and authored sidecars.
- Compatibility / migration path: unchanged output for repositories without a
  marker. Detected repositories degrade to an availability note if the local
  executable cannot provide its digest.
- Rollback or recovery path: remove this optional status adapter; no state is
  written by detection or delegation.

## Functional design

- Pure rule / transformation / state transition: detect known regular-file
  markers, then deserialize only the versioned `susumu.digest.v1` native
  digest summary.
- Effect boundary or adapter: `susumu digest . --json` executes without a shell,
  with null stdin, a 10-second timeout, a 64-KiB stdout ceiling, and hidden
  stderr through the existing bounded process adapter.
- Intentional mutation or non-determinism, if any: Susumu owns the current
  rescan; Middleman records nothing and treats all output as advisory.

## Decisions

- Delegate the pipe-friendly native `digest` command rather than parsing the
  evolving `.susu` grammar or recreating review/verification rules.
- A detected-but-unavailable executable is informational, not a failed doctor
  check, because Susumu is optional.

## Changes made

- Added bounded detection and native digest delegation.
- Added status and doctor visibility plus unit coverage for marker detection and
  forward-compatible digest deserialization.
- Documented clear tool ownership and graceful fallback.

## Validation

- Command: `cargo test --workspace --offline`
- Result: passed on Windows (including detection, versioned-digest, and
  non-blocking optional-delegation coverage).
- Command: `cargo clippy --workspace --all-targets --offline -- -D warnings`
- Result: passed on Windows.
- Command: `cargo fmt --all --check`
- Result: passed on Windows.
- Platform note: Cargo emitted the existing non-fatal warning that it could not
  canonicalize `C:\\Users\\justi`.

## Next step / handoff

If a later task needs Susumu material inside a Middleman packet, define and
version a narrow Susumu machine contract first. Do not parse `.susu` artifacts
or couple Middleman's index to Susumu scanner internals without that contract.
