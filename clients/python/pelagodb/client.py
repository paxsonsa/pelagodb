from __future__ import annotations

import datetime as dt
import uuid
from typing import Any, Dict, Iterable, Iterator, List, Optional

import grpc

from pelagodb.generated import pelago_pb2
from pelagodb.generated import pelago_pb2_grpc


_PROPERTY_TYPE = {
    "string": pelago_pb2.PROPERTY_TYPE_STRING,
    "int": pelago_pb2.PROPERTY_TYPE_INT,
    "integer": pelago_pb2.PROPERTY_TYPE_INT,
    "float": pelago_pb2.PROPERTY_TYPE_FLOAT,
    "double": pelago_pb2.PROPERTY_TYPE_FLOAT,
    "bool": pelago_pb2.PROPERTY_TYPE_BOOL,
    "boolean": pelago_pb2.PROPERTY_TYPE_BOOL,
    "timestamp": pelago_pb2.PROPERTY_TYPE_TIMESTAMP,
    "bytes": pelago_pb2.PROPERTY_TYPE_BYTES,
}

_INDEX_TYPE = {
    "none": pelago_pb2.INDEX_TYPE_NONE,
    "unique": pelago_pb2.INDEX_TYPE_UNIQUE,
    "equality": pelago_pb2.INDEX_TYPE_EQUALITY,
    "range": pelago_pb2.INDEX_TYPE_RANGE,
}

_EDGE_DIRECTION_DEF = {
    "outgoing": pelago_pb2.EDGE_DIRECTION_DEF_OUTGOING,
    "bidirectional": pelago_pb2.EDGE_DIRECTION_DEF_BIDIRECTIONAL,
}

_OWNERSHIP_MODE = {
    "source_site": pelago_pb2.OWNERSHIP_MODE_SOURCE_SITE,
    "independent": pelago_pb2.OWNERSHIP_MODE_INDEPENDENT,
}

_EXTRAS_POLICY = {
    "reject": pelago_pb2.EXTRAS_POLICY_REJECT,
    "allow": pelago_pb2.EXTRAS_POLICY_ALLOW,
    "warn": pelago_pb2.EXTRAS_POLICY_WARN,
}


class PelagoClient:
    """High-level synchronous gRPC client for PelagoDB.

    Generated stubs must exist under `pelagodb/generated`.
    Use `clients/python/scripts/generate_proto.sh` first.
    """

    def __init__(
        self,
        endpoint: str,
        *,
        database: str = "default",
        namespace: str = "default",
        site_id: str = "",
        bearer_token: Optional[str] = None,
        api_key: Optional[str] = None,
        insecure: bool = True,
    ) -> None:
        self.database = database
        self.namespace = namespace
        self.site_id = site_id
        self._bearer_token = bearer_token
        self._api_key = api_key

        self._endpoint = endpoint
        self._insecure = insecure

        if insecure:
            self._channel = grpc.insecure_channel(endpoint)
        else:
            raise ValueError("TLS channel wiring is not implemented in this scaffold")

        self.schema = pelago_pb2_grpc.SchemaServiceStub(self._channel)
        self.node = pelago_pb2_grpc.NodeServiceStub(self._channel)
        self.edge = pelago_pb2_grpc.EdgeServiceStub(self._channel)
        self.query = pelago_pb2_grpc.QueryServiceStub(self._channel)
        self.admin = pelago_pb2_grpc.AdminServiceStub(self._channel)
        self.auth = pelago_pb2_grpc.AuthServiceStub(self._channel)
        self.watch = pelago_pb2_grpc.WatchServiceStub(self._channel)
        self.replication = pelago_pb2_grpc.ReplicationServiceStub(self._channel)
        self.health = pelago_pb2_grpc.HealthServiceStub(self._channel)

        # Lazy async channel for watch streams
        self._async_channel: Optional[grpc.aio.Channel] = None

    def _ensure_async_channel(self) -> grpc.aio.Channel:
        """Lazily create an async gRPC channel for watch streams."""
        if self._async_channel is None:
            if self._insecure:
                self._async_channel = grpc.aio.insecure_channel(self._endpoint)
            else:
                raise ValueError("TLS channel wiring is not implemented in this scaffold")
        return self._async_channel

    def _async_watch_stub(self) -> pelago_pb2_grpc.WatchServiceStub:
        """Get a WatchServiceStub backed by the async channel."""
        return pelago_pb2_grpc.WatchServiceStub(self._ensure_async_channel())

    def close(self) -> None:
        self._channel.close()
        if self._async_channel is not None:
            # grpc.aio channels are closed via .close(); safe to call sync
            self._async_channel.close()

    def _context(self) -> pelago_pb2.RequestContext:
        return pelago_pb2.RequestContext(
            database=self.database,
            namespace=self.namespace,
            site_id=self.site_id,
            request_id=str(uuid.uuid4()),
        )

    def _metadata(self) -> List[tuple[str, str]]:
        md: List[tuple[str, str]] = []
        if self._api_key:
            md.append(("x-api-key", self._api_key))
        if self._bearer_token:
            md.append(("authorization", f"Bearer {self._bearer_token}"))
        return md

    # ----------------------
    # Auth
    # ----------------------

    def authenticate(self, *, username: str = "", api_key: str = "") -> pelago_pb2.AuthenticateResponse:
        req = pelago_pb2.AuthenticateRequest(username=username, api_key=api_key)
        resp = self.auth.Authenticate(req, metadata=self._metadata())
        return resp

    def validate_token(self, token: str) -> pelago_pb2.ValidateTokenResponse:
        req = pelago_pb2.ValidateTokenRequest(token=token)
        return self.auth.ValidateToken(req, metadata=self._metadata())

    # ----------------------
    # Schema
    # ----------------------

    def register_schema_dict(self, schema: Dict[str, Any]) -> pelago_pb2.RegisterSchemaResponse:
        proto_schema = _schema_dict_to_proto(schema)
        req = pelago_pb2.RegisterSchemaRequest(
            context=self._context(),
            schema=proto_schema,
            index_default_mode=pelago_pb2.INDEX_DEFAULT_MODE_AUTO_BY_TYPE_V1,
        )
        return self.schema.RegisterSchema(req, metadata=self._metadata())

    def get_schema(self, entity_type: str, version: int = 0) -> pelago_pb2.GetSchemaResponse:
        req = pelago_pb2.GetSchemaRequest(
            context=self._context(), entity_type=entity_type, version=version
        )
        return self.schema.GetSchema(req, metadata=self._metadata())

    # ----------------------
    # Nodes
    # ----------------------

    def create_node(self, entity_type: str, properties: Dict[str, Any]) -> pelago_pb2.Node:
        req = pelago_pb2.CreateNodeRequest(
            context=self._context(),
            entity_type=entity_type,
            properties={k: _py_to_value(v) for k, v in properties.items()},
        )
        resp = self.node.CreateNode(req, metadata=self._metadata())
        if not resp.node:
            raise RuntimeError("CreateNode returned no node")
        return resp.node

    def get_node(self, entity_type: str, node_id: str) -> Optional[pelago_pb2.Node]:
        req = pelago_pb2.GetNodeRequest(
            context=self._context(),
            entity_type=entity_type,
            node_id=node_id,
            consistency=pelago_pb2.READ_CONSISTENCY_STRONG,
            fields=[],
        )
        resp = self.node.GetNode(req, metadata=self._metadata())
        return resp.node if resp.HasField("node") else None

    def update_node(
        self, entity_type: str, node_id: str, changed_properties: Dict[str, Any]
    ) -> pelago_pb2.Node:
        req = pelago_pb2.UpdateNodeRequest(
            context=self._context(),
            entity_type=entity_type,
            node_id=node_id,
            properties={k: _py_to_value(v) for k, v in changed_properties.items()},
        )
        resp = self.node.UpdateNode(req, metadata=self._metadata())
        if not resp.node:
            raise RuntimeError("UpdateNode returned no node")
        return resp.node

    def delete_node(self, entity_type: str, node_id: str) -> bool:
        req = pelago_pb2.DeleteNodeRequest(
            context=self._context(), entity_type=entity_type, node_id=node_id
        )
        resp = self.node.DeleteNode(req, metadata=self._metadata())
        return bool(resp.deleted)

    # ----------------------
    # Edges
    # ----------------------

    def create_edge(
        self,
        source_type: str,
        source_id: str,
        label: str,
        target_type: str,
        target_id: str,
        properties: Optional[Dict[str, Any]] = None,
    ) -> pelago_pb2.Edge:
        req = pelago_pb2.CreateEdgeRequest(
            context=self._context(),
            source=pelago_pb2.NodeRef(entity_type=source_type, node_id=source_id),
            target=pelago_pb2.NodeRef(entity_type=target_type, node_id=target_id),
            label=label,
            properties={k: _py_to_value(v) for k, v in (properties or {}).items()},
        )
        resp = self.edge.CreateEdge(req, metadata=self._metadata())
        if not resp.edge:
            raise RuntimeError("CreateEdge returned no edge")
        return resp.edge

    def delete_edge(
        self, source_type: str, source_id: str, label: str, target_type: str, target_id: str
    ) -> bool:
        req = pelago_pb2.DeleteEdgeRequest(
            context=self._context(),
            source=pelago_pb2.NodeRef(entity_type=source_type, node_id=source_id),
            target=pelago_pb2.NodeRef(entity_type=target_type, node_id=target_id),
            label=label,
        )
        resp = self.edge.DeleteEdge(req, metadata=self._metadata())
        return bool(resp.deleted)

    # ----------------------
    # Query
    # ----------------------

    def find_nodes(self, entity_type: str, cel_expression: str = "", limit: int = 100) -> Iterator[pelago_pb2.Node]:
        req = pelago_pb2.FindNodesRequest(
            context=self._context(),
            entity_type=entity_type,
            cel_expression=cel_expression,
            consistency=pelago_pb2.READ_CONSISTENCY_STRONG,
            fields=[],
            limit=limit,
            cursor=b"",
        )
        stream = self.query.FindNodes(req, metadata=self._metadata())
        for item in stream:
            if item.HasField("node"):
                yield item.node

    def execute_pql(self, pql: str, explain: bool = False) -> Iterator[pelago_pb2.PQLResult]:
        req = pelago_pb2.ExecutePQLRequest(
            context=self._context(), pql=pql, params={}, explain=explain
        )
        return self.query.ExecutePQL(req, metadata=self._metadata())

    # ----------------------
    # Typed schema layer
    # ----------------------

    def register(self, namespace_or_model) -> None:
        """Register a Namespace (all its entities) or a single Entity.

        For Namespace: registers all inner Entity classes in that namespace.
        For Entity without namespace: registers in the client's default namespace.
        """
        from pelagodb.namespace_view import NamespaceView
        from pelagodb.schema import Entity, Namespace

        if isinstance(namespace_or_model, type) and issubclass(namespace_or_model, Namespace):
            ns_cls = namespace_or_model
            ns_name = ns_cls.__pelago_ns_resolved_name__
            if ns_name is None:
                raise ValueError(
                    f"Cannot register templated namespace {ns_cls.__name__}. "
                    f"Call .bind() first: {ns_cls.__name__}.bind(...)"
                )
            view = NamespaceView(self, ns_name)
            for entity_cls in ns_cls.__pelago_entities__.values():
                view.register_schema(entity_cls)
        elif isinstance(namespace_or_model, type) and issubclass(namespace_or_model, Entity):
            self.register_schema_dict(namespace_or_model.to_schema_dict())
        else:
            raise TypeError(f"Expected Namespace or Entity subclass, got {type(namespace_or_model)}")

    def ns(self, namespace) -> "NamespaceView":
        """Get a namespace-scoped view of this client.

        Args:
            namespace: Namespace subclass, bound namespace, or string name.
        """
        from pelagodb.namespace_view import NamespaceView
        from pelagodb.schema import Namespace

        if isinstance(namespace, str):
            return NamespaceView(self, namespace)
        if isinstance(namespace, type) and issubclass(namespace, Namespace):
            ns_name = namespace.__pelago_ns_resolved_name__
            if ns_name is None:
                raise ValueError(
                    f"Cannot scope to templated namespace {namespace.__name__}. "
                    f"Call .bind() first."
                )
            return NamespaceView(self, ns_name)
        raise TypeError(f"Expected Namespace subclass or string, got {type(namespace)}")

    def link(self, source, label: str, target, properties=None):
        """Create an edge between Entity instances. Infers namespaces from instances.

        Cross-namespace edges are supported: if source and target are in
        different namespaces, the edge is created with proper cross-namespace NodeRef.
        """
        src_ns = source._namespace or self.namespace
        tgt_ns = target._namespace or self.namespace

        context = pelago_pb2.RequestContext(
            database=self.database,
            namespace=src_ns,
            site_id=self.site_id,
            request_id=str(uuid.uuid4()),
        )

        tgt_ref = pelago_pb2.NodeRef(
            entity_type=type(target).entity_name(),
            node_id=target.id,
        )
        if tgt_ns != src_ns:
            tgt_ref.database = self.database
            tgt_ref.namespace = tgt_ns

        req = pelago_pb2.CreateEdgeRequest(
            context=context,
            source=pelago_pb2.NodeRef(
                entity_type=type(source).entity_name(),
                node_id=source.id,
            ),
            target=tgt_ref,
            label=label,
            properties={k: _py_to_value(v) for k, v in (properties or {}).items()},
        )
        resp = self.edge.CreateEdge(req, metadata=self._metadata())
        if not resp.edge:
            raise RuntimeError("CreateEdge returned no edge")
        return resp.edge

    def unlink(self, source, label: str, target) -> bool:
        """Delete an edge between Entity instances. Infers namespaces."""
        src_ns = source._namespace or self.namespace
        tgt_ns = target._namespace or self.namespace

        context = pelago_pb2.RequestContext(
            database=self.database,
            namespace=src_ns,
            site_id=self.site_id,
            request_id=str(uuid.uuid4()),
        )

        tgt_ref = pelago_pb2.NodeRef(
            entity_type=type(target).entity_name(),
            node_id=target.id,
        )
        if tgt_ns != src_ns:
            tgt_ref.database = self.database
            tgt_ref.namespace = tgt_ns

        req = pelago_pb2.DeleteEdgeRequest(
            context=context,
            source=pelago_pb2.NodeRef(
                entity_type=type(source).entity_name(),
                node_id=source.id,
            ),
            target=tgt_ref,
            label=label,
        )
        resp = self.edge.DeleteEdge(req, metadata=self._metadata())
        return bool(resp.deleted)

    # ----------------------
    # Admin
    # ----------------------

    def list_sites(self) -> pelago_pb2.ListSitesResponse:
        return self.admin.ListSites(
            pelago_pb2.ListSitesRequest(context=self._context()),
            metadata=self._metadata(),
        )

    def replication_status(self) -> pelago_pb2.GetReplicationStatusResponse:
        return self.admin.GetReplicationStatus(
            pelago_pb2.GetReplicationStatusRequest(context=self._context()),
            metadata=self._metadata(),
        )

    def query_audit(
        self,
        *,
        principal_id: str = "",
        action: str = "",
        from_timestamp: int = 0,
        to_timestamp: int = 0,
        limit: int = 100,
    ) -> pelago_pb2.QueryAuditLogResponse:
        req = pelago_pb2.QueryAuditLogRequest(
            context=self._context(),
            principal_id=principal_id,
            action=action,
            from_timestamp=from_timestamp,
            to_timestamp=to_timestamp,
            limit=limit,
        )
        return self.admin.QueryAuditLog(req, metadata=self._metadata())


# ----------------------
# Value helpers
# ----------------------


def _py_to_value(value: Any) -> pelago_pb2.Value:
    if value is None:
        return pelago_pb2.Value(null_value=True)
    if isinstance(value, bool):
        return pelago_pb2.Value(bool_value=value)
    if isinstance(value, int):
        return pelago_pb2.Value(int_value=value)
    if isinstance(value, float):
        return pelago_pb2.Value(float_value=value)
    if isinstance(value, bytes):
        return pelago_pb2.Value(bytes_value=value)
    if isinstance(value, dt.datetime):
        micros = int(value.timestamp() * 1_000_000)
        return pelago_pb2.Value(timestamp_value=micros)
    return pelago_pb2.Value(string_value=str(value))


def value_to_py(value: pelago_pb2.Value) -> Any:
    kind = value.WhichOneof("kind")
    if kind == "null_value":
        return None
    if kind == "bool_value":
        return value.bool_value
    if kind == "int_value":
        return value.int_value
    if kind == "float_value":
        return value.float_value
    if kind == "bytes_value":
        return value.bytes_value
    if kind == "timestamp_value":
        return value.timestamp_value
    if kind == "string_value":
        return value.string_value
    return None


# ----------------------
# Schema helper
# ----------------------


def _schema_dict_to_proto(schema: Dict[str, Any]) -> pelago_pb2.EntitySchema:
    entity = pelago_pb2.EntitySchema(
        name=schema["name"],
        version=int(schema.get("version", 0)),
        created_by=str(schema.get("created_by", "")),
    )

    for prop_name, prop in schema.get("properties", {}).items():
        p = pelago_pb2.PropertyDef(
            type=_PROPERTY_TYPE.get(str(prop.get("type", "string")).lower(), pelago_pb2.PROPERTY_TYPE_UNSPECIFIED),
            required=bool(prop.get("required", False)),
        )
        if "index" in prop:
            p.index = _INDEX_TYPE.get(str(prop.get("index", "none")).lower(), pelago_pb2.INDEX_TYPE_NONE)
        if "default" in prop:
            p.default_value.CopyFrom(_py_to_value(prop["default"]))
        entity.properties[prop_name].CopyFrom(p)

    for edge_name, edge in schema.get("edges", {}).items():
        target = edge.get("target", "*")
        target_proto = pelago_pb2.EdgeTarget()
        if target == "*":
            target_proto.polymorphic = True
        else:
            target_proto.specific_type = str(target)

        e = pelago_pb2.EdgeDef(
            target=target_proto,
            direction=_EDGE_DIRECTION_DEF.get(
                str(edge.get("direction", "outgoing")).lower(),
                pelago_pb2.EDGE_DIRECTION_DEF_OUTGOING,
            ),
            sort_key=str(edge.get("sort_key", "")),
            ownership=_OWNERSHIP_MODE.get(
                str(edge.get("ownership", "source_site")).lower(),
                pelago_pb2.OWNERSHIP_MODE_SOURCE_SITE,
            ),
        )
        entity.edges[edge_name].CopyFrom(e)

    meta = schema.get("meta")
    if meta:
        entity.meta.CopyFrom(
            pelago_pb2.SchemaMeta(
                allow_undeclared_edges=bool(meta.get("allow_undeclared_edges", False)),
                extras_policy=_EXTRAS_POLICY.get(
                    str(meta.get("extras_policy", "reject")).lower(),
                    pelago_pb2.EXTRAS_POLICY_REJECT,
                ),
            )
        )

    return entity
