# MOD-15 · Measurement, tests and release acceptance

[简体中文](README.md) · English · [Architecture](../../README.en.md)

Measurement consumes bounded, redacted observations. An attempt has one accounting owner. Late usage updates that attempt. Parent totals can refer to child costs without adding them twice. Runtime measurements exclude compilation and installation time.

## Runtime behavior

Attribute usage and timing to each physical attempt. Leave missing cache counters unknown. Record static inspection, isolated regressions, live provider requests, installed artifacts and target-device interaction as distinct kinds of evidence.

## Failure and verification

Test duplicate and late usage, genuine zero hits and missing counts. In a long-task acceptance run, identify the actual Host, number of steps, interruptions and terminal evidence. For client UI, record clicks and screenshots from the installed target.

## Implementation

- [nola-measurement/src/lib.rs](../../../../core/nola-measurement/src/lib.rs)
- [tests/suites/dependency_boundary.rs](../../../../apps/cli/tests/suites/dependency_boundary.rs)
- [../scripts/test_mac_layers.sh](../../../../scripts/test_mac_layers.sh)

## Detailed references

[Harness Resource Accounting](HARNESS_RESOURCE_ACCOUNTING.md) · [Measurement](MEASUREMENT.md) · [Performance And Evaluation](PERFORMANCE_AND_EVALUATION.md) · [Real E2E Testing](REAL_E2E_TESTING.md) · [Testing](TESTING.md) · [Testing Architecture](TESTING_ARCHITECTURE.md) · [Mobile-Agent-Test-Plan](mobile-agent-test-plan.md) · [Privacy-Release-Checklist](privacy-release-checklist.md)
