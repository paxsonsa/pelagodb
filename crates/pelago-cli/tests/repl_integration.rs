//! REPL integration tests.
//!
//! These tests are ignored by default because they require FoundationDB plus a local
//! `pelago-server` binary. Use:
//! `cargo test -p pelago-cli --test repl_integration -- --ignored --nocapture`

mod common;

use common::CliTestFixture;

#[test]
#[ignore = "requires FoundationDB + pelago-server binary (see module docs)"]
fn test_repl_explain_and_query_script() {
    let fixture = CliTestFixture::start("repl_explain");
    let person_type = fixture.unique_entity_type("PersonW5Repl");

    let person_schema = serde_json::json!({
        "name": person_type,
        "properties": {
            "name": { "type": "string", "required": true, "index": "equality" },
            "age": { "type": "int", "required": false, "index": "range" }
        }
    })
    .to_string();
    fixture.run_json(&["schema", "register", "--inline", person_schema.as_str()]);

    let props = serde_json::json!({ "name": "Alice", "age": 34 }).to_string();
    fixture.run_json(&["node", "create", &person_type, "--props", props.as_str()]);

    let explain_query = format!("{} @filter(age >= 30) {{ name }}", person_type);
    let execute_query = format!("{} @filter(age >= 30) {{ name, age }}", person_type);
    let script = format!(
        ":format json\n:explain {}\n{}\n:quit\n",
        explain_query, execute_query
    );

    let output = fixture.run_repl_script(&script);
    assert!(
        output.status.success(),
        "repl command failed with status {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Query Plan:"),
        "expected explain output in repl stdout:\n{}",
        stdout
    );
    assert!(
        stdout.contains("Strategy: index_scan"),
        "expected strategy details in explain output:\n{}",
        stdout
    );
    assert!(
        stdout.contains("\"name\": \"Alice\""),
        "expected query result row in repl output:\n{}",
        stdout
    );
    assert!(
        stdout.contains("result(s)"),
        "expected repl summary line in output:\n{}",
        stdout
    );
}
