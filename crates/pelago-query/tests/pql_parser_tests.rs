use pelago_query::pql::*;

#[test]
fn test_parse_short_form() {
    let ast = parse_pql("Person @filter(age >= 30) { name, age }").unwrap();
    assert_eq!(ast.blocks.len(), 1);
    assert_eq!(ast.blocks[0].name, "result");
    match &ast.blocks[0].root {
        RootFunction::Type(qt) => assert_eq!(qt.entity_type, "Person"),
        _ => panic!("Expected Type root function"),
    }
    assert_eq!(ast.blocks[0].directives.len(), 1);
    // Verify field selections
    let fields: Vec<&str> = ast.blocks[0]
        .selections
        .iter()
        .filter_map(|s| match s {
            Selection::Field(f) => Some(f.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(fields, vec!["name", "age"]);
}

#[test]
fn test_parse_short_form_no_filter() {
    let ast = parse_pql("Person { name, age }").unwrap();
    assert_eq!(ast.blocks.len(), 1);
    assert_eq!(ast.blocks[0].directives.len(), 0);
}

#[test]
fn test_parse_full_query() {
    let pql = r#"query friends_of_friends {
        start(func: uid(Person:1_42)) {
            name
            age
            -[KNOWS]-> @filter(age >= 30) {
                name
                -[WORKS_AT]-> {
                    name
                    industry
                }
            }
        }
    }"#;
    let ast = parse_pql(pql).unwrap();
    assert_eq!(ast.blocks.len(), 1);
    assert_eq!(ast.name, Some("friends_of_friends".to_string()));
    assert_eq!(ast.blocks[0].name, "start");
    // Verify nested edge traversals exist
    let has_edges = ast.blocks[0]
        .selections
        .iter()
        .any(|s| matches!(s, Selection::Edge(_)));
    assert!(has_edges, "Should have edge selections");
}

#[test]
fn test_parse_multi_block_with_variables() {
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
    assert_eq!(ast.blocks[0].name, "start");
}

#[test]
fn test_parse_edge_directions() {
    let pql = r#"query {
        start(func: uid(Person:1_42)) {
            -[KNOWS]-> { name }
            -[FOLLOWS]<- { name }
            -[RELATED]<-> { name }
        }
    }"#;
    let ast = parse_pql(pql).unwrap();
    let edges: Vec<&EdgeTraversal> = ast.blocks[0]
        .selections
        .iter()
        .filter_map(|s| match s {
            Selection::Edge(e) => Some(e),
            _ => None,
        })
        .collect();
    assert_eq!(edges.len(), 3);
    assert_eq!(edges[0].direction, PqlEdgeDirection::Outgoing);
    assert_eq!(edges[1].direction, PqlEdgeDirection::Incoming);
    assert_eq!(edges[2].direction, PqlEdgeDirection::Both);
}

#[test]
fn test_parse_directives() {
    let pql =
        "Person @filter(age >= 30) @limit(first: 10, offset: 5) @sort(age: desc) @cascade { name }";
    let ast = parse_pql(pql).unwrap();
    assert_eq!(ast.blocks[0].directives.len(), 4);
}

#[test]
fn test_parse_aggregations() {
    let pql = r#"query {
        start(func: type(Person)) {
            name
            count(-[KNOWS]->)
            avg(age)
        }
    }"#;
    let ast = parse_pql(pql).unwrap();
    let aggs: Vec<_> = ast.blocks[0]
        .selections
        .iter()
        .filter(|s| matches!(s, Selection::Aggregate(_)))
        .collect();
    assert_eq!(aggs.len(), 2);
}

#[test]
fn test_parse_root_functions() {
    // type()
    let ast = parse_pql("Person { name }").unwrap();
    assert!(matches!(&ast.blocks[0].root, RootFunction::Type(_)));

    // uid()
    let pql = "query { start(func: uid(Person:1_42)) { name } }";
    let ast = parse_pql(pql).unwrap();
    assert!(matches!(&ast.blocks[0].root, RootFunction::Uid(_)));

    // eq()
    let pql = r#"query { start(func: eq(name, "Alice")) { name } }"#;
    let ast = parse_pql(pql).unwrap();
    assert!(matches!(&ast.blocks[0].root, RootFunction::Eq(_, _)));

    // has()
    let pql = "query { start(func: has(email)) { name } }";
    let ast = parse_pql(pql).unwrap();
    assert!(matches!(&ast.blocks[0].root, RootFunction::Has(_)));
}

#[test]
fn test_parse_errors() {
    assert!(parse_pql("Person {").is_err());
    assert!(parse_pql("").is_err());
    assert!(parse_pql("{ }").is_err());
}

#[test]
fn test_parse_cross_namespace() {
    let pql = r#"query {
        start(func: uid(Person:1_42)) {
            -[core:WORKS_AT]-> core:Company {
                name
            }
        }
    }"#;
    let ast = parse_pql(pql).unwrap();
    let edges: Vec<&EdgeTraversal> = ast.blocks[0]
        .selections
        .iter()
        .filter_map(|s| match s {
            Selection::Edge(e) => Some(e),
            _ => None,
        })
        .collect();
    assert_eq!(edges[0].edge_namespace, Some("core".to_string()));
    assert_eq!(
        edges[0].target_type.as_ref().unwrap().namespace,
        Some("core".to_string())
    );
}

#[test]
fn test_parse_comments() {
    let pql = r#"# This is a comment
    Person {
        name  # inline comment
        age
    }"#;
    let ast = parse_pql(pql).unwrap();
    let fields: Vec<&str> = ast.blocks[0]
        .selections
        .iter()
        .filter_map(|s| match s {
            Selection::Field(f) => Some(f.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(fields, vec!["name", "age"]);
}
