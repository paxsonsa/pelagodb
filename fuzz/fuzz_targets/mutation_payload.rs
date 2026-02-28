#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use pelago_core::schema::{EntitySchema, IndexType, PropertyDef};
use pelago_core::{PropertyType, Value};
use pelago_storage::index::{compute_index_entries, compute_index_removals};
use pelago_storage::Subspace;
use std::collections::HashMap;

#[derive(Arbitrary, Debug)]
struct MutationPayload {
    old_email: Option<String>,
    new_email: Option<String>,
    old_age: Option<i64>,
    new_age: Option<i64>,
}

fn schema() -> EntitySchema {
    EntitySchema::new("Person")
        .with_property(
            "email",
            PropertyDef::new(PropertyType::String).with_index(IndexType::Unique),
        )
        .with_property(
            "age",
            PropertyDef::new(PropertyType::Int).with_index(IndexType::Range),
        )
}

fn to_props(email: Option<String>, age: Option<i64>) -> HashMap<String, Value> {
    let mut props = HashMap::new();
    if let Some(v) = email {
        props.insert("email".to_string(), Value::String(v));
    }
    if let Some(v) = age {
        props.insert("age".to_string(), Value::Int(v));
    }
    props
}

fuzz_target!(|payload: MutationPayload| {
    let old_props = to_props(payload.old_email, payload.old_age);
    let new_props = to_props(payload.new_email, payload.new_age);
    let schema = schema();
    let subspace = Subspace::namespace("db", "ns");
    let node_id = [1u8; 8];

    let _ = compute_index_entries(&subspace, "Person", &node_id, &schema, &old_props);
    let _ = compute_index_entries(&subspace, "Person", &node_id, &schema, &new_props);
    let _ = compute_index_removals(
        &subspace,
        "Person",
        &node_id,
        &schema,
        &old_props,
        &new_props,
    );
});

