"""Async watch stream wrappers for PelagoDB typed schema layer.

Watches use an async gRPC channel (``grpc.aio``) so they can be used with
``async for`` inside an ``async with`` block. Breaking out of the loop or
exiting the context manager cancels both the gRPC stream and the server-side
subscription.

Usage::

    async with acme_ns.watch_query(Person, Person.age > 30) as events:
        async for event in events:
            print(event.type, event.node.name)
            if done:
                break  # triggers cleanup
"""

from __future__ import annotations

import uuid
from enum import Enum
from typing import TYPE_CHECKING, Any, AsyncIterator

if TYPE_CHECKING:
    from pelagodb.client import PelagoClient
    from pelagodb.schema import Entity


class WatchEventType(str, Enum):
    ENTER = "enter"
    UPDATE = "update"
    EXIT = "exit"
    DELETE = "delete"
    HEARTBEAT = "heartbeat"


_EVENT_TYPE_MAP: dict[int, WatchEventType] = {
    1: WatchEventType.ENTER,
    2: WatchEventType.UPDATE,
    3: WatchEventType.EXIT,
    4: WatchEventType.DELETE,
    5: WatchEventType.HEARTBEAT,
}


class TypedWatchEvent:
    """A watch event with an optional typed Entity instance."""

    def __init__(
        self,
        type: WatchEventType,
        subscription_id: str,
        versionstamp: bytes,
        node: Entity | None = None,
        edge: Any = None,
        raw: Any = None,
    ) -> None:
        self.type = type
        self.subscription_id = subscription_id
        self.versionstamp = versionstamp
        self.node = node
        self.edge = edge
        self.raw = raw

    def __repr__(self) -> str:
        node_repr = repr(self.node) if self.node else "None"
        return f"WatchEvent(type={self.type.value!r}, node={node_repr})"


class WatchStream:
    """Async context-managed gRPC watch stream.

    Implements ``AsyncIterator`` and ``AsyncContextManager``. Breaking out
    of the ``async for`` loop or exiting the ``async with`` block cancels
    the gRPC stream and sends ``CancelSubscription`` to the server.
    """

    def __init__(
        self,
        grpc_stream: Any,
        client: PelagoClient,
        model: type[Entity] | None,
        namespace: str,
    ) -> None:
        self._stream = grpc_stream
        self._client = client
        self._model = model
        self._namespace = namespace
        self._subscription_id: str | None = None
        self._cancelled = False

    async def __aenter__(self) -> WatchStream:
        return self

    async def __aexit__(self, exc_type: Any, exc_val: Any, exc_tb: Any) -> None:
        await self.cancel()

    async def cancel(self) -> None:
        """Cancel the watch stream and clean up the server-side subscription."""
        if self._cancelled:
            return
        self._cancelled = True

        # Cancel the gRPC async stream
        try:
            self._stream.cancel()
        except Exception:
            pass

        # Cancel the server-side subscription
        if self._subscription_id:
            try:
                from pelagodb.generated import pelago_pb2

                req = pelago_pb2.CancelSubscriptionRequest(
                    context=pelago_pb2.RequestContext(
                        database=self._client.database,
                        namespace=self._namespace,
                        site_id=self._client.site_id,
                        request_id=str(uuid.uuid4()),
                    ),
                    subscription_id=self._subscription_id,
                )
                async_watch = self._client._async_watch_stub()
                await async_watch.CancelSubscription(req, metadata=self._client._metadata())
            except Exception:
                pass

    def __aiter__(self) -> AsyncIterator[TypedWatchEvent]:
        return self

    async def __anext__(self) -> TypedWatchEvent:
        if self._cancelled:
            raise StopAsyncIteration

        try:
            raw_event = await self._stream.read()
            if raw_event is None:
                raise StopAsyncIteration
        except StopAsyncIteration:
            raise
        except Exception:
            self._cancelled = True
            raise StopAsyncIteration

        # Capture subscription_id from first event
        if raw_event.subscription_id and not self._subscription_id:
            self._subscription_id = raw_event.subscription_id

        event_type = _EVENT_TYPE_MAP.get(raw_event.type, WatchEventType.HEARTBEAT)

        # Convert node to typed instance if we have a model and node data
        typed_node = None
        if self._model and raw_event.HasField("node"):
            typed_node = self._model.from_node(raw_event.node, namespace=self._namespace)

        return TypedWatchEvent(
            type=event_type,
            subscription_id=raw_event.subscription_id,
            versionstamp=raw_event.versionstamp,
            node=typed_node,
            edge=raw_event.edge if raw_event.HasField("edge") else None,
            raw=raw_event,
        )
