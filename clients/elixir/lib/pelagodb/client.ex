defmodule PelagoDB.Client do
  @moduledoc """
  High-level PelagoDB gRPC client.
  """

  alias Pelago.V1.RequestContext
  alias Pelago.V1.Value

  alias Pelago.V1.{
    SchemaService,
    NodeService,
    EdgeService,
    QueryService,
    AdminService,
    CreateNodeRequest,
    GetNodeRequest,
    UpdateNodeRequest,
    DeleteNodeRequest,
    CreateEdgeRequest,
    DeleteEdgeRequest,
    FindNodesRequest,
    ExecutePQLRequest,
    RegisterSchemaRequest,
    ListSitesRequest,
    QueryAuditLogRequest,
    NodeRef,
    EntitySchema,
    PropertyDef,
    EdgeDef,
    EdgeTarget,
    SchemaMeta
  }

  defstruct endpoint: "127.0.0.1:27615",
            database: "default",
            namespace: "default",
            site_id: "",
            api_key: nil,
            bearer_token: nil

  @type t :: %__MODULE__{}

  def new(opts \\ []) do
    struct!(__MODULE__, opts)
  end

  def register_schema_dict(client, schema_map) when is_map(schema_map) do
    schema = schema_dict_to_proto(schema_map)
    req = %RegisterSchemaRequest{
      context: context(client),
      schema: schema,
      index_default_mode: :INDEX_DEFAULT_MODE_AUTO_BY_TYPE_V1
    }

    with_channel(client, fn ch ->
      SchemaService.Stub.register_schema(ch, req, headers: headers(client))
    end)
  end

  def create_node(client, entity_type, properties) do
    req = %CreateNodeRequest{
      context: context(client),
      entity_type: entity_type,
      properties: Enum.into(properties, %{}, fn {k, v} -> {to_string(k), to_value(v)} end)
    }

    with_channel(client, fn ch ->
      case NodeService.Stub.create_node(ch, req, headers: headers(client)) do
        {:ok, %{node: node}} -> {:ok, node}
        other -> other
      end
    end)
  end

  def get_node(client, entity_type, node_id) do
    req = %GetNodeRequest{
      context: context(client),
      entity_type: entity_type,
      node_id: node_id,
      consistency: :READ_CONSISTENCY_STRONG,
      fields: []
    }

    with_channel(client, fn ch -> NodeService.Stub.get_node(ch, req, headers: headers(client)) end)
  end

  def update_node(client, entity_type, node_id, changed_properties) do
    req = %UpdateNodeRequest{
      context: context(client),
      entity_type: entity_type,
      node_id: node_id,
      properties: Enum.into(changed_properties, %{}, fn {k, v} -> {to_string(k), to_value(v)} end)
    }

    with_channel(client, fn ch ->
      case NodeService.Stub.update_node(ch, req, headers: headers(client)) do
        {:ok, %{node: node}} -> {:ok, node}
        other -> other
      end
    end)
  end

  def delete_node(client, entity_type, node_id) do
    req = %DeleteNodeRequest{context: context(client), entity_type: entity_type, node_id: node_id}

    with_channel(client, fn ch ->
      NodeService.Stub.delete_node(ch, req, headers: headers(client))
    end)
  end

  def create_edge(
        client,
        source_type,
        source_id,
        label,
        target_type,
        target_id,
        properties \\ %{}
      ) do
    req = %CreateEdgeRequest{
      context: context(client),
      source: %NodeRef{entity_type: source_type, node_id: source_id},
      target: %NodeRef{entity_type: target_type, node_id: target_id},
      label: label,
      properties: Enum.into(properties, %{}, fn {k, v} -> {to_string(k), to_value(v)} end)
    }

    with_channel(client, fn ch ->
      EdgeService.Stub.create_edge(ch, req, headers: headers(client))
    end)
  end

  def delete_edge(client, source_type, source_id, label, target_type, target_id) do
    req = %DeleteEdgeRequest{
      context: context(client),
      source: %NodeRef{entity_type: source_type, node_id: source_id},
      target: %NodeRef{entity_type: target_type, node_id: target_id},
      label: label
    }

    with_channel(client, fn ch ->
      EdgeService.Stub.delete_edge(ch, req, headers: headers(client))
    end)
  end

  def find_nodes(client, entity_type, cel_expression \\ "", limit \\ 100) do
    req = %FindNodesRequest{
      context: context(client),
      entity_type: entity_type,
      cel_expression: cel_expression,
      consistency: :READ_CONSISTENCY_STRONG,
      fields: [],
      limit: limit,
      cursor: <<>>
    }

    with_channel(client, fn ch ->
      QueryService.Stub.find_nodes(ch, req, headers: headers(client))
    end)
  end

  def execute_pql(client, pql, explain \\ false) do
    req = %ExecutePQLRequest{context: context(client), pql: pql, params: %{}, explain: explain}

    with_channel(client, fn ch ->
      stub = QueryService.Stub

      cond do
        function_exported?(stub, :execute_pql, 3) ->
          apply(stub, :execute_pql, [ch, req, [headers: headers(client)]])

        function_exported?(stub, :execute_p_q_l, 3) ->
          apply(stub, :execute_p_q_l, [ch, req, [headers: headers(client)]])

        true ->
          {:error, :execute_pql_stub_not_found}
      end
    end)
  end

  def list_sites(client) do
    req = %ListSitesRequest{context: context(client)}

    with_channel(client, fn ch ->
      AdminService.Stub.list_sites(ch, req, headers: headers(client))
    end)
  end

  def query_audit(client, opts \\ []) do
    req = %QueryAuditLogRequest{
      context: context(client),
      principal_id: Keyword.get(opts, :principal_id, ""),
      action: Keyword.get(opts, :action, ""),
      from_timestamp: Keyword.get(opts, :from_timestamp, 0),
      to_timestamp: Keyword.get(opts, :to_timestamp, 0),
      limit: Keyword.get(opts, :limit, 100)
    }

    with_channel(client, fn ch ->
      AdminService.Stub.query_audit_log(ch, req, headers: headers(client))
    end)
  end

  def context(%__MODULE__{} = client) do
    %RequestContext{
      database: client.database,
      namespace: client.namespace,
      site_id: client.site_id,
      request_id: Integer.to_string(System.unique_integer([:positive]))
    }
  end

  defp headers(%__MODULE__{} = client) do
    []
    |> maybe_put_header("x-api-key", client.api_key)
    |> maybe_put_header(
      "authorization",
      if(client.bearer_token, do: "Bearer " <> client.bearer_token, else: nil)
    )
  end

  defp maybe_put_header(headers, _name, nil), do: headers
  defp maybe_put_header(headers, name, value), do: [{name, value} | headers]

  defp with_channel(%__MODULE__{endpoint: endpoint}, fun) do
    with {:ok, channel} <- GRPC.Stub.connect(endpoint) do
      fun.(channel)
    end
  end

  defp to_value(nil), do: %Value{kind: {:null_value, true}}
  defp to_value(v) when is_boolean(v), do: %Value{kind: {:bool_value, v}}
  defp to_value(v) when is_integer(v), do: %Value{kind: {:int_value, v}}
  defp to_value(v) when is_float(v), do: %Value{kind: {:float_value, v}}
  defp to_value(v) when is_binary(v), do: %Value{kind: {:string_value, v}}
  defp to_value(v), do: %Value{kind: {:string_value, to_string(v)}}

  defp schema_dict_to_proto(schema) do
    props =
      schema
      |> Map.get("properties", %{})
      |> Enum.into(%{}, fn {name, p} ->
        base = %PropertyDef{
          type: property_type(p["type"] || "string"),
          required: p["required"] || false
        }

        prop =
          case Map.fetch(p, "index") do
            {:ok, index} -> %PropertyDef{base | index: index_type(index)}
            :error -> base
          end

        {name, prop}
      end)

    edges =
      schema
      |> Map.get("edges", %{})
      |> Enum.into(%{}, fn {name, e} ->
        target =
          case e["target"] do
            "*" -> %EdgeTarget{kind: {:polymorphic, true}}
            t -> %EdgeTarget{kind: {:specific_type, to_string(t)}}
          end

        {name,
         %EdgeDef{
           target: target,
           direction: edge_direction(e["direction"] || "outgoing"),
           sort_key: to_string(e["sort_key"] || ""),
           ownership: ownership_mode(e["ownership"] || "source_site")
         }}
      end)

    meta_map = Map.get(schema, "meta", %{})

    %EntitySchema{
      name: schema["name"],
      version: schema["version"] || 0,
      properties: props,
      edges: edges,
      meta: %SchemaMeta{
        allow_undeclared_edges: Map.get(meta_map, "allow_undeclared_edges", false),
        extras_policy: extras_policy(Map.get(meta_map, "extras_policy", "reject"))
      }
    }
  end

  defp property_type("string"), do: :PROPERTY_TYPE_STRING
  defp property_type("int"), do: :PROPERTY_TYPE_INT
  defp property_type("integer"), do: :PROPERTY_TYPE_INT
  defp property_type("float"), do: :PROPERTY_TYPE_FLOAT
  defp property_type("bool"), do: :PROPERTY_TYPE_BOOL
  defp property_type("timestamp"), do: :PROPERTY_TYPE_TIMESTAMP
  defp property_type("bytes"), do: :PROPERTY_TYPE_BYTES
  defp property_type(_), do: :PROPERTY_TYPE_UNSPECIFIED

  defp index_type("none"), do: :INDEX_TYPE_NONE
  defp index_type("unique"), do: :INDEX_TYPE_UNIQUE
  defp index_type("equality"), do: :INDEX_TYPE_EQUALITY
  defp index_type("range"), do: :INDEX_TYPE_RANGE
  defp index_type(_), do: :INDEX_TYPE_NONE

  defp edge_direction("outgoing"), do: :EDGE_DIRECTION_DEF_OUTGOING
  defp edge_direction("bidirectional"), do: :EDGE_DIRECTION_DEF_BIDIRECTIONAL
  defp edge_direction(_), do: :EDGE_DIRECTION_DEF_OUTGOING

  defp ownership_mode("source_site"), do: :OWNERSHIP_MODE_SOURCE_SITE
  defp ownership_mode("independent"), do: :OWNERSHIP_MODE_INDEPENDENT
  defp ownership_mode(_), do: :OWNERSHIP_MODE_SOURCE_SITE

  defp extras_policy("reject"), do: :EXTRAS_POLICY_REJECT
  defp extras_policy("allow"), do: :EXTRAS_POLICY_ALLOW
  defp extras_policy("warn"), do: :EXTRAS_POLICY_WARN
  defp extras_policy(_), do: :EXTRAS_POLICY_REJECT
end
