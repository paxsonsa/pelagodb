#![no_main]

use libfuzzer_sys::fuzz_target;
use pelago_query::pql::{parse_pql, PqlCompiler, PqlResolver, SchemaInfo};

fuzz_target!(|data: &[u8]| {
    let Ok(input) = std::str::from_utf8(data) else {
        return;
    };

    let Ok(ast) = parse_pql(input) else {
        return;
    };

    let mut provider = pelago_query::pql::InMemorySchemaProvider::new();
    provider.add_schema(SchemaInfo {
        entity_type: "Person".to_string(),
        fields: vec![
            "name".to_string(),
            "age".to_string(),
            "email".to_string(),
            "status".to_string(),
        ],
        edges: vec![
            "KNOWS".to_string(),
            "WORKS_AT".to_string(),
            "AUTHORED".to_string(),
        ],
        allow_undeclared_edges: true,
    });
    provider.add_schema(SchemaInfo {
        entity_type: "Company".to_string(),
        fields: vec!["name".to_string(), "industry".to_string()],
        edges: vec![],
        allow_undeclared_edges: true,
    });

    let resolver = PqlResolver::new();
    let Ok(resolved) = resolver.resolve(&ast, &provider) else {
        return;
    };

    let compiler = PqlCompiler::new();
    let _ = compiler.compile(&resolved);
});

