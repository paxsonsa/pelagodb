//! REPL integration tests.
//!
//! These tests require a running `pelago-server` instance and are ignored by default.

#[tokio::test]
#[ignore = "requires running pelago-server"]
async fn test_repl_simple_query() {
    // Execute: `Person @filter(age >= 30) { name, age }`
    // Verify results are rendered in configured output format.
}

#[tokio::test]
#[ignore = "requires running pelago-server"]
async fn test_repl_explain() {
    // Execute: `:explain Person @filter(age >= 30) { name }`
    // Verify explain plan output includes strategy details.
}

#[tokio::test]
#[ignore = "requires running pelago-server"]
async fn test_repl_multi_block() {
    // Execute a multi-block PQL query with variable capture and verify ordered execution.
}
