# Benchmarks

Recorded mainnet, block 26376237. Measured 12 Nov 2026 on an Apple M4 Pro (14 cores, 48 GB),
proving on CPU with `r0vm` 3.0.5.

| Kind | Public: cycles | Of which, the check | Input | Private: proving | Private: peak memory |
|---|---:|---:|---:|---:|---:|
| Account | 153,931 | 135,599 | 4,028 B | 46.0 s | 5.2 GB |
| Storage slot with its account | 270,062 | 244,617 | 7,104 B | 59.4 s | 5.9 GB |
| Receipt log | 61,074 | 49,801 | 1,743 B | 39.6 s | 4.8 GB |
| Each extra condition | +507 to +589 | — | +33 B | — | — |

- **Public: cycles** are the guest's user cycles in the executor, one segment each. The rest of
  each figure, beyond the check, is reading the input (11,719 / 18,832 / 4,681).
- **Each extra condition** is the slope from 1 to 8 conditions: account 153,931 → 158,007,
  storage 270,062 → 274,188, log 61,074 → 64,623.
- **Private** is a succinct receipt from `r0vm` 3.0.5 (223,362 bytes), verified against the image
  ID, timed with `/usr/bin/time -l`. Proven totals are 262,144 / 524,288 / 131,072 cycles, one
  segment each.
- **The verifier crates must match `r0vm`.** `host/Cargo.toml` pins the circuits to the versions
  LEZ `v0.4.0-rc1` locks; left to float, `risc0-circuit-recursion` 4.1.0 refuses `r0vm` 3.0.5's
  receipts (verifier parameters `f57f355f…` against `ece5e9b8…`).

# Quick Instructions

```bash
cd ~/work/caisson/experiments/mpt-zkvm
unset RISC0_DEV_MODE        # with it set, proofs are fake
```

Build (the first time also fetches the crates and RISC Zero's `tiny-keccak` fork):

```bash
cargo build --release --bin mpt
```

Public path: cycles, no proof:

```bash
./target/release/mpt account execute
./target/release/mpt storage execute
./target/release/mpt log execute
```

Private path: real proofs, with peak memory. On macOS:

```bash
/usr/bin/time -l ./target/release/mpt account prove
/usr/bin/time -l ./target/release/mpt storage prove
/usr/bin/time -l ./target/release/mpt log prove
```

On Linux, for example the RTX 4090 machine, use `/usr/bin/time -v` instead of `-l`.

Per-condition cost (compare the cycles at 1, 2 and 8 conditions):

```bash
for n in 1 2 8; do ./target/release/mpt storage execute $n; done
```
