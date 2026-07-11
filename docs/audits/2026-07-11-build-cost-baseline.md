# Foundry Build-Cost Baseline

Measured with `make build-metrics`, which cleans and builds a dedicated release target before collecting the package metrics.

| Metric | Baseline |
| --- | ---: |
| Measured at (UTC) | 2026-07-10T17:20:23Z |
| Host | Darwin arm64 |
| Rust | 1.95.0 (59807616e 2026-04-14) |
| Clean release build | 58 seconds |
| Foundry rlib | 69,697,720 bytes |
| Dedicated Cargo target | 1,017,532 KiB |
| Normal dependency packages | 475 |

The target-size value includes Cargo's dedicated build intermediates and is not a shipped binary size. Compare future runs on equivalent hosts and toolchains; treat changes as investigation signals, not automatic feature-gating thresholds. No module was split or gated from this first measurement alone.
