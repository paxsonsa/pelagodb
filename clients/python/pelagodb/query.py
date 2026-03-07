"""Filter expressions and query builder for PelagoDB typed schema layer.

FilterExpr objects are produced by Property operator overloads (e.g. Person.age > 30)
and compile to PQL @filter() expressions. QueryBuilder chains method calls into a
complete PQL query string.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    from pelagodb.namespace_view import NamespaceView
    from pelagodb.schema import EdgeDef, Entity


class FilterExpr:
    """A single filter condition like ``age > 30``.

    Compiles to a PQL ``@filter()`` clause fragment.
    """

    def __init__(self, field: str, op: str, value: Any) -> None:
        self.field = field
        self.op = op
        self.value = value

    def to_expr(self) -> str:
        if isinstance(self.value, str):
            return f'{self.field} {self.op} "{self.value}"'
        if isinstance(self.value, bool):
            return f'{self.field} {self.op} {"true" if self.value else "false"}'
        return f"{self.field} {self.op} {self.value}"

    def __and__(self, other: FilterExpr | CompoundExpr) -> CompoundExpr:
        return CompoundExpr("&&", self, other)

    def __or__(self, other: FilterExpr | CompoundExpr) -> CompoundExpr:
        return CompoundExpr("||", self, other)

    def __repr__(self) -> str:
        return f"FilterExpr({self.to_expr()!r})"


class CompoundExpr:
    """Two filter expressions joined by ``&&`` or ``||``."""

    def __init__(self, op: str, left: FilterExpr | CompoundExpr, right: FilterExpr | CompoundExpr) -> None:
        self.op = op
        self.left = left
        self.right = right

    def to_expr(self) -> str:
        return f"({self.left.to_expr()} {self.op} {self.right.to_expr()})"

    def __and__(self, other: FilterExpr | CompoundExpr) -> CompoundExpr:
        return CompoundExpr("&&", self, other)

    def __or__(self, other: FilterExpr | CompoundExpr) -> CompoundExpr:
        return CompoundExpr("||", self, other)

    def __repr__(self) -> str:
        return f"CompoundExpr({self.to_expr()!r})"


class QueryBuilder:
    """Builds PQL queries from Python method chains.

    All queries route through ExecutePQL for full planner/index support.
    """

    def __init__(self, namespace_view: NamespaceView, model: type[Entity]) -> None:
        self._ns_view = namespace_view
        self._model = model
        self._filters: list[FilterExpr | CompoundExpr | str] = []
        self._limit: int | None = None
        self._offset: int = 0
        self._fields: list[str] = []
        self._traversals: list[dict[str, Any]] = []

    def filter(self, expr: FilterExpr | CompoundExpr | str) -> QueryBuilder:
        """Add a filter. Accepts FilterExpr (Person.age > 30) or raw string."""
        self._filters.append(expr)
        return self

    def limit(self, n: int, offset: int = 0) -> QueryBuilder:
        self._limit = n
        self._offset = offset
        return self

    def select(self, *fields: str) -> QueryBuilder:
        """Restrict returned fields."""
        self._fields = list(fields)
        return self

    def traverse(self, edge: EdgeDef | str, *, filter: FilterExpr | CompoundExpr | str | None = None, limit: int | None = None) -> QueryBuilder:
        """Add a 1-hop edge traversal step."""
        self._traversals.append({"edge": edge, "filter": filter, "limit": limit})
        return self

    def to_pql(self) -> str:
        """Compile the builder chain to a PQL string."""
        entity_name = self._model.entity_name()
        parts: list[str] = [entity_name]

        # @filter
        expr_parts: list[str] = []
        for f in self._filters:
            if hasattr(f, "to_expr"):
                expr_parts.append(f.to_expr())  # type: ignore[union-attr]
            else:
                expr_parts.append(str(f))
        if expr_parts:
            parts.append(f"@filter({' && '.join(expr_parts)})")

        # @limit
        if self._limit is not None:
            if self._offset:
                parts.append(f"@limit(first: {self._limit}, offset: {self._offset})")
            else:
                parts.append(f"@limit(first: {self._limit})")

        # Selection block
        fields = self._fields or list(self._model.__pelago_properties__)
        field_list = ["uid"] + fields

        traversal_strs: list[str] = []
        for t in self._traversals:
            edge = t["edge"]
            label = edge.name if hasattr(edge, "name") else str(edge)
            trav = f"-[{label}]->"
            if t["filter"]:
                tf = t["filter"]
                trav += f" @filter({tf.to_expr() if hasattr(tf, 'to_expr') else str(tf)})"
            if t["limit"]:
                trav += f" @limit(first: {t['limit']})"
            trav += " { uid }"
            traversal_strs.append(trav)

        selection = ", ".join(field_list)
        if traversal_strs:
            selection += "\n    " + "\n    ".join(traversal_strs)
        parts.append("{ " + selection + " }")

        return " ".join(parts)

    def run(self):
        """Execute the query and yield typed model instances."""
        pql = self.to_pql()
        yield from self._ns_view.pql(self._model, pql)
