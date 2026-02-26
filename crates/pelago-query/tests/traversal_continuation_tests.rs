use pelago_query::traversal::*;

#[test]
fn test_traversal_results_has_continuation_fields() {
    let results = TraversalResults {
        paths: vec![],
        truncated: false,
        continuation_token: None,
        frontier: vec![],
        scanned_keys: 0,
        result_bytes: 0,
        elapsed_ms: 0,
    };
    assert!(!results.truncated);
    assert!(results.continuation_token.is_none());
    assert!(results.frontier.is_empty());
}

#[test]
fn test_traversal_results_with_continuation() {
    let results = TraversalResults {
        paths: vec![],
        truncated: true,
        continuation_token: Some(vec![1, 2, 3]),
        frontier: vec![("Person".to_string(), "1_42".to_string())],
        scanned_keys: 12,
        result_bytes: 64,
        elapsed_ms: 3,
    };
    assert!(results.truncated);
    assert!(results.continuation_token.is_some());
    assert_eq!(results.frontier.len(), 1);
    assert_eq!(results.frontier[0].0, "Person");
    assert_eq!(results.frontier[0].1, "1_42");
}

#[test]
fn test_traversal_results_multiple_frontier_entries() {
    let results = TraversalResults {
        paths: vec![],
        truncated: true,
        continuation_token: Some(vec![0xA2]), // arbitrary bytes
        frontier: vec![
            ("Person".to_string(), "1_1".to_string()),
            ("Company".to_string(), "1_2".to_string()),
            ("Person".to_string(), "1_3".to_string()),
        ],
        scanned_keys: 99,
        result_bytes: 1024,
        elapsed_ms: 11,
    };
    assert_eq!(results.frontier.len(), 3);
    assert_eq!(results.frontier[1].0, "Company");
}

#[tokio::test]
#[ignore = "requires native FDB"]
async fn test_traversal_continuation_token() {
    // 1. Create graph with > 100 nodes at depth 1
    // 2. Traverse with max_results = 50
    // 3. Verify results.truncated == true
    // 4. Verify results.continuation_token is present
    // 5. Resume traversal with continuation_token
    // 6. Verify remaining results returned
    // 7. Verify no duplicates across pages
}

#[tokio::test]
#[ignore = "requires native FDB"]
async fn test_traversal_frontier_tracking() {
    // 1. Create graph with high fan-out
    // 2. Traverse with low max_results
    // 3. Verify truncation and non-empty frontier
    // 4. Resume traversal and verify frontier shrinks/clears
}
