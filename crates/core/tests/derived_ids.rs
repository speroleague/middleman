#![allow(clippy::unwrap_used, clippy::expect_used)]

use middleman_core::EntityId;

#[test]
fn derived_ids_round_trip_and_encode_components_without_collisions() {
    let id = EntityId::derived("sym", &["src/a b.rs", "function", "renew", "1"]).unwrap();
    assert_eq!(id.as_str(), "sym:src/a%20b.rs#function#renew#1");
    let encoded = serde_json::to_string(&id).unwrap();
    assert_eq!(serde_json::from_str::<EntityId>(&encoded).unwrap(), id);
    assert_ne!(
        EntityId::derived("mod", &["a#b"]).unwrap(),
        EntityId::derived("mod", &["a", "b"]).unwrap()
    );
    assert_ne!(
        EntityId::derived("mod", &["a%20b"]).unwrap(),
        EntityId::derived("mod", &["a b"]).unwrap()
    );
    assert!(EntityId::derived("doc", &["docs/日本語.md"]).is_ok());
}

#[test]
fn invalid_namespaces_keys_and_escapes_are_rejected() {
    for raw in [
        "mod:",
        "unknown:a",
        "mod:a b",
        "mod:a\n",
        "mod:a%",
        "mod:a%xx",
        "mod:a\\b",
        "sym:a##b",
    ] {
        assert!(EntityId::new(raw).is_err(), "accepted {raw:?}");
        assert!(serde_json::from_str::<EntityId>(&serde_json::to_string(raw).unwrap()).is_err());
    }
    assert!(EntityId::derived("unknown", &["a"]).is_err());
    assert!(EntityId::derived("mod", &[]).is_err());
    assert!(EntityId::derived("mod", &[""]).is_err());
}

#[test]
fn existing_entity_ulids_remain_compatible() {
    let id = EntityId::new("ent_01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    assert_eq!(
        serde_json::from_str::<EntityId>(&serde_json::to_string(&id).unwrap()).unwrap(),
        id
    );
}
