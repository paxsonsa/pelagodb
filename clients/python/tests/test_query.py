"""Unit tests for the PelagoDB filter expressions and query builder (no server required)."""

from __future__ import annotations

from unittest.mock import MagicMock

import pytest

from pelagodb.query import CompoundExpr, FilterExpr, QueryBuilder
from pelagodb.schema import Entity, Namespace, OutEdge, Property
from pelagodb.types import IndexType


# ---------------------------------------------------------------------------
# Test entities
# ---------------------------------------------------------------------------

class TestNS(Namespace):
    name = "test"

    class Person(Entity):
        name: str = Property(required=True, index=IndexType.EQUALITY)
        age: int = Property(default=0, index=IndexType.RANGE)
        active: bool = Property(default=True)
        follows = OutEdge("Person")


# ---------------------------------------------------------------------------
# FilterExpr tests
# ---------------------------------------------------------------------------


class TestFilterExprGt:
    def test_to_expr(self):
        expr = TestNS.Person.age > 30
        assert isinstance(expr, FilterExpr)
        assert expr.to_expr() == "age > 30"


class TestFilterExprGe:
    def test_to_expr(self):
        assert (TestNS.Person.age >= 30).to_expr() == "age >= 30"


class TestFilterExprLt:
    def test_to_expr(self):
        assert (TestNS.Person.age < 30).to_expr() == "age < 30"


class TestFilterExprLe:
    def test_to_expr(self):
        assert (TestNS.Person.age <= 30).to_expr() == "age <= 30"


class TestFilterExprEq:
    def test_to_expr(self):
        assert (TestNS.Person.age == 30).to_expr() == "age == 30"


class TestFilterExprNe:
    def test_to_expr(self):
        assert (TestNS.Person.age != 30).to_expr() == "age != 30"


class TestFilterExprStringValue:
    def test_string_quoted(self):
        expr = TestNS.Person.name == "Alice"
        assert expr.to_expr() == 'name == "Alice"'


class TestFilterExprBoolValue:
    def test_true(self):
        expr = TestNS.Person.active == True  # noqa: E712
        assert expr.to_expr() == "active == true"

    def test_false(self):
        expr = TestNS.Person.active == False  # noqa: E712
        assert expr.to_expr() == "active == false"


class TestCompoundAnd:
    def test_to_expr(self):
        expr = (TestNS.Person.age > 1) & (TestNS.Person.age < 100)
        assert isinstance(expr, CompoundExpr)
        assert expr.to_expr() == "(age > 1 && age < 100)"


class TestCompoundOr:
    def test_to_expr(self):
        expr = (TestNS.Person.age > 1) | (TestNS.Person.age < 100)
        assert expr.to_expr() == "(age > 1 || age < 100)"


class TestCompoundChaining:
    def test_triple_and(self):
        expr = (TestNS.Person.age > 1) & (TestNS.Person.age < 100) & (TestNS.Person.active == True)  # noqa: E712
        assert "&&" in expr.to_expr()
        assert "active == true" in expr.to_expr()

    def test_mixed_and_or(self):
        expr = ((TestNS.Person.age > 1) & (TestNS.Person.age < 100)) | (TestNS.Person.active == False)  # noqa: E712
        assert "||" in expr.to_expr()


# ---------------------------------------------------------------------------
# QueryBuilder tests
# ---------------------------------------------------------------------------


class TestQueryBuilderSimplePQL:
    def test_basic_filter_and_limit(self):
        mock_view = MagicMock()
        q = QueryBuilder(mock_view, TestNS.Person)
        pql = q.filter(TestNS.Person.age > 30).limit(50).to_pql()
        assert "Person" in pql
        assert "@filter(age > 30)" in pql
        assert "@limit(first: 50)" in pql
        assert "uid" in pql

    def test_no_filter(self):
        mock_view = MagicMock()
        q = QueryBuilder(mock_view, TestNS.Person)
        pql = q.limit(10).to_pql()
        assert "Person" in pql
        assert "@filter" not in pql
        assert "@limit(first: 10)" in pql

    def test_multiple_filters(self):
        mock_view = MagicMock()
        q = QueryBuilder(mock_view, TestNS.Person)
        pql = q.filter(TestNS.Person.age > 20).filter(TestNS.Person.active == True).to_pql()  # noqa: E712
        assert "@filter(age > 20 && active == true)" in pql

    def test_string_filter(self):
        mock_view = MagicMock()
        q = QueryBuilder(mock_view, TestNS.Person)
        pql = q.filter("age >= 25").to_pql()
        assert "@filter(age >= 25)" in pql


class TestQueryBuilderWithOffset:
    def test_offset(self):
        mock_view = MagicMock()
        q = QueryBuilder(mock_view, TestNS.Person)
        pql = q.limit(50, offset=10).to_pql()
        assert "@limit(first: 50, offset: 10)" in pql


class TestQueryBuilderWithTraversal:
    def test_traversal_edge_def(self):
        mock_view = MagicMock()
        q = QueryBuilder(mock_view, TestNS.Person)
        pql = q.filter(TestNS.Person.name == "Alice").traverse(TestNS.Person.follows).to_pql()
        assert "-[follows]->" in pql

    def test_traversal_with_filter(self):
        mock_view = MagicMock()
        q = QueryBuilder(mock_view, TestNS.Person)
        pql = (
            q.filter(TestNS.Person.name == "Alice")
            .traverse(TestNS.Person.follows, filter=TestNS.Person.age > 25)
            .to_pql()
        )
        assert "-[follows]-> @filter(age > 25)" in pql

    def test_traversal_string_edge(self):
        mock_view = MagicMock()
        q = QueryBuilder(mock_view, TestNS.Person)
        pql = q.traverse("follows").to_pql()
        assert "-[follows]->" in pql


class TestQueryBuilderSelect:
    def test_select_limits_fields(self):
        mock_view = MagicMock()
        q = QueryBuilder(mock_view, TestNS.Person)
        pql = q.select("name", "age").to_pql()
        assert "uid, name, age" in pql
        # Should NOT contain 'active' or 'email' since we selected specific fields
        assert "active" not in pql


class TestPropertyHash:
    def test_property_hashable(self):
        """Property must be hashable (for use in sets/dicts) despite __eq__ override."""
        p = TestNS.Person.__pelago_properties__["age"]
        s = {p}
        assert p in s
