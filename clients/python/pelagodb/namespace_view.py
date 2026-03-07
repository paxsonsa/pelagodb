"""Namespace-scoped client view for PelagoDB typed schema layer.

A NamespaceView wraps a PelagoClient and overrides the namespace in all
RequestContext objects, providing typed CRUD and PQL query methods.
"""

from __future__ import annotations

import uuid
from typing import TYPE_CHECKING, Any, Iterator

from pelagodb.schema import Entity

from pelagodb.watch import WatchStream

if TYPE_CHECKING:
    from pelagodb.client import PelagoClient
    from pelagodb.query import CompoundExpr, FilterExpr


class NamespaceView:
    """Namespace-scoped client view. All operations target a specific namespace."""

    def __init__(self, client: PelagoClient, namespace_name: str) -> None:
        self._client = client
        self._namespace = namespace_name

    def _context(self) -> Any:
        from pelagodb.generated import pelago_pb2

        return pelago_pb2.RequestContext(
            database=self._client.database,
            namespace=self._namespace,
            site_id=self._client.site_id,
            request_id=str(uuid.uuid4()),
        )

    def register_schema(self, model: type[Entity]) -> Any:
        """Register a single Entity schema in this namespace."""
        from pelagodb.client import _schema_dict_to_proto
        from pelagodb.generated import pelago_pb2

        proto_schema = _schema_dict_to_proto(model.to_schema_dict())
        req = pelago_pb2.RegisterSchemaRequest(
            context=self._context(),
            schema=proto_schema,
            index_default_mode=pelago_pb2.INDEX_DEFAULT_MODE_AUTO_BY_TYPE_V1,
        )
        return self._client.schema.RegisterSchema(req, metadata=self._client._metadata())

    def create(self, instance: Entity) -> Entity:
        """Create a node, returning a typed instance with server-assigned ID."""
        from pelagodb.client import _py_to_value
        from pelagodb.generated import pelago_pb2

        req = pelago_pb2.CreateNodeRequest(
            context=self._context(),
            entity_type=type(instance).entity_name(),
            properties={k: _py_to_value(v) for k, v in instance.to_properties_dict().items()},
        )
        resp = self._client.node.CreateNode(req, metadata=self._client._metadata())
        if not resp.node:
            raise RuntimeError("CreateNode returned no node")
        return type(instance).from_node(resp.node, namespace=self._namespace)

    def get(self, model: type[Entity], node_id: str) -> Entity | None:
        """Get a node by ID as a typed instance. Returns None if not found."""
        from pelagodb.generated import pelago_pb2

        req = pelago_pb2.GetNodeRequest(
            context=self._context(),
            entity_type=model.entity_name(),
            node_id=node_id,
            consistency=pelago_pb2.READ_CONSISTENCY_STRONG,
            fields=[],
        )
        resp = self._client.node.GetNode(req, metadata=self._client._metadata())
        if not resp.HasField("node"):
            return None
        return model.from_node(resp.node, namespace=self._namespace)

    def update(self, instance: Entity, **changed: Any) -> Entity:
        """Update properties on an existing entity instance."""
        from pelagodb.client import _py_to_value
        from pelagodb.generated import pelago_pb2

        req = pelago_pb2.UpdateNodeRequest(
            context=self._context(),
            entity_type=type(instance).entity_name(),
            node_id=instance.id,
            properties={k: _py_to_value(v) for k, v in changed.items()},
        )
        resp = self._client.node.UpdateNode(req, metadata=self._client._metadata())
        if not resp.node:
            raise RuntimeError("UpdateNode returned no node")
        return type(instance).from_node(resp.node, namespace=self._namespace)

    def delete(self, instance_or_model: Entity | type[Entity], node_id: str = "") -> bool:
        """Delete a node by instance or by model + ID."""
        from pelagodb.generated import pelago_pb2

        if isinstance(instance_or_model, Entity):
            et = type(instance_or_model).entity_name()
            nid = instance_or_model.id
        else:
            et = instance_or_model.entity_name()
            nid = node_id
        req = pelago_pb2.DeleteNodeRequest(
            context=self._context(), entity_type=et, node_id=nid
        )
        resp = self._client.node.DeleteNode(req, metadata=self._client._metadata())
        return bool(resp.deleted)

    def find(self, model: type[Entity], filter_expr: FilterExpr | CompoundExpr | str | None = None, limit: int = 100) -> Iterator[Entity]:
        """Find nodes matching a filter expression. Returns typed instances."""
        from pelagodb.query import QueryBuilder

        q = QueryBuilder(self, model)
        if filter_expr is not None:
            q = q.filter(filter_expr)
        q = q.limit(limit)
        yield from q.run()

    def pql(self, model: type[Entity], pql_str: str) -> Iterator[Entity]:
        """Execute raw PQL and return typed instances."""
        from pelagodb.generated import pelago_pb2

        req = pelago_pb2.ExecutePQLRequest(
            context=self._context(), pql=pql_str, params={}, explain=False
        )
        for result in self._client.query.ExecutePQL(req, metadata=self._client._metadata()):
            if result.node:
                yield model.from_node(result.node, namespace=self._namespace)

    def query(self, model: type[Entity]) -> Any:
        """Start a QueryBuilder for this namespace."""
        from pelagodb.query import QueryBuilder

        return QueryBuilder(self, model)

    # ------------------------------------------------------------------
    # Watch
    # ------------------------------------------------------------------

    def watch_node(
        self,
        model: type[Entity],
        node_id: str,
        *,
        properties: list[str] | None = None,
        include_initial: bool = False,
        resume_after: bytes = b"",
        ttl_secs: int = 0,
        heartbeat_secs: int = 0,
    ) -> WatchStream:
        """Watch a specific node for changes.

        Returns an async WatchStream context manager. Breaking out of the
        loop or exiting the ``async with`` block cancels the stream and
        cleans up the server-side subscription::

            async with ns.watch_node(Person, alice.id) as events:
                async for event in events:
                    print(event.type, event.node)
                    if event.type == WatchEventType.DELETE:
                        break
        """
        from pelagodb.generated import pelago_pb2

        opts = pelago_pb2.WatchOptions(
            include_initial=include_initial,
            ttl_secs=ttl_secs,
            heartbeat_secs=heartbeat_secs,
        )
        node_watch = pelago_pb2.NodeWatch(
            entity_type=model.entity_name(),
            node_id=node_id,
            properties=properties or [],
        )
        req = pelago_pb2.WatchPointRequest(
            context=self._context(),
            entity_type=model.entity_name(),
            node_id=node_id,
            options=opts,
            resume_after=resume_after,
            nodes=[node_watch],
        )
        async_watch = self._client._async_watch_stub()
        stream = async_watch.WatchPoint(req, metadata=self._client._metadata())
        return WatchStream(stream, self._client, model, self._namespace)

    def watch_query(
        self,
        model: type[Entity],
        filter_expr: FilterExpr | CompoundExpr | str | None = None,
        *,
        pql: str = "",
        include_initial: bool = False,
        resume_after: bytes = b"",
        ttl_secs: int = 0,
        heartbeat_secs: int = 0,
    ) -> WatchStream:
        """Watch a query for changes — get notified when matching results change.

        Accepts either a filter expression (compiled to PQL) or a raw PQL string::

            async with ns.watch_query(Person, Person.age > 30) as events:
                async for event in events:
                    print(event.type, event.node.name)
        """
        from pelagodb.generated import pelago_pb2

        opts = pelago_pb2.WatchOptions(
            include_initial=include_initial,
            ttl_secs=ttl_secs,
            heartbeat_secs=heartbeat_secs,
        )

        pql_str = pql
        cel_str = ""
        if not pql_str and filter_expr is not None:
            from pelagodb.query import QueryBuilder

            q = QueryBuilder(self, model)
            q = q.filter(filter_expr)
            pql_str = q.to_pql()

        req = pelago_pb2.WatchQueryRequest(
            context=self._context(),
            entity_type=model.entity_name(),
            cel_expression=cel_str,
            pql_query=pql_str,
            options=opts,
            resume_after=resume_after,
        )
        async_watch = self._client._async_watch_stub()
        stream = async_watch.WatchQuery(req, metadata=self._client._metadata())
        return WatchStream(stream, self._client, model, self._namespace)

    def watch(
        self,
        *,
        include_initial: bool = False,
        resume_after: bytes = b"",
        ttl_secs: int = 0,
        heartbeat_secs: int = 0,
    ) -> WatchStream:
        """Watch the entire namespace for any changes.

        Returns untyped async events (node/edge payloads are raw protobuf)::

            async with ns.watch() as events:
                async for event in events:
                    print(event.type, event.raw)
        """
        from pelagodb.generated import pelago_pb2

        opts = pelago_pb2.WatchOptions(
            include_initial=include_initial,
            ttl_secs=ttl_secs,
            heartbeat_secs=heartbeat_secs,
        )
        req = pelago_pb2.WatchNamespaceRequest(
            context=self._context(),
            options=opts,
            resume_after=resume_after,
        )
        async_watch = self._client._async_watch_stub()
        stream = async_watch.WatchNamespace(req, metadata=self._client._metadata())
        return WatchStream(stream, self._client, None, self._namespace)
