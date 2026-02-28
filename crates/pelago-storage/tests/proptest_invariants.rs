use pelago_core::schema::{EntitySchema, IndexType, PropertyDef};
use pelago_core::{NodeId, PropertyType, Value};
use pelago_storage::cdc::Versionstamp;
use pelago_storage::index::{compute_index_entries, compute_index_removals};
use pelago_storage::Subspace;
use proptest::prelude::*;
use std::collections::{HashMap, HashSet};

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

fn index_keys(
    props: &HashMap<String, Value>,
) -> Result<HashSet<Vec<u8>>, pelago_core::PelagoError> {
    let subspace = Subspace::namespace("db", "ns");
    let node_id = NodeId::new(1, 99).to_bytes();
    let entries = compute_index_entries(&subspace, "Person", &node_id, &schema(), props)?;
    Ok(entries.iter().map(|e| e.key.to_vec()).collect())
}

fn removals_keys(
    old_props: &HashMap<String, Value>,
    new_props: &HashMap<String, Value>,
) -> Result<HashSet<Vec<u8>>, pelago_core::PelagoError> {
    let subspace = Subspace::namespace("db", "ns");
    let node_id = NodeId::new(1, 99).to_bytes();
    let removals = compute_index_removals(
        &subspace,
        "Person",
        &node_id,
        &schema(),
        old_props,
        new_props,
    )?;
    Ok(removals.iter().map(|e| e.key.to_vec()).collect())
}

fn props_strategy() -> impl Strategy<Value = HashMap<String, Value>> {
    (
        prop_oneof![
            Just(Value::Null),
            "\\PC{1,16}".prop_map(Value::String),
            Just(Value::Null),
        ],
        prop_oneof![Just(Value::Null), any::<i64>().prop_map(Value::Int),],
    )
        .prop_map(|(email, age)| {
            let mut map = HashMap::new();
            if !email.is_null() {
                map.insert("email".to_string(), email);
            }
            if !age.is_null() {
                map.insert("age".to_string(), age);
            }
            map
        })
}

#[derive(Clone, Debug)]
enum Op {
    SetEmail(String),
    ClearEmail,
    SetAge(i64),
    ClearAge,
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        "\\PC{1,16}".prop_map(Op::SetEmail),
        Just(Op::ClearEmail),
        any::<i64>().prop_map(Op::SetAge),
        Just(Op::ClearAge),
    ]
}

proptest! {
    #[test]
    fn versionstamp_roundtrip_and_next_is_monotonic(bytes in any::<[u8;10]>()) {
        let vs = Versionstamp::from_bytes(&bytes).expect("10 bytes should always parse");
        prop_assert_eq!(vs.to_bytes(), &bytes);

        if bytes != [0xFF; 10] {
            let next = vs.next();
            prop_assert!(next > vs);
        }
    }

    #[test]
    fn index_diff_matches_new_snapshot(old_props in props_strategy(), new_props in props_strategy()) {
        let old_keys = index_keys(&old_props).expect("old index keys");
        let new_keys = index_keys(&new_props).expect("new index keys");
        let removals = removals_keys(&old_props, &new_props).expect("removals");

        let mut projected = old_keys
            .difference(&removals)
            .cloned()
            .collect::<HashSet<_>>();
        let adds = new_keys
            .difference(&old_keys)
            .cloned()
            .collect::<HashSet<_>>();
        projected.extend(adds.into_iter());

        prop_assert_eq!(projected, new_keys);
    }

    #[test]
    fn stateful_index_invariant_holds(ops in prop::collection::vec(op_strategy(), 1..64)) {
        let mut current = HashMap::<String, Value>::new();
        for op in ops {
            let mut next = current.clone();
            match op {
                Op::SetEmail(v) => {
                    next.insert("email".to_string(), Value::String(v));
                }
                Op::ClearEmail => {
                    next.remove("email");
                }
                Op::SetAge(v) => {
                    next.insert("age".to_string(), Value::Int(v));
                }
                Op::ClearAge => {
                    next.remove("age");
                }
            }

            let old_keys = index_keys(&current).expect("old index keys");
            let new_keys = index_keys(&next).expect("new index keys");
            let removals = removals_keys(&current, &next).expect("removals");

            let mut projected = old_keys
                .difference(&removals)
                .cloned()
                .collect::<HashSet<_>>();
            let adds = new_keys
                .difference(&old_keys)
                .cloned()
                .collect::<HashSet<_>>();
            projected.extend(adds.into_iter());

            prop_assert_eq!(projected, new_keys);
            current = next;
        }
    }
}
