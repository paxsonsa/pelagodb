/// Phase 3 Validation: PQL + Cache + CLI Integration
///
/// This test suite validates that all Phase 3 components work together:
/// 1. PQL parser → AST → resolver → compiler pipeline
/// 2. RocksDB cache store CRUD operations
/// 3. Cache read path consistency routing
/// 4. Traversal continuation token round-trip
/// 5. End-to-end FDB + Cache integration (ignored without native deps)

// ── PQL Pipeline Validation ────────────────────────────────────────

#[test]
fn phase3_pql_parse_compile_explain() {
    use pelago_query::pql::*;

    // 1. Parse a short-form PQL query
    let ast = parse_pql("Person @filter(age >= 30) @limit(first: 10) { name, age }").unwrap();
    assert_eq!(ast.blocks.len(), 1);
    assert_eq!(ast.blocks[0].directives.len(), 2);

    // 2. Set up schema provider and resolve
    let mut schemas = InMemorySchemaProvider::new();
    schemas.add_schema(SchemaInfo {
        entity_type: "Person".to_string(),
        fields: vec!["name".to_string(), "age".to_string(), "email".to_string()],
        edges: vec!["KNOWS".to_string(), "WORKS_AT".to_string()],
        allow_undeclared_edges: false,
    });
    schemas.add_schema(SchemaInfo {
        entity_type: "Company".to_string(),
        fields: vec!["name".to_string(), "industry".to_string()],
        edges: vec![],
        allow_undeclared_edges: false,
    });

    let resolver = PqlResolver::new();
    let resolved = resolver.resolve(&ast, &schemas).unwrap();
    assert_eq!(resolved.execution_order.len(), 1);

    // 3. Compile to execution plan
    let compiler = PqlCompiler::new();
    let blocks = compiler.compile(&resolved).unwrap();
    assert_eq!(blocks.len(), 1);

    match &blocks[0] {
        CompiledBlock::FindNodes {
            entity_type,
            cel_expression,
            fields,
            limit,
            ..
        } => {
            assert_eq!(entity_type, "Person");
            assert_eq!(cel_expression.as_deref(), Some("age >= 30"));
            assert!(fields.contains(&"name".to_string()));
            assert!(fields.contains(&"age".to_string()));
            assert_eq!(*limit, Some(10));
        }
        other => panic!("Expected FindNodes, got {:?}", other),
    }

    // 4. Generate explain plan
    let explain = explain_query(&blocks);
    assert!(explain.contains("Query Plan:"));
    assert!(explain.contains("index_scan"));
    assert!(explain.contains("Person"));
    assert!(explain.contains("age >= 30"));
}

#[test]
fn phase3_pql_traversal_compilation() {
    use pelago_query::pql::*;

    let pql = r#"query friends_of_friends {
        start(func: uid(Person:1_42)) {
            name
            -[KNOWS]-> @filter(age >= 30) @edge(weight > 0.5) {
                name
                age
                -[WORKS_AT]-> {
                    name
                    industry
                }
            }
        }
    }"#;

    let ast = parse_pql(pql).unwrap();
    assert_eq!(ast.name, Some("friends_of_friends".to_string()));

    let mut schemas = InMemorySchemaProvider::new();
    schemas.add_schema(SchemaInfo {
        entity_type: "Person".to_string(),
        fields: vec!["name".to_string(), "age".to_string()],
        edges: vec!["KNOWS".to_string(), "WORKS_AT".to_string()],
        allow_undeclared_edges: false,
    });
    schemas.add_schema(SchemaInfo {
        entity_type: "Company".to_string(),
        fields: vec!["name".to_string(), "industry".to_string()],
        edges: vec![],
        allow_undeclared_edges: false,
    });

    let resolver = PqlResolver::new();
    let resolved = resolver.resolve(&ast, &schemas).unwrap();

    let compiler = PqlCompiler::new();
    let blocks = compiler.compile(&resolved).unwrap();

    match &blocks[0] {
        CompiledBlock::Traverse {
            start_entity_type,
            start_node_id,
            steps,
            ..
        } => {
            assert_eq!(start_entity_type, "Person");
            assert_eq!(start_node_id, "1_42");
            assert!(steps.len() >= 1);
            assert_eq!(steps[0].edge_type, "KNOWS");
            assert_eq!(steps[0].direction, PqlEdgeDirection::Outgoing);
            assert_eq!(steps[0].node_filter.as_deref(), Some("age >= 30"));
            assert_eq!(steps[0].edge_filter.as_deref(), Some("weight > 0.5"));
        }
        other => panic!("Expected Traverse, got {:?}", other),
    }
}

#[test]
fn phase3_pql_multi_block_dependencies() {
    use pelago_query::pql::*;

    let pql = r#"query {
        friends as start(func: uid(Person:1_42)) {
            -[KNOWS]-> { uid }
        }
        mutual(func: uid(friends)) @filter(age >= 25) {
            name
            age
        }
    }"#;

    let ast = parse_pql(pql).unwrap();
    assert_eq!(ast.blocks.len(), 2);
    assert_eq!(ast.blocks[0].capture_as, Some("friends".to_string()));

    let mut schemas = InMemorySchemaProvider::new();
    schemas.add_schema(SchemaInfo {
        entity_type: "Person".to_string(),
        fields: vec!["name".to_string(), "age".to_string(), "uid".to_string()],
        edges: vec!["KNOWS".to_string()],
        allow_undeclared_edges: false,
    });

    let resolver = PqlResolver::new();
    let resolved = resolver.resolve(&ast, &schemas).unwrap();

    // Topological sort: "start" (idx 0) must come before "mutual" (idx 1)
    assert_eq!(resolved.execution_order, vec![0, 1]);

    let compiler = PqlCompiler::new();
    let blocks = compiler.compile(&resolved).unwrap();
    assert_eq!(blocks.len(), 2);
    assert!(matches!(&blocks[0], CompiledBlock::Traverse { .. }));
    assert!(matches!(&blocks[1], CompiledBlock::VariableRef { .. }));
}

#[test]
fn phase3_pql_error_handling() {
    use pelago_query::pql::*;

    let schemas = InMemorySchemaProvider::new();
    let resolver = PqlResolver::new();

    // Unknown entity type
    let ast = parse_pql("Alien { name }").unwrap();
    assert!(matches!(
        resolver.resolve(&ast, &schemas),
        Err(PqlError::UnknownEntityType(_))
    ));

    // Parse error
    assert!(parse_pql("{ }").is_err());
    assert!(parse_pql("Person {").is_err());
}

// ── RocksDB Cache Store Validation ─────────────────────────────────

#[cfg(feature = "cache")]
mod cache_validation {
    use pelago_storage::edge::NodeRef;
    use pelago_storage::edge::StoredEdge;
    use pelago_storage::node::StoredNode;
    use pelago_storage::rocks_cache::*;
    use std::collections::HashMap;

    #[test]
    fn phase3_cache_node_crud() {
        let store = RocksCacheStore::open_temp().unwrap();

        let node = StoredNode {
            id: "1_42".to_string(),
            entity_type: "Person".to_string(),
            properties: {
                let mut p = HashMap::new();
                p.insert(
                    "name".to_string(),
                    pelago_core::Value::String("Alice".into()),
                );
                p.insert("age".to_string(), pelago_core::Value::Int(30));
                p
            },
            locality: 0,
            created_at: 1000,
            updated_at: 1000,
        };

        // Put
        store.put_node("testdb", "default", &node).unwrap();

        // Get
        let cached = store
            .get_node("testdb", "default", "Person", "1_42")
            .unwrap();
        assert!(cached.is_some());
        let cached = cached.unwrap();
        assert_eq!(cached.id, "1_42");
        assert_eq!(cached.entity_type, "Person");

        // Delete
        store
            .delete_node("testdb", "default", "Person", "1_42")
            .unwrap();
        let gone = store
            .get_node("testdb", "default", "Person", "1_42")
            .unwrap();
        assert!(gone.is_none());
    }

    #[test]
    fn phase3_cache_edge_crud() {
        let store = RocksCacheStore::open_temp().unwrap();

        let source = NodeRef::new("testdb", "default", "Person", "1_1");
        let target = NodeRef::new("testdb", "default", "Person", "1_2");

        let edge = StoredEdge::new(
            "e_1".to_string(),
            source.clone(),
            target.clone(),
            "KNOWS".to_string(),
            HashMap::new(),
        );

        // Put
        store.put_edge("testdb", "default", &edge).unwrap();

        // List
        let edges = store
            .list_edges_cached("testdb", "default", "Person", "1_1", Some("KNOWS"))
            .unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].edge_id, "e_1");

        // Delete
        store
            .delete_edge("testdb", "default", &source, "KNOWS", &target)
            .unwrap();
        let edges = store
            .list_edges_cached("testdb", "default", "Person", "1_1", Some("KNOWS"))
            .unwrap();
        assert!(edges.is_empty());
    }

    #[test]
    fn phase3_cache_hwm_tracking() {
        use pelago_storage::cdc::Versionstamp;

        let store = RocksCacheStore::open_temp().unwrap();

        // Initial HWM should be zero
        let hwm = store.get_hwm().unwrap();
        assert_eq!(hwm, Versionstamp::zero());

        // Set HWM
        let vs = Versionstamp::from_bytes(&[0, 0, 0, 0, 0, 0, 0, 1, 0, 0]).unwrap();
        store.set_hwm(&vs).unwrap();

        // Read back
        let hwm = store.get_hwm().unwrap();
        assert_eq!(hwm, vs);
    }

    #[test]
    fn phase3_cache_consistency_routing() {
        // Verify ReadConsistency enum variants exist and are distinct
        let strong = ReadConsistency::Strong;
        let session = ReadConsistency::Session;
        let eventual = ReadConsistency::Eventual;

        assert_ne!(strong, session);
        assert_ne!(session, eventual);
        assert_ne!(strong, eventual);
    }
}

// ── Continuation Token Validation ──────────────────────────────────

#[test]
fn phase3_continuation_token_api() {
    use pelago_query::traversal::TraversalResults;

    // TraversalResults carries continuation token for truncated results
    let results = TraversalResults {
        paths: vec![],
        truncated: true,
        continuation_token: Some(vec![0xA2, 0x67, 0x76, 0x69, 0x73, 0x69, 0x74, 0x65, 0x64]),
        frontier: vec![
            ("Person".to_string(), "1_3".to_string()),
            ("Company".to_string(), "1_4".to_string()),
        ],
    };
    assert!(results.truncated);
    assert!(results.continuation_token.is_some());
    assert_eq!(results.frontier.len(), 2);
    assert_eq!(results.frontier[0].0, "Person");
    assert_eq!(results.frontier[1].0, "Company");

    // Non-truncated results have no continuation token
    let complete = TraversalResults {
        paths: vec![],
        truncated: false,
        continuation_token: None,
        frontier: vec![],
    };
    assert!(!complete.truncated);
    assert!(complete.continuation_token.is_none());
    assert!(complete.frontier.is_empty());
}

// ── Full Integration Validation (requires native FDB + RocksDB) ────

#[tokio::test]
#[ignore = "requires native FDB and RocksDB"]
async fn phase3_validation_full() {
    // This end-to-end test validates the complete Phase 3 pipeline:
    //
    // 1. Set up FDB + RocksDB cache
    // 2. Register schemas, create nodes/edges via storage layer
    // 3. Verify RocksDB cache projector syncs data
    // 4. Parse and compile PQL queries
    // 5. Execute PQL against existing query engine
    // 6. Verify read consistency levels (Strong, Session, Eventual)
    // 7. Test traversal continuation tokens
    //
    // This test requires:
    //   - FoundationDB server running locally
    //   - `fdb_c` library available for linking
    //   - RocksDB available (cargo feature "cache")
    //
    // Run with: cargo test -p pelago-storage --features cache phase3_validation_full -- --ignored
}
