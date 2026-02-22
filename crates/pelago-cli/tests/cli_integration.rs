//! CLI integration tests.
//!
//! These tests are ignored by default because they require FoundationDB plus a local
//! `pelago-server` binary. Use:
//! `cargo test -p pelago-cli --test cli_integration -- --ignored --nocapture`

mod common;

use common::CliTestFixture;
use serde_json::Value;

#[test]
#[ignore = "requires FoundationDB + pelago-server binary (see module docs)"]
fn test_cli_schema_node_edge_query_workflow() {
    let fixture = CliTestFixture::start("cli_workflow");
    let person_type = fixture.unique_entity_type("PersonW5");
    let company_type = fixture.unique_entity_type("CompanyW5");

    let person_v1 = serde_json::json!({
        "name": person_type,
        "properties": {
            "name": { "type": "string", "required": true, "index": "equality" },
            "age": { "type": "int", "required": false, "index": "range" }
        },
        "meta": { "allow_undeclared_edges": true }
    })
    .to_string();
    let person_register_v1 =
        fixture.run_json(&["schema", "register", "--inline", person_v1.as_str()]);
    assert_eq!(person_register_v1["version"], 1);
    assert_eq!(person_register_v1["created"], true);

    let person_v2 = serde_json::json!({
        "name": person_type,
        "properties": {
            "name": { "type": "string", "required": true, "index": "equality" },
            "age": { "type": "int", "required": false, "index": "range" },
            "email": { "type": "string", "required": false, "index": "none" }
        },
        "meta": { "allow_undeclared_edges": true }
    })
    .to_string();
    let person_register_v2 =
        fixture.run_json(&["schema", "register", "--inline", person_v2.as_str()]);
    assert_eq!(person_register_v2["version"], 2);
    assert_eq!(person_register_v2["created"], true);

    let company_schema = serde_json::json!({
        "name": company_type,
        "properties": {
            "name": { "type": "string", "required": true, "index": "equality" },
            "industry": { "type": "string", "required": false, "index": "none" }
        }
    })
    .to_string();
    let company_register =
        fixture.run_json(&["schema", "register", "--inline", company_schema.as_str()]);
    assert_eq!(company_register["version"], 1);

    let person_get = fixture.run_json(&["schema", "get", &person_type]);
    assert_eq!(person_get["name"], person_type);
    assert_eq!(person_get["version"], 2);

    let schema_list = fixture.run_json(&["schema", "list"]);
    let schemas = schema_list
        .as_array()
        .expect("schema list should return json array");
    assert!(schemas.iter().any(|s| s["name"] == person_type));
    assert!(schemas.iter().any(|s| s["name"] == company_type));

    let diff = fixture.run_json(&["schema", "diff", &person_type, "1", "2"]);
    let changes = diff["changes"]
        .as_array()
        .expect("schema diff should return changes array");
    assert!(changes
        .iter()
        .any(|c| c["kind"] == "added" && c["field"] == "email"));

    let alice_props = serde_json::json!({ "name": "Alice", "age": 31 }).to_string();
    let alice = fixture.run_json(&[
        "node",
        "create",
        &person_type,
        "--props",
        alice_props.as_str(),
    ]);
    let alice_id = alice["id"]
        .as_str()
        .expect("alice id missing from create output")
        .to_string();

    let bob_props = serde_json::json!({ "name": "Bob", "age": 25 }).to_string();
    let bob = fixture.run_json(&[
        "node",
        "create",
        &person_type,
        "--props",
        bob_props.as_str(),
    ]);
    let bob_id = bob["id"]
        .as_str()
        .expect("bob id missing from create output")
        .to_string();

    let company_props = serde_json::json!({ "name": "Acme", "industry": "Software" }).to_string();
    let company = fixture.run_json(&[
        "node",
        "create",
        &company_type,
        "--props",
        company_props.as_str(),
    ]);
    let company_id = company["id"]
        .as_str()
        .expect("company id missing from create output")
        .to_string();

    let fetched_alice = fixture.run_json(&["node", "get", &person_type, &alice_id]);
    assert_eq!(fetched_alice["properties"]["name"], "Alice");
    assert_eq!(fetched_alice["properties"]["age"], 31);

    let bob_update_props = serde_json::json!({ "age": 32 }).to_string();
    let bob_updated = fixture.run_json(&[
        "node",
        "update",
        &person_type,
        &bob_id,
        "--props",
        bob_update_props.as_str(),
    ]);
    assert_eq!(bob_updated["properties"]["age"], 32);

    let listed_people = fixture.run_json(&["node", "list", &person_type, "--limit", "50"]);
    assert!(json_array_has_id(&listed_people, &alice_id));
    assert!(json_array_has_id(&listed_people, &bob_id));

    let source_ref = format!("{}:{}", person_type, bob_id);
    let target_ref = format!("{}:{}", company_type, company_id);
    let edge_props = serde_json::json!({ "since": 2020 }).to_string();
    let created_edge = fixture.run_json(&[
        "edge",
        "create",
        source_ref.as_str(),
        "WORKS_AT",
        target_ref.as_str(),
        "--props",
        edge_props.as_str(),
    ]);
    assert_eq!(created_edge["label"], "WORKS_AT");

    let listed_edges = fixture.run_json(&[
        "edge",
        "list",
        &person_type,
        &bob_id,
        "--dir",
        "out",
        "--label",
        "WORKS_AT",
    ]);
    let edges = listed_edges
        .as_array()
        .expect("edge list should return array");
    assert!(edges.iter().any(|edge| edge["target"] == target_ref));

    let found_people = fixture.run_json(&[
        "query",
        "find",
        &person_type,
        "--filter",
        "age >= 30",
        "--limit",
        "50",
    ]);
    assert!(json_array_has_id(&found_people, &alice_id));
    assert!(json_array_has_id(&found_people, &bob_id));

    let traversed = fixture.run_json(&[
        "query",
        "traverse",
        source_ref.as_str(),
        "WORKS_AT",
        "--max-depth",
        "1",
        "--max-results",
        "50",
    ]);
    assert!(json_array_has_id(&traversed, &company_id));

    let pql = format!("{} @filter(age >= 30) {{ name, age }}", person_type);
    let pql_results = fixture.run_json(&["query", "pql", "--query", pql.as_str()]);
    assert!(json_array_has_id(&pql_results, &alice_id));
    assert!(json_array_has_id(&pql_results, &bob_id));

    let edge_deleted = fixture.run_json(&[
        "edge",
        "delete",
        source_ref.as_str(),
        "WORKS_AT",
        target_ref.as_str(),
    ]);
    assert_eq!(edge_deleted["deleted"], true);

    let bob_deleted = fixture.run_json(&["node", "delete", &person_type, &bob_id, "--force"]);
    let alice_deleted = fixture.run_json(&["node", "delete", &person_type, &alice_id, "--force"]);
    let company_deleted =
        fixture.run_json(&["node", "delete", &company_type, &company_id, "--force"]);
    assert_eq!(bob_deleted["deleted"], true);
    assert_eq!(alice_deleted["deleted"], true);
    assert_eq!(company_deleted["deleted"], true);
}

fn json_array_has_id(value: &Value, node_id: &str) -> bool {
    value
        .as_array()
        .map(|items| {
            items.iter().any(|item| {
                item.get("id")
                    .and_then(Value::as_str)
                    .map(|id| id == node_id)
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}
