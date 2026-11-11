//! Every operation in spec/freeastro.json maps to exactly one client method.
//!
//! `OPERATIONS` is generated next to the methods; tests/operations.rs calls each of those methods
//! (so they exist and hit the right method and path). This test closes the loop against the
//! vendored spec itself, so a spec update without regeneration fails here too.

use std::collections::BTreeMap;

use serde_json::Value;

#[test]
fn every_operation_id_maps_to_a_method() {
    let spec: Value = serde_json::from_str(include_str!("../spec/freeastro.json")).unwrap();
    let mut in_spec = BTreeMap::new();
    for (path, item) in spec["paths"].as_object().unwrap() {
        for (method, op) in item.as_object().unwrap() {
            if let Some(id) = op.get("operationId").and_then(Value::as_str) {
                in_spec.insert(id.to_owned(), (method.to_uppercase(), path.clone()));
            }
        }
    }
    let in_sdk: BTreeMap<_, _> = freeastroapi::OPERATIONS
        .iter()
        .map(|op| {
            assert!(!op.accessor.is_empty());
            (
                op.operation_id.to_owned(),
                (op.method.to_owned(), op.path.to_owned()),
            )
        })
        .collect();
    assert_eq!(
        in_spec.len(),
        freeastroapi::OPERATIONS.len(),
        "duplicate operation ids"
    );
    let missing: Vec<_> = in_spec
        .keys()
        .filter(|id| !in_sdk.contains_key(*id))
        .collect();
    let extra: Vec<_> = in_sdk
        .keys()
        .filter(|id| !in_spec.contains_key(*id))
        .collect();
    assert!(
        missing.is_empty(),
        "operations without a method: {missing:?}"
    );
    assert!(
        extra.is_empty(),
        "methods for operations not in the spec: {extra:?}"
    );
    assert_eq!(in_spec, in_sdk);
}
