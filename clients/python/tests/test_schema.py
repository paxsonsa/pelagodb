"""Unit tests for the PelagoDB typed schema layer (no server required)."""

from __future__ import annotations

import datetime
from typing import Optional
from unittest.mock import MagicMock

import pytest

from pelagodb.schema import (
    BiEdge,
    EdgeDef,
    Entity,
    Namespace,
    OutEdge,
    Property,
    _UNSET,
)
from pelagodb.types import EdgeDirection, IndexType, OwnershipMode


# ---------------------------------------------------------------------------
# Fixtures: define entities used across tests
# ---------------------------------------------------------------------------


class GlobalNS(Namespace):
    name = "global"

    class Vendor(Entity):
        name: str = Property(required=True, index=IndexType.EQUALITY)
        industry: str = Property()


class TenantNS(Namespace):
    name = "tenant_{tenant_id}"

    class Person(Entity):
        name: str = Property(required=True, index=IndexType.EQUALITY)
        email: str = Property(index=IndexType.UNIQUE)
        age: int = Property(default=0, index=IndexType.RANGE)
        active: bool = Property(default=True)
        follows = OutEdge("Person")
        supplied_by = OutEdge(GlobalNS.Vendor)


# ---------------------------------------------------------------------------
# Entity / Property tests
# ---------------------------------------------------------------------------


class TestTypeInference:
    def test_str_to_string(self):
        assert TenantNS.Person.__pelago_properties__["name"].pelago_type == "string"

    def test_int_to_int(self):
        assert TenantNS.Person.__pelago_properties__["age"].pelago_type == "int"

    def test_bool_to_bool(self):
        assert TenantNS.Person.__pelago_properties__["active"].pelago_type == "bool"

    def test_float_to_float(self):
        class NS(Namespace):
            name = "test_float"

            class Measurement(Entity):
                value: float = Property()

        assert NS.Measurement.__pelago_properties__["value"].pelago_type == "float"

    def test_datetime_to_timestamp(self):
        class NS(Namespace):
            name = "test_dt"

            class Event(Entity):
                ts: datetime.datetime = Property()

        assert NS.Event.__pelago_properties__["ts"].pelago_type == "timestamp"

    def test_bytes_to_bytes(self):
        class NS(Namespace):
            name = "test_bytes"

            class Blob(Entity):
                data: bytes = Property()

        assert NS.Blob.__pelago_properties__["data"].pelago_type == "bytes"


class TestOptionalType:
    def test_optional_str_resolves(self):
        class NS(Namespace):
            name = "test_opt"

            class Item(Entity):
                label: Optional[str] = Property()

        assert NS.Item.__pelago_properties__["label"].pelago_type == "string"


class TestUnsupportedType:
    def test_list_raises(self):
        with pytest.raises(TypeError, match="unsupported type"):
            class Bad(Entity):
                tags: list = Property()

    def test_dict_raises(self):
        with pytest.raises(TypeError, match="unsupported type"):
            class Bad(Entity):
                meta: dict = Property()


class TestPropertyWithoutAnnotation:
    def test_raises(self):
        with pytest.raises(TypeError, match="must have a type annotation"):
            class Bad(Entity):
                oops = Property()


class TestGetSetValidation:
    def test_valid_set(self):
        p = TenantNS.Person(name="Alice", age=31)
        assert p.name == "Alice"
        assert p.age == 31

    def test_wrong_type_raises(self):
        with pytest.raises(TypeError, match="expected str"):
            TenantNS.Person(name=123)

    def test_wrong_type_on_set_raises(self):
        p = TenantNS.Person(name="Alice")
        with pytest.raises(TypeError, match="expected int"):
            p.age = "not_an_int"


class TestDefaults:
    def test_defaults_applied(self):
        p = TenantNS.Person(name="Bob")
        assert p.age == 0
        assert p.active is True

    def test_explicit_overrides_default(self):
        p = TenantNS.Person(name="Bob", age=25, active=False)
        assert p.age == 25
        assert p.active is False


class TestUnknownProperty:
    def test_raises(self):
        with pytest.raises(AttributeError, match="has no property 'foo'"):
            TenantNS.Person(name="Alice", foo="bar")


class TestToSchemaDict:
    def test_matches_expected(self):
        schema = TenantNS.Person.to_schema_dict()
        assert schema["name"] == "Person"
        assert "name" in schema["properties"]
        assert schema["properties"]["name"]["type"] == "string"
        assert schema["properties"]["name"]["required"] is True
        assert schema["properties"]["name"]["index"] == "equality"
        assert schema["properties"]["age"]["type"] == "int"
        assert schema["properties"]["age"]["index"] == "range"
        assert schema["properties"]["age"]["default"] == 0

    def test_vendor_schema(self):
        schema = GlobalNS.Vendor.to_schema_dict()
        assert schema["name"] == "Vendor"
        assert schema["properties"]["name"]["required"] is True


class TestEdgesInSchemaDict:
    def test_same_ns_edge(self):
        schema = TenantNS.Person.to_schema_dict()
        assert "follows" in schema["edges"]
        assert schema["edges"]["follows"]["target"] == "Person"
        assert schema["edges"]["follows"]["direction"] == "outgoing"

    def test_cross_ns_edge(self):
        schema = TenantNS.Person.to_schema_dict()
        assert "supplied_by" in schema["edges"]
        assert schema["edges"]["supplied_by"]["target"] == "Vendor"


class TestMetaInSchemaDict:
    def test_meta_propagates(self):
        class NS(Namespace):
            name = "test_meta"

            class Flexible(Entity):
                label: str = Property()

                class Meta:
                    extras_policy = "allow"
                    allow_undeclared_edges = True

        schema = NS.Flexible.to_schema_dict()
        assert schema["meta"]["extras_policy"] == "allow"
        assert schema["meta"]["allow_undeclared_edges"] is True


class TestEntityName:
    def test_default_is_class_name(self):
        assert TenantNS.Person.entity_name() == "Person"

    def test_override_via_meta(self):
        class NS(Namespace):
            name = "test_name"

            class MyModel(Entity):
                val: str = Property()

                class Meta:
                    entity_name = "custom_entity"

        assert NS.MyModel.entity_name() == "custom_entity"


class TestFromNodeRoundtrip:
    def test_roundtrip(self):
        mock_node = MagicMock()
        mock_node.id = "abc123"
        mock_node.created_at = 1000
        mock_node.updated_at = 2000

        # Mock properties as a dict-like
        mock_name_val = MagicMock()
        mock_name_val.WhichOneof.return_value = "string_value"
        mock_name_val.string_value = "Alice"

        mock_age_val = MagicMock()
        mock_age_val.WhichOneof.return_value = "int_value"
        mock_age_val.int_value = 31

        mock_node.properties = {"name": mock_name_val, "age": mock_age_val}

        person = TenantNS.Person.from_node(mock_node, namespace="tenant_acme")
        assert person.id == "abc123"
        assert person.name == "Alice"
        assert person.age == 31
        assert person._namespace == "tenant_acme"


class TestInheritance:
    def test_child_inherits_parent_properties(self):
        class Base(Entity):
            label: str = Property(required=True)

        class Child(Base):
            count: int = Property(default=0)

        assert "label" in Child.__pelago_properties__
        assert "count" in Child.__pelago_properties__
        assert Child.__pelago_properties__["label"].required is True


class TestEdgeNotInPropertiesDict:
    def test_edges_excluded(self):
        p = TenantNS.Person(name="Alice", age=31)
        props = p.to_properties_dict()
        assert "follows" not in props
        assert "supplied_by" not in props
        assert "name" in props


# ---------------------------------------------------------------------------
# Namespace tests
# ---------------------------------------------------------------------------


class TestNamespaceCollectsEntities:
    def test_entities_collected(self):
        assert "Vendor" in GlobalNS.__pelago_entities__
        assert "Person" in TenantNS.__pelago_entities__
        assert "Asset" not in TenantNS.__pelago_entities__  # not defined in our fixture


class TestNamespaceEntityBackReference:
    def test_back_reference(self):
        assert GlobalNS.Vendor.__pelago_namespace__ is GlobalNS
        assert TenantNS.Person.__pelago_namespace__ is TenantNS


class TestNamespaceBind:
    def test_resolves_template(self):
        acme = TenantNS.bind(tenant_id="acme")
        assert acme.name == "tenant_acme"
        assert acme.__pelago_ns_resolved_name__ == "tenant_acme"
        assert acme.__pelago_ns_template_vars__ == set()

    def test_entities_accessible_on_bound(self):
        acme = TenantNS.bind(tenant_id="acme")
        assert "Person" in acme.__pelago_entities__

    def test_multiple_binds_independent(self):
        acme = TenantNS.bind(tenant_id="acme")
        beta = TenantNS.bind(tenant_id="beta")
        assert acme.name == "tenant_acme"
        assert beta.name == "tenant_beta"


class TestNamespaceBindMissingVar:
    def test_raises(self):
        with pytest.raises(ValueError, match="Missing template variables"):
            TenantNS.bind()


class TestNamespaceUnboundRegistrationFails:
    def test_unbound_has_no_resolved_name(self):
        assert TenantNS.__pelago_ns_resolved_name__ is None

    def test_non_template_resolves(self):
        assert GlobalNS.__pelago_ns_resolved_name__ == "global"


class TestCrossNamespaceEdgeRef:
    def test_resolve_target(self):
        edge = TenantNS.Person.__pelago_edges__["supplied_by"]
        entity_type, ns_name = edge.resolve_target()
        assert entity_type == "Vendor"
        assert ns_name == "global"

    def test_same_ns_resolve(self):
        edge = TenantNS.Person.__pelago_edges__["follows"]
        entity_type, ns_name = edge.resolve_target()
        assert entity_type == "Person"
        assert ns_name is None  # same namespace


class TestBiEdge:
    def test_direction(self):
        class NS(Namespace):
            name = "test_bi"

            class Node(Entity):
                label: str = Property()
                sibling = BiEdge("Node")

        edge = NS.Node.__pelago_edges__["sibling"]
        assert edge.direction == EdgeDirection.BIDIRECTIONAL
        assert edge.to_schema_dict()["direction"] == "bidirectional"


class TestEntityRepr:
    def test_repr(self):
        p = TenantNS.Person(name="Alice", age=31, _id="abc", _namespace="tenant_acme")
        r = repr(p)
        assert "Person" in r
        assert "Alice" in r
        assert "abc" in r
