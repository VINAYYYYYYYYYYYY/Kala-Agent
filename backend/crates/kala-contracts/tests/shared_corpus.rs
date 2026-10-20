//! Read-only checks against the frozen shared corpus.
//!
//! The shared corpus is owned by the corpus cell; this test only reads it.

use std::path::PathBuf;

use kala_contracts::{ContractModel, DomainPackageManifest, model_json_limits};
use kala_json::{CanonicalValue, parse_strict};

fn fixture(path: &str) -> Vec<u8> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    std::fs::read(root.join(path)).expect("shared fixture")
}

fn member<'a>(value: &'a CanonicalValue, name: &str) -> Option<&'a CanonicalValue> {
    match value {
        CanonicalValue::Object(object) => object.get(&kala_json::JsonText::new(name)),
        _ => None,
    }
}

#[test]
fn domain_provenance_manifests_validate_against_the_real_corpus() {
    let bytes = fixture("corpus/v1/domain_provenance.json");
    let root = parse_strict(&bytes, &model_json_limits()).expect("corpus JSON");
    let Some(CanonicalValue::Array(packages)) = member(&root, "packages") else {
        panic!("packages missing");
    };
    assert_eq!(packages.len(), 3);
    for package in packages {
        let manifest = member(package, "manifest").expect("manifest");
        let parsed = DomainPackageManifest::from_value(manifest).expect("manifest validates");
        assert!(!parsed.package_id().is_empty());
        assert!(parsed.to_wire_json_ascii().contains(parsed.package_id()));
        assert!(!parsed.to_canonical_json().unwrap().is_empty());
    }
}
