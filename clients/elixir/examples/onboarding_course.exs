alias PelagoDB.Client

endpoint = System.get_env("PELAGO_ENDPOINT", "127.0.0.1:27615")
database = System.get_env("PELAGO_DATABASE", "default")
namespace = System.get_env("PELAGO_NAMESPACE", "onboarding.demo")

client = Client.new(endpoint: endpoint, database: database, namespace: namespace)

schema = %{
  "name" => "Person",
  "properties" => %{
    "name" => %{"type" => "string", "required" => true},
    "role" => %{"type" => "string", "index" => "equality"},
    "level" => %{"type" => "int", "index" => "range"}
  }
}

{:ok, _} = Client.register_schema_dict(client, schema)

{:ok, node} =
  Client.create_node(client, "Person", %{
    "name" => "Ex Onboard",
    "role" => "Platform Engineer",
    "level" => 2
  })

{:ok, updated} = Client.update_node(client, "Person", node.id, %{"level" => 3})

IO.puts("elixir created node id=#{node.id}")
IO.puts("elixir updated node id=#{updated.id} level=3")

{:ok, stream} = Client.find_nodes(client, "Person", "level >= 3", 10)
IO.puts("elixir find level >= 3")

Enum.each(stream, fn item ->
  if item.node do
    IO.puts("- #{item.node.id} #{item.node.entity_type}")
  end
end)

{:ok, pql_stream} =
  Client.execute_pql(client, "Person @filter(level >= 3) { uid name role level }")

IO.puts("elixir pql level >= 3")

Enum.each(pql_stream, fn item ->
  if item.node do
    IO.puts("- #{item.node.id}")
  end
end)
