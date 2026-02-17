# Client SDK Guide

PelagoDB exposes a gRPC API (`proto/pelago.proto`) with SDK wrappers in `clients/`.

## Supported SDKs
- Python: `clients/python` (critical)
- Elixir: `clients/elixir` (critical)
- Rust: `clients/rust` (scaffold + usable crate)
- Swift: `clients/swift` (scaffold)

## Protobuf Generation Matrix
- Python:
  - command: `clients/python/scripts/generate_proto.sh`
  - requires: `grpcio-tools`
- Elixir:
  - command: `clients/elixir/scripts/generate_proto.sh`
  - requires: `protoc-gen-elixir`
- Rust:
  - generated on build via `tonic-build`
- Swift:
  - command: `clients/swift/Scripts/generate_proto.sh`
  - requires: `protoc-gen-swift`, `protoc-gen-grpc-swift`

## Python Quickstart
```bash
cd clients/python
python -m venv .venv
source .venv/bin/activate
pip install -e .
pip install -r requirements-dev.txt
./scripts/generate_proto.sh
```

```python
from pelagodb import PelagoClient

client = PelagoClient("127.0.0.1:27615", database="default", namespace="demo")
node = client.create_node("Person", {"name": "Alice", "age": 31})
rows = list(client.find_nodes("Person", "age >= 30", limit=10))
client.close()
```

## Elixir Quickstart
```bash
cd clients/elixir
mix deps.get
./scripts/generate_proto.sh
```

```elixir
alias PelagoDB.Client

client = Client.new(endpoint: "127.0.0.1:27615", database: "default", namespace: "demo")
{:ok, node} = Client.create_node(client, "Person", %{"name" => "Alice", "age" => 31})
{:ok, _stream} = Client.find_nodes(client, "Person", "age >= 30", 100)
```

## Rust Quickstart
```bash
cargo check --manifest-path clients/rust/pelagodb-client/Cargo.toml
```

See `clients/rust/README.md` for async usage examples.

## Swift Quickstart
```bash
cd clients/swift
./Scripts/generate_proto.sh
swift build
```

See `clients/swift/Sources/PelagoDBClient/PelagoClient.swift` for scaffold extension points.

## Auth in SDKs
When `PELAGO_AUTH_REQUIRED=true`, provide one of:
- API key header (`x-api-key`)
- Bearer token (`authorization: Bearer <token>`)

Both Python and Elixir wrappers support auth header injection in client constructors.

## Dataset Loader Integration
The dataset loader uses the Python SDK and can serve as a reference integration:

```bash
python datasets/load_dataset.py social_graph --namespace demo
```
