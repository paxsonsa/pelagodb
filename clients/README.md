# PelagoDB Client SDKs

This directory contains language SDKs and generation tooling for `proto/pelago.proto`.

## Priority SDKs
- `clients/python` (critical)
- `clients/elixir` (critical)

## Additional SDK Scaffolds
- `clients/rust`
- `clients/swift`

## Common Pattern
1. Generate language-specific gRPC/protobuf code from `proto/pelago.proto`.
2. Use the provided high-level wrapper client for common operations.
3. For advanced use, call generated stubs directly.

See each SDK README for exact setup and examples.
