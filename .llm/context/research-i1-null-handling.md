# Research: Gap I1 — CEL Null Value Handling

**Status:** Design Specification
**Author:** Andrew + Claude
**Date:** 2026-02-10

---

## 1. The Problem

### Concrete Scenarios

The CEL evaluation pipeline (Section 5 of graph-db-spec-v0.3.md) assumes all fields referenced in a CEL expression exist as variables in the evaluation environment. This assumption fails when:

1. **Optional properties** — Schema declares `age: int` with `required: false`. A node created without `age` has no key in its property CBOR. Evaluating `age > 30` must not crash.

2. **Schema evolution** — A new field is added to Person schema (e.g., `verified: bool`). Existing nodes created before this field was added have no value. When an old node is queried with `verified == true`, the expression must evaluate gracefully.

3. **Deprecated properties** — A field is marked deprecated but still exists on some nodes. New code tries to query it; old nodes have it, new nodes don't. The query must handle both cases.

4. **Partial node import** — A bulk import tool populates some properties but omits others (intentionally). The query engine must not crash due to missing fields.

5. **Schema policy mismatch** — Schema says `required: true` but validation wasn't enforced on older data. Old nodes lack the field.

---

## 2. The Design: Missing = Null. Period.

There is **no distinction** between a missing field and an explicitly null field. Both represent the same concept: "this property has no value."

### Fundamental Rule

If a property is not present in a node's CBOR blob, it is null.
If a property is explicitly set to null, it is null.
Same thing. One concept. No three-valued logic (present/null/missing).

---

## 3. Null Handling in CEL Expressions

### Comparison Operators with Null

All standard comparison operators (`>`, `>=`, `<`, `<=`, `==`, `!=`) have well-defined behavior when either operand is null:

| Expression | Null operand | Result | Rationale |
|---|---|---|---|
| `age > 30` | age is null | `false` | A null value does not satisfy any comparison |
| `age >= 30` | age is null | `false` | Same: null cannot be greater than or equal to any value |
| `age < 30` | age is null | `false` | Null is not less than any value |
| `age <= 30` | age is null | `false` | Null is not less than or equal to any value |
| `age == null` | age is null | `true` | This is how you query for "nodes that don't have age set" |
| `age != null` | age is null | `false` | Negation: if age is null, then age != null is false |
| `age != 30` | age is null | `false` | Null is not equal to 30, so the negation is false |

**Key insight:** `age == null` and `age != null` are the **only** operators that produce `true` when a field is null. All other comparisons return `false`.

### String Operations with Null

String operations (startsWith, contains, endsWith, etc.) return `false` on null operands:

| Expression | Null operand | Result |
|---|---|---|
| `email.startsWith("alice")` | email is null | `false` |
| `bio.contains("engineer")` | bio is null | `false` |

### Boolean Operators with Null

Null is treated as a falsy value in boolean AND/OR expressions:

| Expression | Result | Rationale |
|---|---|---|
| `null && true` | `false` | Null is falsy; AND short-circuits |
| `null && false` | `false` | AND with false is always false |
| `null \|\| true` | `true` | OR with true is always true |
| `null \|\| false` | `false` | Null is falsy and false is falsy, result is false |
| `null && age > 30` | `false` | Null is falsy, AND short-circuits immediately |
| `active \|\| (age > 30)` | depends on `active` | If active is true, result is true. If active is null (falsy), evaluate `(age > 30)` |

---

## 4. Example Query Semantics

### Query: "Find people older than 30"

```cel
age > 30
```

- **Nodes with age set to a value >= 30:** included ✓
- **Nodes with age set to a value < 30:** excluded ✓
- **Nodes without age (age is null):** excluded ✓ (null > 30 = false)

**Result:** Only nodes with an explicit age value greater than 30.

### Query: "Find people with age set (any age)"

```cel
age != null
```

- **Nodes with age set to any value:** included ✓
- **Nodes without age (age is null):** excluded ✓ (null != null = false)

**Result:** Only nodes where the age field is present.

### Query: "Find people without age set"

```cel
age == null
```

- **Nodes with age set to any value:** excluded ✓
- **Nodes without age (age is null):** included ✓ (null == null = true)

**Result:** Only nodes where age was never set.

### Query: "Find active people or people older than 30"

```cel
active || age > 30
```

- **Node A:** `active` is true, `age` is null → true OR false = **true** ✓
- **Node B:** `active` is null, `age` is 35 → false OR true = **true** ✓
- **Node C:** `active` is null, `age` is 25 → false OR false = **false** ✗
- **Node D:** `active` is true, `age` is 25 → true OR false = **true** ✓

Null is falsy; the OR evaluates correctly.

### Query: "Find people named Alice who are older than 30"

```cel
name == "Alice" && age > 30
```

- **Node with name="Alice", age=35:** true AND true = **true** ✓
- **Node with name="Alice", age=null:** true AND false = **false** ✗
- **Node with name="Bob", age=35:** false AND true = **false** ✗

Both conditions must be true. Missing age causes the AND to fail, as expected.

---

## 5. Null Handling in Indexes

### Core Rule

Null values are **NOT indexed.** If a property is indexed and a node has no value for that property (null), no index entry is created for that node.

### Index Scans Exclude Null Fields

**Scenario:** Query `age >= 30` where age is indexed with a range index.

**Nodes:**
- A: `{name: "Alice", age: 35}` — in age index under key (age, 35, A)
- B: `{name: "Bob"}` — NOT in age index (age is null)
- C: `{name: "Charlie", age: 25}` — in age index under key (age, 25, C)

**Query execution:** Scan age index from 30 to MAX → returns [A]. Node B is excluded because it has no age index entry. Node C is excluded because its age value (25) is below the range.

**Correctness:** This is correct. A node without age set should not match a range predicate on age.

### Null Equality Queries Cannot Use Index

**Query:** `age == null`

This cannot be satisfied by an index scan on the age field because null values are not in the index. The query must use a **full scan with residual filter:**

1. Scan all nodes of the entity type (full scan, not via age index)
2. For each node, evaluate: is `age` present in the CBOR? If not, include it in results.

**Performance note:** This is the trade-off. Null-equality queries are slower (require full scan). To optimize this if it becomes a bottleneck, a separate null-tracking index could be added in the future, but it's not in the core design.

### Unique Index Constraints

**Scenario:** Email is declared with `index: "unique"`.

**Write-time behavior:**
- Node A created with email="alice@example.com" → index entry created
- Node B created with email="bob@example.com" → index entry created
- Node C created without email (email is null) → NO index entry

**Query-time behavior:**
- Lookup by email="alice@example.com" → finds A immediately via index ✓
- Lookup by email=null → full scan required (see above)

**Uniqueness constraint:**
- The system enforces that no two nodes have the same non-null email value
- Multiple nodes can have null email (no constraint on null values)

This matches the behavior of most databases (PostgreSQL, MySQL, SQL Server).

---

## 6. Null in Projections

When returning node data in a response, null/missing fields are simply **absent from the response**.

**Example:**

```json
// Node in storage
{
  "id": "p_42",
  "name": "Alice",
  "age": null  // explicitly null or missing in CBOR
}

// Returned to client
{
  "id": "p_42",
  "name": "Alice"
  // age field is not present
}
```

Client SDKs represent missing fields using their language's natural optional type:
- **Rust:** `Option<T>` (e.g., `Option<i64>` for age)
- **TypeScript:** `undefined` or `type | undefined`
- **Python:** `None`
- **Go:** pointer types (e.g., `*int64`)

---

## 7. Required Fields and Validation

Schema properties can be marked `required: true` to enforce validation at write time.

### Schema Definition

```json
{
  "entity": {
    "name": "Person",
    "properties": {
      "name": {
        "type": "string",
        "required": true
      },
      "age": {
        "type": "int",
        "required": false
      }
    }
  }
}
```

### Write-Time Validation

When creating or updating a node:

1. Check that all `required: true` properties are present in the request
2. Reject with `ERR_MISSING_REQUIRED_FIELD` if any required field is missing
3. Allow creation/update to proceed if all required fields are supplied

**Example:**

```
CreateNode(Person, {name: "Alice"})
  ✓ Accepted: name is supplied, age is optional and omitted

CreateNode(Person, {age: 35})
  ✗ Rejected: name is required but missing
```

### Implication

A field marked `required: true` can never be null on a node in storage. The API layer enforces this; storage never contains a required field without a value.

A field marked `required: false` (the default) may be null. This is the normal case for optional properties.

---

## 8. Default Values

Schema properties can declare a `default` value that is applied at node creation time if the field is omitted.

### Schema Definition

```json
{
  "entity": {
    "name": "Person",
    "properties": {
      "name": {
        "type": "string",
        "required": true
      },
      "created_at": {
        "type": "timestamp",
        "default": "now"
      },
      "age": {
        "type": "int",
        "required": false
        // no default
      }
    }
  }
}
```

### Creation Behavior

When a node is created:

1. Validate all required fields are present
2. Apply defaults to any field with a `default` value that was omitted
3. Leave non-required fields without a default as null (absent from CBOR)

**Example:**

```
CreateNode(Person, {name: "Alice"})
  → name: "Alice" (supplied)
  → created_at: <current timestamp> (default applied)
  → age: null (no default, not required)
```

After defaults are applied, the field is **present** in storage (not null).

### No Retroactive Default Application

Defaults are applied **only at node creation time**. If a schema evolves to add a new field with a default:

```
Old schema: Person { name, age }
Old node: {name: "Alice"} (created before created_at existed)

New schema: Person { name, age, created_at: timestamp, default: "now" }
New query on old node: Select created_at
  → returns: null (not the "now" default)
```

Defaults are not retroactively applied to old nodes. This aligns with the schema evolution model: old nodes without the new field have null values for that field, which is correct.

If schema evolution requires backfilling a new field with defaults, that is an application-level concern (using a background job to update all old nodes).

---

## 9. Schema Evolution

### Adding an Optional Property

**Scenario:** Add a new field `verified: bool` to the Person schema.

```
Old schema: Person { name, age }
New schema: Person { name, age, verified: bool, required: false }
```

**Behavior:**
- Old nodes: do not have `verified` field in CBOR → queries see it as null
- New nodes created after schema change: may omit `verified` → null, or set `verified: true/false`
- Queries like `verified == true` correctly exclude old nodes (null != true)
- Queries like `verified != null` correctly include only new nodes with verified set

**No retroactive indexing:** If the new field is indexed, old nodes are not retroactively added to the index. A background job (us-3) can be triggered to index existing nodes, but queries work correctly even without it (they return fewer results, but correct results).

### Removing a Property

This is a schema maintenance concern, out of scope for Phase 1. Handled in the C6 gap.

---

## 10. Implementation

### CEL Type Environment

When building the CEL type environment for an entity type, all properties (required or optional) are declared at their declared types:

```rust
fn build_cel_environment(schema: &EntitySchema) -> CelEnvironment {
    let mut env = CelEnvironment::new();
    for (field_name, field_schema) in &schema.properties {
        let cel_type = match field_schema.type_name.as_str() {
            "int" => CelType::Int,
            "string" => CelType::String,
            "bool" => CelType::Bool,
            "timestamp" => CelType::Timestamp,
            _ => return Err("Unknown type"),
        };

        // Add to environment. Optional fields are still added; null-safety
        // is handled at eval time, not type-check time.
        env.add_variable(field_name.clone(), cel_type)?;
    }
    Ok(env)
}
```

### CEL Evaluation Context

When evaluating a CEL expression against a node:

```rust
fn eval_cel_expression(
    compiled_expr: &CompiledExpr,
    node: &Node,
    schema: &EntitySchema,
) -> Result<bool> {
    let mut context = HashMap::new();
    for (field_name, _field_schema) in &schema.properties {
        if let Some(value) = node.properties.get(field_name) {
            context.insert(field_name.clone(), value.clone());
        }
        // If field is not in node.properties, it is simply absent from context.
        // CEL runtime handles absence as null with the semantics below.
    }

    compiled_expr.eval(&context)
}
```

### CEL Runtime Null Semantics

The CEL runtime must implement the null semantics defined in Section 3:

- Missing variables (not in evaluation context) are treated as null
- Null comparisons: `null == null` → true, `null != value` → false, `null <op> value` → false for all other operators
- Null in AND/OR: null is falsy
- Null in string operations: return false

If cel-rust does not natively support this, it must be wrapped:

```rust
// Wrapper around CEL evaluation
fn eval_with_null_semantics(
    expr: &CompiledExpr,
    context: &HashMap<String, Value>,
) -> Result<bool> {
    // Handle missing variables by setting them to a null sentinel
    let mut safe_context = context.clone();
    for field in expr.referenced_fields() {
        if !safe_context.contains_key(field) {
            safe_context.insert(field.clone(), Value::Null);
        }
    }
    expr.eval(&safe_context)
}
```

### Node Validation on Write

```rust
fn validate_node_properties(
    properties: &Map<String, Value>,
    schema: &EntitySchema,
) -> Result<()> {
    for (field_name, field_schema) in &schema.properties {
        if field_schema.required && !properties.contains_key(field_name) {
            return Err(ValidationError::MissingRequiredField(field_name));
        }
    }
    Ok(())
}
```

### Index Entry Creation

```rust
fn index_node(
    node: &Node,
    schema: &EntitySchema,
    txn: &mut FdbTransaction,
) -> Result<()> {
    for (field_name, field_schema) in &schema.properties {
        if let Some(index_type) = &field_schema.index {
            if let Some(value) = node.properties.get(field_name) {
                // Field is present: create index entry
                let idx_key = format!("(idx, {}, {}, {})", field_name, value, node.id);
                txn.set(&idx_key, b"");
            }
            // If field is null: no index entry (O.K., matches the design)
        }
    }
    Ok(())
}
```

---

## 11. Query Examples

### Example 1: Range Query with Null

**Query:** "Find all people aged 30 or older"

```cel
age >= 30
```

**Result:**
- Nodes with age >= 30: included ✓
- Nodes with age < 30: excluded ✓
- Nodes without age (null): excluded ✓

**Index behavior:**
- Compiles to `RangeScan(age, >=, 30)`
- Scans age index from 30 to MAX
- Nodes without age have no index entry, so are naturally excluded

---

### Example 2: Null Check Query

**Query:** "Find people without an age set"

```cel
age == null
```

**Result:**
- Nodes with age set to any value: excluded ✓
- Nodes without age: included ✓

**Index behavior:**
- Cannot use age index (null values not indexed)
- Full scan with residual filter: `does this node lack the age field?`

---

### Example 3: Presence Check Query

**Query:** "Find people with a bio"

```cel
bio != null
```

**Result:**
- Nodes with bio set to any non-null value: included ✓
- Nodes without bio: excluded ✓

**Index behavior:**
- If bio is indexed: scan index (bio values present)
- If bio is not indexed: full scan with residual filter

---

### Example 4: Complex Predicate with Null

**Query:** "Find active people or people with manager set"

```cel
active || manager != null
```

**Nodes:**
- A: `active=true, manager=null` → true OR false = **true** ✓
- B: `active=null, manager="bob"` → false OR true = **true** ✓
- C: `active=false, manager=null` → false OR false = **false** ✗
- D: `active=null, manager=null` → false OR false = **false** ✗

Null is falsy; the expression evaluates correctly.

---

## 12. Edge Properties and Null

Edge properties follow the same null semantics as node properties.

### Edge Filter CEL

When filtering edges by properties, the same rules apply:

```cel
// Edge type: MANAGES { role: string, started: timestamp }
// Query: edges with role set
role != null

// Query: edges started before 2024
started < timestamp("2024-01-01")  // null started values excluded
```

---

## 13. Migration Path for Existing Data

If you have existing data that uses different null semantics (e.g., explicit NULL markers in CBOR distinct from missing fields), migration steps:

1. Define a schema version for the data (old behavior vs. new)
2. During schema evolution, add a background job that:
   - Reads all nodes of the entity type
   - For any node with explicit NULL markers, delete the CBOR key (making it truly absent)
   - Writes updated node back
3. After migration completes, all data follows the new semantics

For new data created after this design is implemented, the unified semantics are automatic.

---

## 14. Summary

| Aspect | Design |
|---|---|
| **Missing vs. Null** | No distinction. Both are null. One concept. |
| **Null in `>`, `>=`, `<`, `<=`, `!=`** | Always `false` |
| **Null in `==`** | `null == null` → `true`; `null == value` → `false` |
| **Null in string operations** | Always `false` (startsWith, contains, etc.) |
| **Null in AND/OR** | Null is falsy. `null && x` → `false`. `null \|\| x` → `x`. |
| **Null in indexes** | Not indexed. Range scans exclude null fields. Null equality requires full scan. |
| **Null in responses** | Absent field (not returned to client). Represented as `Option<T>`, `undefined`, `None`, etc. in SDKs. |
| **Required fields** | Validated at write time. Cannot be null in storage. |
| **Default values** | Applied at creation time if field omitted. Not retroactively applied to old data. |
| **Schema evolution** | New optional fields are null on old nodes. Indexed new fields may not have old nodes in index until background backfill completes. |

---

## 15. Rationale

This design is:

- **Simple:** One null concept, not three-valued logic. Predictable semantics.
- **Compatible with NoSQL:** Matches document database idioms (MongoDB, Firebase).
- **Index-efficient:** No write amplification from defaults. Missing fields don't bloat indexes.
- **Schema-evolution-friendly:** Adding optional fields doesn't retroactively index old data.
- **Standards-aligned:** CEL supports this natively. Client SDKs already have Option/Optional/nullable types.
- **Correct:** Null values don't satisfy comparisons; null equality is explicit (`== null`, `!= null`). Matches developer intuition.

---

## 16. No Open Questions

This design is complete and final. Implementation begins immediately.

