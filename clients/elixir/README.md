# PelagoDB Elixir SDK

Elixir wrapper for PelagoDB gRPC APIs.

## Setup
```bash
cd clients/elixir
mix deps.get
```

## Generate Protobuf Modules
```bash
./scripts/generate_proto.sh
```

This generates Elixir modules under `lib/generated`.

## Schema Registration Defaults
- `Client.register_schema_dict/2` now sets `index_default_mode=:INDEX_DEFAULT_MODE_AUTO_BY_TYPE_V1`.
- If a property omits `index`, the server infers:
  - `int`, `float`, `timestamp` -> `range`
  - `bool` -> `equality`
  - `string`, `bytes` -> `none`
- Use explicit `"index" => "none"` to disable inferred indexing for a property.

## Usage
```elixir
alias PelagoDB.Client

client = Client.new(endpoint: "127.0.0.1:27615", database: "default", namespace: "demo")

{:ok, node} = Client.create_node(client, "Person", %{"name" => "Alice", "age" => 31})
IO.inspect(node.id)

{:ok, stream} = Client.find_nodes(client, "Person", "age >= 30", 100)
Enum.each(stream, fn item -> IO.inspect(item.node.id) end)
```

## Notes
- Header-based auth supported via `api_key` or `bearer_token` options in `Client.new/1`.
- For advanced use, call generated stubs directly.
