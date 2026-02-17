use pelago_query::pql::*;

fn person_schema() -> SchemaInfo {
    SchemaInfo {
        entity_type: "Person".to_string(),
        fields: vec!["name".to_string(), "age".to_string(), "email".to_string()],
        edges: vec!["KNOWS".to_string(), "WORKS_AT".to_string()],
        allow_undeclared_edges: false,
    }
}

fn company_schema() -> SchemaInfo {
    SchemaInfo {
        entity_type: "Company".to_string(),
        fields: vec!["name".to_string(), "industry".to_string()],
        edges: vec!["EMPLOYS".to_string()],
        allow_undeclared_edges: false,
    }
}

fn make_schemas() -> InMemorySchemaProvider {
    let mut schemas = InMemorySchemaProvider::new();
    schemas.add_schema(person_schema());
    schemas.add_schema(company_schema());
    schemas
}

fn resolve_and_compile(input: &str) -> Vec<CompiledBlock> {
    let ast = parse_pql(input).unwrap();
    let schemas = make_schemas();
    let resolver = PqlResolver::new();
    let resolved = resolver.resolve(&ast, &schemas).unwrap();
    let compiler = PqlCompiler::new();
    compiler.compile(&resolved).unwrap()
}

// 1. Simple type query with filter -> FindNodes
#[test]
fn test_compile_simple_find() {
    let ast = parse_pql("Person @filter(age >= 30) { name, age }").unwrap();
    let schemas = make_schemas();
    let resolver = PqlResolver::new();
    let resolved = resolver.resolve(&ast, &schemas).unwrap();
    let compiler = PqlCompiler::new();
    let blocks = compiler.compile(&resolved).unwrap();

    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
        CompiledBlock::FindNodes {
            entity_type,
            cel_expression,
            fields,
            ..
        } => {
            assert_eq!(entity_type, "Person");
            assert_eq!(cel_expression.as_deref(), Some("age >= 30"));
            assert!(fields.contains(&"name".to_string()));
            assert!(fields.contains(&"age".to_string()));
        }
        other => panic!("Expected FindNodes, got {:?}", other),
    }
}

// 2. uid() lookup without edges -> PointLookup
#[test]
fn test_compile_uid_lookup_point() {
    let input = r#"query { result(func: uid(Person:abc123)) { name, age } }"#;
    let blocks = resolve_and_compile(input);

    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
        CompiledBlock::PointLookup {
            entity_type,
            node_id,
            fields,
            ..
        } => {
            assert_eq!(entity_type, "Person");
            assert_eq!(node_id, "abc123");
            assert_eq!(fields.len(), 2);
            assert!(fields.contains(&"name".to_string()));
        }
        other => panic!("Expected PointLookup, got {:?}", other),
    }
}

// 3. uid() with edge traversal -> Traverse
#[test]
fn test_compile_traversal() {
    let input = r#"query {
        result(func: uid(Person:abc123)) {
            name
            -[KNOWS]-> { name, age }
        }
    }"#;
    let blocks = resolve_and_compile(input);

    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
        CompiledBlock::Traverse {
            start_entity_type,
            start_node_id,
            steps,
            ..
        } => {
            assert_eq!(start_entity_type, "Person");
            assert_eq!(start_node_id, "abc123");
            assert_eq!(steps.len(), 1);
            assert_eq!(steps[0].edge_type, "KNOWS");
            assert_eq!(steps[0].direction, PqlEdgeDirection::Outgoing);
            assert!(steps[0].fields.contains(&"name".to_string()));
            assert!(steps[0].fields.contains(&"age".to_string()));
        }
        other => panic!("Expected Traverse, got {:?}", other),
    }
}

// 4. Edge and filter directives -> separate edge_filter and node_filter
#[test]
fn test_compile_edge_and_node_filters() {
    let input = r#"query {
        result(func: uid(Person:abc123)) {
            -[KNOWS]-> @edge(weight > 0.5) @filter(age > 21) { name }
        }
    }"#;
    let blocks = resolve_and_compile(input);

    match &blocks[0] {
        CompiledBlock::Traverse { steps, .. } => {
            assert_eq!(steps.len(), 1);
            assert_eq!(steps[0].edge_filter.as_deref(), Some("weight > 0.5"));
            assert_eq!(steps[0].node_filter.as_deref(), Some("age > 21"));
        }
        other => panic!("Expected Traverse, got {:?}", other),
    }
}

// 5. @limit directive -> limit in FindNodes
#[test]
fn test_compile_with_limit_directive() {
    let input = "Person @limit(first: 10, offset: 5) { name }";
    let blocks = resolve_and_compile(input);

    match &blocks[0] {
        CompiledBlock::FindNodes { limit, offset, .. } => {
            assert_eq!(*limit, Some(10));
            assert_eq!(*offset, Some(5));
        }
        other => panic!("Expected FindNodes, got {:?}", other),
    }
}

// 6. @sort directive on edge traversal
#[test]
fn test_compile_sort_on_edge() {
    let input = r#"query {
        result(func: uid(Person:abc123)) {
            -[KNOWS]-> @sort(age: desc) { name, age }
        }
    }"#;
    let blocks = resolve_and_compile(input);

    match &blocks[0] {
        CompiledBlock::Traverse { steps, .. } => {
            let sort = steps[0].sort.as_ref().expect("Expected sort");
            assert_eq!(sort.field, "age");
            assert!(sort.descending);
        }
        other => panic!("Expected Traverse, got {:?}", other),
    }
}

// 7. Multi-block with variable capture -> correct execution order
#[test]
fn test_compile_multi_block_dependency_order() {
    let input = r#"query {
        friends as result(func: uid(Person:abc123)) {
            name
            -[KNOWS]-> { name }
        }
        related(func: uid(friends)) {
            name
        }
    }"#;
    let ast = parse_pql(input).unwrap();
    let schemas = make_schemas();
    let resolver = PqlResolver::new();
    let resolved = resolver.resolve(&ast, &schemas).unwrap();

    // Block 0 ("result") should come before block 1 ("related")
    assert_eq!(resolved.execution_order.len(), 2);
    assert_eq!(resolved.execution_order[0], 0);
    assert_eq!(resolved.execution_order[1], 1);

    let compiler = PqlCompiler::new();
    let blocks = compiler.compile(&resolved).unwrap();
    assert_eq!(blocks.len(), 2);

    // First block is a Traverse (uid with edge traversal)
    assert!(matches!(&blocks[0], CompiledBlock::Traverse { .. }));
    // Second block is a VariableRef
    match &blocks[1] {
        CompiledBlock::VariableRef { variable, .. } => {
            assert_eq!(variable, "friends");
        }
        other => panic!("Expected VariableRef, got {:?}", other),
    }
}

// 8. Explain output is human-readable
#[test]
fn test_explain_output() {
    let blocks = resolve_and_compile("Person @filter(age >= 30) { name, age }");
    let output = explain_query(&blocks);

    assert!(output.contains("Query Plan:"));
    assert!(output.contains("Block 1"));
    assert!(output.contains("Strategy: index_scan"));
    assert!(output.contains("Type: Person"));
    assert!(output.contains("Filter: age >= 30"));
    assert!(output.contains("Fields: [name, age]"));
}

// 9. Unknown entity type -> PqlError::UnknownEntityType
#[test]
fn test_schema_resolution_unknown_type() {
    let ast = parse_pql("Alien { name }").unwrap();
    let schemas = make_schemas();
    let resolver = PqlResolver::new();
    let result = resolver.resolve(&ast, &schemas);

    match result {
        Err(PqlError::UnknownEntityType(t)) => assert_eq!(t, "Alien"),
        other => panic!("Expected UnknownEntityType, got {:?}", other.err()),
    }
}

// 10. Unknown field -> PqlError::UnknownField
#[test]
fn test_schema_resolution_unknown_field() {
    let ast = parse_pql("Person { name, salary }").unwrap();
    let schemas = make_schemas();
    let resolver = PqlResolver::new();
    let result = resolver.resolve(&ast, &schemas);

    match result {
        Err(PqlError::UnknownField {
            entity_type, field, ..
        }) => {
            assert_eq!(entity_type, "Person");
            assert_eq!(field, "salary");
        }
        other => panic!("Expected UnknownField, got {:?}", other.err()),
    }
}

// 11. Unknown edge type -> PqlError::UnknownEdge
#[test]
fn test_schema_resolution_unknown_edge() {
    let input = r#"query {
        result(func: uid(Person:abc123)) {
            -[LIKES]-> { name }
        }
    }"#;
    let ast = parse_pql(input).unwrap();
    let schemas = make_schemas();
    let resolver = PqlResolver::new();
    let result = resolver.resolve(&ast, &schemas);

    match result {
        Err(PqlError::UnknownEdge {
            entity_type,
            edge_type,
        }) => {
            assert_eq!(entity_type, "Person");
            assert_eq!(edge_type, "LIKES");
        }
        other => panic!("Expected UnknownEdge, got {:?}", other.err()),
    }
}

// 12. allow_undeclared_edges bypasses edge validation
#[test]
fn test_allow_undeclared_edges() {
    let input = r#"query {
        result(func: uid(Person:abc123)) {
            -[RANDOM_EDGE]-> { name }
        }
    }"#;
    let ast = parse_pql(input).unwrap();
    let mut schemas = InMemorySchemaProvider::new();
    schemas.add_schema(SchemaInfo {
        entity_type: "Person".to_string(),
        fields: vec!["name".to_string()],
        edges: vec![],
        allow_undeclared_edges: true,
    });
    let resolver = PqlResolver::new();
    let result = resolver.resolve(&ast, &schemas);
    assert!(result.is_ok());
}

// 13. Explain output for point lookup
#[test]
fn test_explain_point_lookup() {
    let input = r#"query { result(func: uid(Person:abc123)) { name } }"#;
    let blocks = resolve_and_compile(input);
    let output = explain_query(&blocks);

    assert!(output.contains("Strategy: point_lookup"));
    assert!(output.contains("ID: abc123"));
    assert!(output.contains("Estimated cost: 1.0"));
    assert!(output.contains("Estimated rows: 1"));
}

// 14. Explain output for traversal
#[test]
fn test_explain_traversal() {
    let input = r#"query {
        result(func: uid(Person:abc123)) @cascade {
            -[KNOWS]-> @edge(weight > 0.5) { name }
        }
    }"#;
    let blocks = resolve_and_compile(input);
    let output = explain_query(&blocks);

    assert!(output.contains("Strategy: traversal"));
    assert!(output.contains("Cascade: true"));
    assert!(output.contains("Step 1:"));
    assert!(output.contains("Edge: KNOWS ->"));
    assert!(output.contains("Edge filter: weight > 0.5"));
}

// 15. FindNodes without filter -> full_scan strategy
#[test]
fn test_explain_full_scan() {
    let blocks = resolve_and_compile("Person { name }");
    let output = explain_query(&blocks);
    assert!(output.contains("Strategy: full_scan"));
}

// 16. Type query with limit only
#[test]
fn test_compile_type_with_limit_only() {
    let input = "Person @limit(first: 5) { name }";
    let blocks = resolve_and_compile(input);

    match &blocks[0] {
        CompiledBlock::FindNodes { limit, offset, .. } => {
            assert_eq!(*limit, Some(5));
            assert_eq!(*offset, None);
        }
        other => panic!("Expected FindNodes, got {:?}", other),
    }
}

// 17. Incoming edge direction
#[test]
fn test_compile_incoming_edge() {
    let input = r#"query {
        result(func: uid(Person:abc123)) {
            -[KNOWS]<- { name }
        }
    }"#;
    let blocks = resolve_and_compile(input);

    match &blocks[0] {
        CompiledBlock::Traverse { steps, .. } => {
            assert_eq!(steps[0].direction, PqlEdgeDirection::Incoming);
        }
        other => panic!("Expected Traverse, got {:?}", other),
    }
}

// 18. Per-node limit on edge traversal
#[test]
fn test_compile_edge_per_node_limit() {
    let input = r#"query {
        result(func: uid(Person:abc123)) {
            -[KNOWS]-> @limit(first: 3) { name }
        }
    }"#;
    let blocks = resolve_and_compile(input);

    match &blocks[0] {
        CompiledBlock::Traverse { steps, .. } => {
            assert_eq!(steps[0].per_node_limit, Some(3));
        }
        other => panic!("Expected Traverse, got {:?}", other),
    }
}
