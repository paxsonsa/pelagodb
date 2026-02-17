alias PelagoDB.Client

client = Client.new(endpoint: "127.0.0.1:27615", database: "default", namespace: "demo")

schema = %{
  "name" => "Person",
  "properties" => %{
    "name" => %{"type" => "string", "required" => true},
    "age" => %{"type" => "int", "index" => "range"}
  },
  "edges" => %{
    "follows" => %{"target" => "Person", "direction" => "outgoing"}
  }
}

{:ok, _} = Client.register_schema_dict(client, schema)
{:ok, alice} = Client.create_node(client, "Person", %{"name" => "Alice", "age" => 31})
{:ok, bob} = Client.create_node(client, "Person", %{"name" => "Bob", "age" => 29})
{:ok, _edge} = Client.create_edge(client, "Person", alice.id, "follows", "Person", bob.id)

{:ok, stream} = Client.find_nodes(client, "Person", "age >= 30", 100)
Enum.each(stream, fn item -> IO.inspect(item.node.id) end)
