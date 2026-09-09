# Phase 4 matched-harness benchmark

This protocol measures the same task with and without Middleman. It reports
rendered initial context, task completion, and validation quality separately;
index/cache work counts are not a proxy for token savings.

Running this benchmark is optional follow-up evidence, not a prerequisite for
installing or using the Phase 4 adapters. Until a matched run is recorded, make
no numerical quality or savings claim.

## Controls

- Use a clean matching checkout for each run, the same harness version and
  model/settings, and one task statement verbatim.
- Run the baseline without Middleman. Record the exact initial input the harness
  renders or displays, not its advertised context-window capacity.
- Run the broker condition after `middleman init` and `middleman index --changed`.
  Capture the rendered `middleman_prepare` packet (or its CLI equivalent) and
  use only that packet as the additional initial context.
- Record a pass only when the stated validation commands pass. Score completion
  against the same acceptance criteria for both runs.

## Worksheet

| Field | Baseline | Middleman |
| --- | --- | --- |
| Task ID / verbatim task | | |
| Checkout revision | | |
| Harness, version, model, settings | | |
| Rendered initial input characters | | |
| Rendered initial input estimated tokens | | |
| Completed acceptance criteria | | |
| Validation commands and results | | |
| Human quality review and evidence | | |
| Notes / failures | | |

For Middleman, obtain the packet metadata from the `prepare` command's stderr
(`estimated_tokens`) and retain the packet text only in the transient benchmark
record if project policy permits. For a baseline, record the harness-visible
initial prompt/context characters and estimate tokens with the same declared
method; label it an estimate, not tokenizer telemetry.

## Interpreting results

Compare token or character reduction only alongside equal-or-better completion
and validation quality. A smaller packet with an incomplete task, failed tests,
or weaker review is not a successful result. Do not publish a savings claim
until at least one completed matched run is recorded for each supported harness.
