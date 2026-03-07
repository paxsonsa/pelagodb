"""Enums and constants for the PelagoDB typed schema layer."""

from __future__ import annotations

import datetime
from enum import Enum


class IndexType(str, Enum):
    NONE = "none"
    UNIQUE = "unique"
    EQUALITY = "equality"
    RANGE = "range"


class EdgeDirection(str, Enum):
    OUTGOING = "outgoing"
    BIDIRECTIONAL = "bidirectional"


class OwnershipMode(str, Enum):
    SOURCE_SITE = "source_site"
    INDEPENDENT = "independent"


class ExtrasPolicy(str, Enum):
    REJECT = "reject"
    ALLOW = "allow"
    WARN = "warn"


PYTHON_TYPE_MAP: dict[type, str] = {
    str: "string",
    int: "int",
    float: "float",
    bool: "bool",
    datetime.datetime: "timestamp",
    bytes: "bytes",
}
