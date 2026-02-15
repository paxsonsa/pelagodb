//! CLI integration tests.
//!
//! These tests require a running `pelago-server` instance and are ignored by default.

#[tokio::test]
#[ignore = "requires running pelago-server"]
async fn test_schema_register_and_get() {
    // Example flow:
    // 1. `pelago schema register person.json`
    // 2. `pelago schema get Person`
    // 3. Verify schema fields in output
}

#[tokio::test]
#[ignore = "requires running pelago-server"]
async fn test_node_crud() {
    // Example flow:
    // 1. `pelago node create Person --props '{\"name\":\"Alice\",\"age\":30}'`
    // 2. `pelago node get Person <id>`
    // 3. `pelago node update Person <id> --props '{\"age\":31}'`
    // 4. `pelago node delete Person <id> --force`
}

#[tokio::test]
#[ignore = "requires running pelago-server"]
async fn test_edge_crud() {
    // Example flow:
    // 1. `pelago edge create Person:1_42 WORKS_AT Company:1_7`
    // 2. `pelago edge list Person 1_42`
    // 3. `pelago edge delete Person:1_42 WORKS_AT Company:1_7`
}
