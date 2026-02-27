# Tutorial: Interactive REPL

Learn PelagoDB's REPL — an interactive shell supporting SQL-like commands, PQL queries, and meta commands.

## Starting the REPL

```bash
pelago repl
```

Or with explicit connection:

```bash
pelago --server http://127.0.0.1:27615 --database default --namespace default repl
```

## REPL Features

- **Syntax highlighting** for meta commands, SQL keywords, params, and PQL directives
- **Tab completion** for meta commands, SQL/PQL phrases, schema types, fields/edges, and `$params`
- **History** with `Ctrl-R` for reverse search
- **Smart multiline** entry for open braces/quotes
- **Fuzzy-ranked completion** with runtime controls

## SQL-Like Commands

### Context Management

```sql
USE DATABASE default;
USE namespace default;
```

### Schema Operations

Create a type:

```sql
CREATE TYPE Person (
  email STRING REQUIRED INDEX UNIQUE,
  age INT INDEX RANGE
) WITH (allow_undeclared_edges = false, extras_policy = reject);
```

View schemas:

```sql
SHOW SCHEMAS;
DESCRIBE Person;
```

### Data Operations

Insert:

```sql
INSERT INTO Person VALUES {"email":"alice@example.com","age":31};
```

Select:

```sql
SELECT * FROM Person WHERE age >= 30 LIMIT 50;
```

Update:

```sql
UPDATE Person:1_0 SET {"age":32};
```

Delete:

```sql
DELETE FROM Person:1_0;
```

### Edge Operations

Create edge:

```sql
CREATE EDGE Person:1_0 WORKS_AT Company:1_5 VALUES {"since":2020};
```

Show edges:

```sql
SHOW EDGES Person:1_0 DIRECTION out LABEL WORKS_AT LIMIT 20;
```

Traverse:

```sql
TRAVERSE FROM Person:1_0 VIA WORKS_AT DEPTH 2 MAX_RESULTS 50;
```

Delete edge:

```sql
DELETE EDGE Person:1_0 WORKS_AT Company:1_5;
```

### Admin Operations

```sql
SHOW JOBS;
SHOW SITES;
SHOW REPLICATION STATUS;
SHOW NAMESPACE SETTINGS;
SET NAMESPACE OWNER site-west;
CLEAR NAMESPACE OWNER;
```

## PQL in the REPL

Enter PQL queries directly:

```pql
Person @filter(age >= 30) { uid name age }
```

With explain:

```
:explain Person @filter(age >= 30) { uid name age }
```

## Meta Commands

### Settings

```
:set completion fuzzy       # fuzzy matching (default)
:set completion prefix      # prefix-only matching
:set color on               # enable syntax highlighting
:set color off              # disable syntax highlighting
```

### Parameters

The REPL supports `:param` for client-side parameter substitution.

## Tips

- Use **tab completion** to discover available schema types and their fields
- Use **multiline entry** by ending a line with an open brace `{` or quote `"`
- Use **`Ctrl-R`** to search command history
- Use **`:explain`** before any PQL query to see the compiled plan

## Related

- [CLI Reference](../reference/cli.md) — full CLI documentation
- [Learn PQL](learn-pql.md) — PQL tutorial
- [Learn CEL Queries](learn-cel-queries.md) — CEL tutorial
