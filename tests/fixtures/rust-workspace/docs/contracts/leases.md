---
id: lease-owner
authority: contract
---
# Lease ownership contract

## Invariants

Only the current owner can renew a lease that has not expired.
Time is supplied by the caller; renewal does not read a clock.

## Validation

See [renewal tests](../../crates/alpha/tests/frontier_test.rs).
