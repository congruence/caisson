# Benchmarks

Recorded mainnet, block 26376237. Measured 12 Nov 2026 on an Apple M4 Pro (14 cores, 48 GB),
proving on CPU with `r0vm` 3.0.5.

| Kind | Public: cycles | Input | Private: proving | Private: peak memory |
|---|---:|---:|---:|---:|
| Account | 153,931 | 4,028 B | 46.0 s | 5.2 GB |
| Storage slot with its account | 270,062 | 7,104 B | 59.4 s | 5.9 GB |
| Receipt log | 61,074 | 1,743 B | 39.6 s | 4.8 GB |
| Each extra condition | +507 to +589 | +33 B | — | — |

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

Per-condition cost (compare the cycles at 1, 2 and 8 conditions):

```bash
for n in 1 2 8; do ./target/release/mpt storage execute $n; done
```
