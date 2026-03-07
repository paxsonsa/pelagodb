"""Namespace, Entity, Property, and EdgeDef definitions for PelagoDB typed schema layer.

This module provides a Pydantic-style (zero-dep) schema definition API:

    class MyNamespace(Namespace):
        name = "my_ns"

        class Person(Entity):
            name: str = Property(required=True, index=IndexType.EQUALITY)
            age: int = Property(default=0, index=IndexType.RANGE)
            follows = OutEdge("Person")
"""

from __future__ import annotations

import re
import typing
from typing import Any

from pelagodb.query import FilterExpr
from pelagodb.types import (
    EdgeDirection,
    IndexType,
    OwnershipMode,
    PYTHON_TYPE_MAP,
)


# Sentinel for "no default provided"
_UNSET = object()


def _resolve_type(annotation: Any) -> type:
    """Unwrap Optional[T] / T | None to the inner type."""
    origin = typing.get_origin(annotation)
    if origin is typing.Union:
        args = [a for a in typing.get_args(annotation) if a is not type(None)]
        if len(args) == 1:
            return args[0]
    return annotation


def _extract_template_vars(name: str) -> set[str]:
    """Extract ``{var_name}`` template variables from a namespace name."""
    return set(re.findall(r"\{(\w+)\}", name))


# ---------------------------------------------------------------------------
# Property descriptor
# ---------------------------------------------------------------------------


class Property:
    """Property descriptor with operator overloading for filter expressions.

    PelagoDB type is inferred from the Python type annotation at class
    definition time (via EntityMeta).
    """

    # Preserve identity hashing despite __eq__ override.
    __hash__ = object.__hash__

    def __init__(
        self,
        *,
        required: bool = False,
        index: IndexType = IndexType.NONE,
        default: Any = _UNSET,
    ) -> None:
        self.required = required
        self.index = index
        self.default = default
        self.name: str = ""
        self.pelago_type: str = ""
        self.python_type: type = object

    def __set_name__(self, owner: type, name: str) -> None:
        self.name = name

    def __get__(self, obj: Any, objtype: type | None = None) -> Any:
        if obj is None:
            return self  # class-level access → descriptor (enables Person.age > 30)
        return obj._data.get(self.name, self.default if self.default is not _UNSET else None)

    def __set__(self, obj: Any, value: Any) -> None:
        if value is not None and not isinstance(value, self.python_type):
            raise TypeError(
                f"{self.name}: expected {self.python_type.__name__}, got {type(value).__name__}"
            )
        obj._data[self.name] = value

    # Operator overloads → FilterExpr
    def __gt__(self, other: Any) -> FilterExpr:
        return FilterExpr(self.name, ">", other)

    def __ge__(self, other: Any) -> FilterExpr:
        return FilterExpr(self.name, ">=", other)

    def __lt__(self, other: Any) -> FilterExpr:
        return FilterExpr(self.name, "<", other)

    def __le__(self, other: Any) -> FilterExpr:
        return FilterExpr(self.name, "<=", other)

    def __eq__(self, other: Any) -> FilterExpr:  # type: ignore[override]
        if other is _UNSET or (isinstance(other, type) and other is Property):
            return NotImplemented  # type: ignore[return-value]
        return FilterExpr(self.name, "==", other)

    def __ne__(self, other: Any) -> FilterExpr:  # type: ignore[override]
        return FilterExpr(self.name, "!=", other)

    def to_schema_dict(self) -> dict[str, Any]:
        d: dict[str, Any] = {
            "type": self.pelago_type,
            "required": self.required,
            "index": self.index.value,
        }
        if self.default is not _UNSET:
            d["default"] = self.default
        return d

    def __repr__(self) -> str:
        return f"Property(name={self.name!r}, type={self.pelago_type!r})"


# ---------------------------------------------------------------------------
# Edge definitions
# ---------------------------------------------------------------------------


class EdgeDef:
    """Declares a graph edge on an Entity.

    ``target`` can be:
    - ``"*"`` — polymorphic (any entity type)
    - A string like ``"Person"`` — same-namespace entity by name
    - An Entity subclass like ``GlobalNamespace.Vendor`` — class reference (may be cross-namespace)
    """

    def __init__(
        self,
        target: Any = "*",
        *,
        direction: EdgeDirection = EdgeDirection.OUTGOING,
        sort_key: str = "",
        ownership: OwnershipMode = OwnershipMode.SOURCE_SITE,
    ) -> None:
        self.target = target
        self.direction = direction
        self.sort_key = sort_key
        self.ownership = ownership
        self.name: str = ""

    def __set_name__(self, owner: type, name: str) -> None:
        self.name = name

    def resolve_target(self) -> tuple[str, str | None]:
        """Return ``(entity_type, namespace_name_or_None)``.

        ``None`` namespace means same-namespace.
        """
        if self.target == "*":
            return ("*", None)
        if isinstance(self.target, str):
            return (self.target, None)
        # Class reference — Entity subclass
        if hasattr(self.target, "__pelago_namespace__"):
            ns = self.target.__pelago_namespace__
            ns_name = ns.__pelago_ns_resolved_name__ if ns else None
            return (self.target.entity_name(), ns_name)
        return (self.target.entity_name(), None)

    def to_schema_dict(self) -> dict[str, Any]:
        entity_type, _ = self.resolve_target()
        return {
            "target": entity_type,
            "direction": self.direction.value,
            "sort_key": self.sort_key,
            "ownership": self.ownership.value,
        }

    def __repr__(self) -> str:
        return f"EdgeDef(name={self.name!r}, target={self.target!r})"


def OutEdge(target: Any = "*", **kw: Any) -> EdgeDef:
    """Shorthand for an outgoing edge definition."""
    return EdgeDef(target, direction=EdgeDirection.OUTGOING, **kw)


def BiEdge(target: Any = "*", **kw: Any) -> EdgeDef:
    """Shorthand for a bidirectional edge definition."""
    return EdgeDef(target, direction=EdgeDirection.BIDIRECTIONAL, **kw)


# ---------------------------------------------------------------------------
# Entity metaclass and base class
# ---------------------------------------------------------------------------


class EntityMeta(type):
    """Metaclass that introspects annotations to build property/edge registries."""

    def __new__(mcs, name: str, bases: tuple[type, ...], namespace: dict[str, Any]) -> EntityMeta:
        cls = super().__new__(mcs, name, bases, namespace)
        if name == "Entity":
            return cls

        # Gather annotations, resolving string annotations from PEP 563
        annotations: dict[str, Any] = {}
        try:
            annotations = typing.get_type_hints(cls)
        except Exception:
            for base in reversed(cls.__mro__):
                if hasattr(base, "__annotations__"):
                    annotations.update(base.__annotations__)

        props: dict[str, Property] = {}
        edges: dict[str, EdgeDef] = {}

        for attr_name, attr in list(namespace.items()):
            if isinstance(attr, Property):
                if attr_name not in annotations:
                    raise TypeError(f"{name}.{attr_name}: Property must have a type annotation")
                py_type = _resolve_type(annotations[attr_name])
                if py_type not in PYTHON_TYPE_MAP:
                    raise TypeError(
                        f"{name}.{attr_name}: unsupported type {py_type}. "
                        f"Supported: {list(PYTHON_TYPE_MAP.keys())}"
                    )
                attr.pelago_type = PYTHON_TYPE_MAP[py_type]
                attr.python_type = py_type
                props[attr_name] = attr
            elif isinstance(attr, EdgeDef):
                edges[attr_name] = attr

        # Inherit from parent Entity classes
        for base in bases:
            if hasattr(base, "__pelago_properties__"):
                for k, v in base.__pelago_properties__.items():
                    props.setdefault(k, v)
            if hasattr(base, "__pelago_edges__"):
                for k, v in base.__pelago_edges__.items():
                    edges.setdefault(k, v)

        cls.__pelago_properties__ = props  # type: ignore[attr-defined]
        cls.__pelago_edges__ = edges  # type: ignore[attr-defined]

        # Read optional Meta inner class
        meta_cls = namespace.get("Meta")
        cls.__pelago_meta__: dict[str, Any] = {}  # type: ignore[attr-defined]
        if meta_cls:
            for attr in ("extras_policy", "allow_undeclared_edges"):
                if hasattr(meta_cls, attr):
                    cls.__pelago_meta__[attr] = getattr(meta_cls, attr)
            if hasattr(meta_cls, "entity_name"):
                cls.__pelago_entity_name__ = meta_cls.entity_name  # type: ignore[attr-defined]

        if not hasattr(cls, "__pelago_entity_name__"):
            cls.__pelago_entity_name__ = name  # type: ignore[attr-defined]

        # Namespace back-reference (set by NamespaceMeta when Entity is nested)
        if not hasattr(cls, "__pelago_namespace__"):
            cls.__pelago_namespace__ = None  # type: ignore[attr-defined]

        return cls


class Entity(metaclass=EntityMeta):
    """Base class for PelagoDB entity definitions.

    Instances hold property data in ``_data`` dict, with typed get/set via
    Property descriptors.
    """

    __pelago_properties__: dict[str, Property]
    __pelago_edges__: dict[str, EdgeDef]
    __pelago_meta__: dict[str, Any]
    __pelago_entity_name__: str
    __pelago_namespace__: type | None

    def __init__(self, *, _id: str = "", _created_at: int = 0, _updated_at: int = 0, _namespace: str | None = None, **kwargs: Any) -> None:
        self._data: dict[str, Any] = {}
        self._id = _id
        self._created_at = _created_at
        self._updated_at = _updated_at
        self._namespace = _namespace

        for key, value in kwargs.items():
            if key not in self.__pelago_properties__:
                raise AttributeError(f"{type(self).__name__} has no property '{key}'")
            setattr(self, key, value)

        # Apply defaults for unset properties
        for prop_name, prop in self.__pelago_properties__.items():
            if prop_name not in self._data and prop.default is not _UNSET:
                self._data[prop_name] = prop.default

    @classmethod
    def entity_name(cls) -> str:
        return cls.__pelago_entity_name__

    @classmethod
    def to_schema_dict(cls) -> dict[str, Any]:
        schema: dict[str, Any] = {"name": cls.entity_name()}
        schema["properties"] = {
            name: prop.to_schema_dict() for name, prop in cls.__pelago_properties__.items()
        }
        if cls.__pelago_edges__:
            schema["edges"] = {
                name: edge.to_schema_dict() for name, edge in cls.__pelago_edges__.items()
            }
        if cls.__pelago_meta__:
            schema["meta"] = dict(cls.__pelago_meta__)
        return schema

    def to_properties_dict(self) -> dict[str, Any]:
        return dict(self._data)

    @classmethod
    def from_node(cls, node: Any, *, namespace: str | None = None) -> Entity:
        """Construct a typed Entity instance from a protobuf Node."""
        from pelagodb.client import value_to_py

        props: dict[str, Any] = {}
        for k, v in node.properties.items():
            if k in cls.__pelago_properties__:
                props[k] = value_to_py(v)
        return cls(
            _id=node.id,
            _created_at=node.created_at,
            _updated_at=node.updated_at,
            _namespace=namespace,
            **props,
        )

    @property
    def id(self) -> str:
        return self._id

    def __repr__(self) -> str:
        props = ", ".join(f"{k}={v!r}" for k, v in self._data.items())
        ns = f", ns={self._namespace!r}" if self._namespace else ""
        return f"{type(self).__name__}(id={self._id!r}{ns}, {props})"


# ---------------------------------------------------------------------------
# Namespace metaclass and base class
# ---------------------------------------------------------------------------


class NamespaceMeta(type):
    """Metaclass that collects inner Entity classes defined within a Namespace."""

    def __new__(mcs, cls_name: str, bases: tuple[type, ...], namespace_dict: dict[str, Any]) -> NamespaceMeta:
        cls = super().__new__(mcs, cls_name, bases, namespace_dict)
        if cls_name == "Namespace":
            return cls

        # Collect inner Entity classes
        entities: dict[str, type[Entity]] = {}
        for attr_name, attr in namespace_dict.items():
            if isinstance(attr, type) and issubclass(attr, Entity) and attr is not Entity:
                entities[attr_name] = attr
                attr.__pelago_namespace__ = cls  # back-reference

        cls.__pelago_entities__ = entities  # type: ignore[attr-defined]

        # Resolve namespace name
        ns_name = namespace_dict.get("name", cls_name)
        cls.__pelago_ns_name__ = ns_name  # type: ignore[attr-defined]
        cls.__pelago_ns_template_vars__ = _extract_template_vars(ns_name)  # type: ignore[attr-defined]
        cls.__pelago_ns_resolved_name__ = (  # type: ignore[attr-defined]
            ns_name if not cls.__pelago_ns_template_vars__ else None  # type: ignore[attr-defined]
        )

        return cls


class Namespace(metaclass=NamespaceMeta):
    """Base class for PelagoDB namespace definitions.

    Inner Entity classes are automatically collected. Templated namespaces
    use ``bind()`` for instantiation::

        class TenantNS(Namespace):
            name = "tenant_{tenant_id}"
            class Person(Entity): ...

        acme = TenantNS.bind(tenant_id="acme")  # name="tenant_acme"
    """

    name: str = ""

    __pelago_entities__: dict[str, type[Entity]]
    __pelago_ns_name__: str
    __pelago_ns_template_vars__: set[str]
    __pelago_ns_resolved_name__: str | None

    @classmethod
    def bind(cls, **kwargs: str) -> type[Namespace]:
        """Instantiate a templated namespace with concrete variable values.

        Returns a new Namespace subclass with a resolved name.
        """
        missing = cls.__pelago_ns_template_vars__ - set(kwargs.keys())
        if missing:
            raise ValueError(f"Missing template variables: {missing}")

        resolved_name = cls.__pelago_ns_name__.format(**kwargs)

        # Create a bound namespace class
        bound: type[Namespace] = type(  # type: ignore[assignment]
            f"{cls.__name__}_{resolved_name}",
            (cls,),
            {
                "name": resolved_name,
                "__pelago_ns_resolved_name__": resolved_name,
                "__pelago_ns_template_vars__": set(),
            },
        )
        bound.__pelago_entities__ = cls.__pelago_entities__
        return bound

    @classmethod
    def entities(cls) -> dict[str, type[Entity]]:
        return cls.__pelago_entities__
