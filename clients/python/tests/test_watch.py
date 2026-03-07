"""Unit tests for the PelagoDB async watch stream layer (no server required)."""

from __future__ import annotations

import asyncio
from unittest.mock import AsyncMock, MagicMock

import pytest

from pelagodb.watch import TypedWatchEvent, WatchEventType, WatchStream
from pelagodb.schema import Entity, Namespace, Property
from pelagodb.types import IndexType


# ---------------------------------------------------------------------------
# Test entities
# ---------------------------------------------------------------------------


class WatchNS(Namespace):
    name = "watch_test"

    class Item(Entity):
        label: str = Property(required=True)
        count: int = Property(default=0)


def _make_raw_event(*, event_type=2, sub_id="sub-1", node_id="n1", label="hello", count=5):
    """Build a mock WatchEvent protobuf."""
    ev = MagicMock()
    ev.type = event_type
    ev.subscription_id = sub_id
    ev.versionstamp = b"\x01\x02"
    ev.HasField = lambda f: f == "node"

    node = MagicMock()
    node.id = node_id
    node.created_at = 1000
    node.updated_at = 2000

    label_val = MagicMock()
    label_val.WhichOneof.return_value = "string_value"
    label_val.string_value = label

    count_val = MagicMock()
    count_val.WhichOneof.return_value = "int_value"
    count_val.int_value = count

    node.properties = {"label": label_val, "count": count_val}
    ev.node = node
    return ev


def _make_mock_async_stream(events):
    """Build a mock async gRPC stream that returns events via .read()."""
    stream = MagicMock()
    iterator = iter(events)

    async def mock_read():
        try:
            return next(iterator)
        except StopIteration:
            return None

    stream.read = mock_read
    stream.cancel = MagicMock()
    return stream


def _make_mock_client():
    client = MagicMock()
    client.database = "default"
    client.namespace = "watch_test"
    client.site_id = ""
    client._metadata.return_value = []

    # Mock the async watch stub for CancelSubscription
    async_cancel = AsyncMock()
    mock_stub = MagicMock()
    mock_stub.CancelSubscription = async_cancel
    client._async_watch_stub.return_value = mock_stub

    return client


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_yields_typed_events():
    raw_events = [_make_raw_event(event_type=1, label="first"), _make_raw_event(event_type=2, label="second")]
    stream = _make_mock_async_stream(raw_events)
    client = _make_mock_client()

    results = []
    async with WatchStream(stream, client, WatchNS.Item, "watch_test") as events:
        async for event in events:
            results.append(event)

    assert len(results) == 2
    assert results[0].type == WatchEventType.ENTER
    assert results[0].node.label == "first"
    assert results[1].type == WatchEventType.UPDATE
    assert results[1].node.label == "second"


@pytest.mark.asyncio
async def test_captures_subscription_id():
    raw_events = [_make_raw_event(sub_id="sub-42")]
    stream = _make_mock_async_stream(raw_events)
    client = _make_mock_client()

    ws = WatchStream(stream, client, WatchNS.Item, "watch_test")
    async with ws as events:
        event = await events.__anext__()
        assert event.subscription_id == "sub-42"
        assert ws._subscription_id == "sub-42"


@pytest.mark.asyncio
async def test_cancel_on_context_exit():
    raw_events = [_make_raw_event()]
    stream = _make_mock_async_stream(raw_events)
    client = _make_mock_client()

    ws = WatchStream(stream, client, WatchNS.Item, "watch_test")
    async with ws as events:
        await events.__anext__()  # consume one event to capture subscription_id

    # gRPC stream should be cancelled
    stream.cancel.assert_called_once()

    # Server-side CancelSubscription should be called
    async_stub = client._async_watch_stub.return_value
    async_stub.CancelSubscription.assert_awaited_once()
    cancel_req = async_stub.CancelSubscription.call_args[0][0]
    assert cancel_req.subscription_id == "sub-1"


@pytest.mark.asyncio
async def test_cancel_on_break():
    events_list = [_make_raw_event(event_type=i + 1) for i in range(5)]
    stream = _make_mock_async_stream(events_list)
    client = _make_mock_client()

    ws = WatchStream(stream, client, WatchNS.Item, "watch_test")
    async with ws as events:
        async for event in events:
            break  # break immediately

    stream.cancel.assert_called_once()


@pytest.mark.asyncio
async def test_double_cancel_is_safe():
    stream = _make_mock_async_stream([_make_raw_event()])
    client = _make_mock_client()

    ws = WatchStream(stream, client, WatchNS.Item, "watch_test")
    async with ws as events:
        await events.__anext__()

    # Already cancelled by __aexit__, calling again should be safe
    await ws.cancel()
    assert stream.cancel.call_count == 1  # only called once


@pytest.mark.asyncio
async def test_no_model_yields_none_node():
    raw = _make_raw_event()
    raw.HasField = lambda f: False  # no node field
    stream = _make_mock_async_stream([raw])
    client = _make_mock_client()

    ws = WatchStream(stream, client, None, "watch_test")
    async with ws as events:
        event = await events.__anext__()
        assert event.node is None
        assert event.type == WatchEventType.UPDATE


@pytest.mark.asyncio
async def test_stopasynciteration_on_empty_stream():
    stream = _make_mock_async_stream([])
    client = _make_mock_client()

    ws = WatchStream(stream, client, WatchNS.Item, "watch_test")
    async with ws as events:
        with pytest.raises(StopAsyncIteration):
            await events.__anext__()


@pytest.mark.asyncio
async def test_all_event_types():
    type_map = {
        1: WatchEventType.ENTER,
        2: WatchEventType.UPDATE,
        3: WatchEventType.EXIT,
        4: WatchEventType.DELETE,
        5: WatchEventType.HEARTBEAT,
    }
    for proto_type, expected in type_map.items():
        raw = _make_raw_event(event_type=proto_type)
        stream = _make_mock_async_stream([raw])
        client = _make_mock_client()
        ws = WatchStream(stream, client, WatchNS.Item, "watch_test")
        event = await ws.__anext__()
        assert event.type == expected


def test_typed_watch_event_repr():
    event = TypedWatchEvent(
        type=WatchEventType.UPDATE,
        subscription_id="sub-1",
        versionstamp=b"\x01",
        node=None,
    )
    r = repr(event)
    assert "update" in r
    assert "None" in r
