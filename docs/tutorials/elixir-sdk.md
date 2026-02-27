# Tutorial: Elixir SDK

End-to-end tutorial using the PelagoDB Elixir SDK for schema registration, CRUD operations, and querying.

## Prerequisites

### Install the SDK

```bash
cd clients/elixir
mix deps.get
./scripts/generate_proto.sh
```

### Ensure server is running

```bash
pelago admin sites   # verify connectivity
```

## Step 1: Connect

```elixir
alias PelagoDB.Client

client = Client.new(
  endpoint: "127.0.0.1:27615",
  database: "default",
  namespace: "demo"
)
```

With authentication:

```elixir
client = Client.new(
  endpoint: "127.0.0.1:27615",
  database: "default",
  namespace: "demo",
  api_key: "dev-admin-key"
)
```

## Step 2: Register a Schema

```elixir
schema = %{
  "name" => "Person",
  "properties" => %{
    "name" => %{"type" => "string", "required" => true},
    "email" => %{"type" => "string", "index" => "unique"},
    "age" => %{"type" => "int", "index" => "range"},
    "active" => %{"type" => "bool"}
  },
  "meta" => %{
    "allow_undeclared_edges" => true,
    "extras_policy" => "allow"
  }
}

{:ok, _} = Client.register_schema(client, schema)
```

## Step 3: Create Nodes

```elixir
{:ok, alice} = Client.create_node(client, "Person", %{
  "name" => "Alice",
  "email" => "alice@example.com",
  "age" => 32
})
IO.puts("Created Alice: #{alice.node_id}")

{:ok, bob} = Client.create_node(client, "Person", %{
  "name" => "Bob",
  "email" => "bob@example.com",
  "age" => 29
})
IO.puts("Created Bob: #{bob.node_id}")
```

## Step 4: Query Nodes

### CEL Filter

```elixir
{:ok, stream} = Client.find_nodes(client, "Person", "age >= 30", 10)

stream
|> Enum.each(fn node ->
  IO.inspect(node, label: "Found")
end)
```

### Stream Consumption

Elixir's stream handling integrates naturally with PelagoDB's gRPC streaming responses:

```elixir
{:ok, stream} = Client.find_nodes(client, "Person", "age >= 25", 100)

results = Enum.to_list(stream)
IO.puts("Found #{length(results)} people")
```

## Step 5: Update and Delete

```elixir
# Update
{:ok, _} = Client.update_node(client, "Person", alice.node_id, %{"age" => 33})

# Get
{:ok, updated} = Client.get_node(client, "Person", alice.node_id)
IO.inspect(updated.properties, label: "Updated Alice")

# Delete
{:ok, _} = Client.delete_node(client, "Person", bob.node_id)
```

## Complete Example

```elixir
alias PelagoDB.Client

client = Client.new(endpoint: "127.0.0.1:27615", database: "default", namespace: "demo")

# Register schema
{:ok, _} = Client.register_schema(client, %{
  "name" => "Person",
  "properties" => %{
    "name" => %{"type" => "string", "required" => true},
    "email" => %{"type" => "string", "index" => "unique"},
    "age" => %{"type" => "int", "index" => "range"}
  },
  "meta" => %{"allow_undeclared_edges" => true, "extras_policy" => "allow"}
})

# Create data
{:ok, alice} = Client.create_node(client, "Person", %{
  "name" => "Alice", "email" => "alice@example.com", "age" => 32
})
{:ok, bob} = Client.create_node(client, "Person", %{
  "name" => "Bob", "email" => "bob@example.com", "age" => 29
})

# Query
{:ok, stream} = Client.find_nodes(client, "Person", "age >= 25", 10)
results = Enum.to_list(stream)
IO.puts("Found #{length(results)} people age >= 25")
```

## Running as a Script

```bash
cd clients/elixir
PELAGO_ENDPOINT=127.0.0.1:27615 PELAGO_DATABASE=default PELAGO_NAMESPACE=demo \
  mix run examples/onboarding_course.exs
```

## Related

- [gRPC API Reference](../reference/grpc-api.md) — underlying API
- [Python SDK Tutorial](python-sdk.md) — Python equivalent
- [Build a Social Graph](build-a-social-graph.md) — CLI-based tutorial
- [Onboarding Course](onboarding-course.md) — guided 60-min course
