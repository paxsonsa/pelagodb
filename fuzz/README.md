# PelagoDB Fuzzing

This folder contains `cargo-fuzz` targets and starter corpora for:

- `pql_parser` (PQL parse/resolve/compile)
- `cdc_decode` (CDC CBOR decode/encode roundtrip)
- `mutation_payload` (storage index mutation payloads)

## Prerequisites

```bash
rustup toolchain install nightly
cargo install cargo-fuzz
```

## Run a short smoke pass

```bash
cd fuzz
cargo +nightly fuzz run pql_parser -- -max_total_time=60 -dict=dictionaries/pql.dict
cargo +nightly fuzz run cdc_decode -- -max_total_time=60
cargo +nightly fuzz run mutation_payload -- -max_total_time=60
```

## Reproduce a crash

```bash
cd fuzz
cargo +nightly fuzz run pql_parser artifacts/pql_parser/crash-*
```
