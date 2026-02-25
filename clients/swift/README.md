# PelagoDB Swift SDK Scaffold

This package provides a Swift client scaffold for PelagoDB gRPC APIs.

## Requirements
- Swift 5.9+
- `protoc`
- `protoc-gen-swift`
- `protoc-gen-grpc-swift`

## Generate gRPC Sources
```bash
./Scripts/generate_proto.sh
```

Generated files are placed under `Sources/PelagoDBClient/Generated`.

## Build
```bash
swift build
```

## Notes
- This is a scaffold intended for app-team integration.
- Extend `PelagoClient` with additional endpoint wrappers as needed.
