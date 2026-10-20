//! Native provenance mutation and determinism tests.
//!
//! These tests copy the crate's Rust sources and the source trees of the
//! accepted local dependencies, mutate evaluator, sibling simulation, constants,
//! shared-helper, and dependency files, and prove the native digest rules
//! react. They also prove the build-embedded constants equal a fresh
//! recomputation of the current tree.

use std::fs;
use std::path::{Path, PathBuf};

use kala_domains::source_audit::{
    DependencySourceRoot, LOCAL_DEPENDENCIES, SourceRoots, crate_source_roots,
    current_domain_digests, digest_domain, embedded_identity, source_files,
};
use kala_domains::{
    EvaluatorImplementation, NATIVE_IMPLEMENTATION_DIGEST_VERSION, NATIVE_IMPLEMENTATION_LANGUAGE,
};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn crates_dir() -> PathBuf {
    manifest_dir()
        .parent()
        .expect("kala-domains has a parent crates directory")
        .to_path_buf()
}

fn copy_dir(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let destination = target.join(entry.file_name());
        if path.is_dir() {
            copy_dir(&path, &destination);
        } else {
            fs::copy(&path, &destination).unwrap();
        }
    }
}

fn copy_dependency(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for name in ["Cargo.toml", "build.rs"] {
        let path = source.join(name);
        if path.is_file() {
            fs::copy(&path, target.join(name)).unwrap();
        }
    }
    copy_dir(&source.join("src"), &target.join("src"));
}

fn copy_sources(temp: &tempfile::TempDir) -> SourceRoots {
    let src = temp.path().join("src");
    copy_dir(&manifest_dir().join("src"), &src);
    let dependencies = LOCAL_DEPENDENCIES
        .iter()
        .map(|name| {
            let target = temp.path().join(name);
            copy_dependency(&crates_dir().join(name), &target);
            DependencySourceRoot::new(*name, target)
        })
        .collect::<Vec<_>>();
    SourceRoots::new(src, dependencies)
}

fn append(path: &Path, text: &str) {
    let mut contents = fs::read_to_string(path).unwrap();
    contents.push_str(text);
    fs::write(path, contents).unwrap();
}

fn mechanical_geometry(module_digest: &str, package_digest: &str) -> EvaluatorImplementation {
    EvaluatorImplementation::new(
        "mechanical.geometry-check",
        "kala_domains.mechanical.evaluation.GeometryEvaluator",
        "kala_domains.mechanical.evaluation",
        module_digest,
        package_digest,
        &[],
    )
    .unwrap()
}

#[test]
fn embedded_constants_equal_a_fresh_source_recomputation() {
    let roots = crate_source_roots();
    for domain in ["mechanical", "building", "wing"] {
        let embedded = embedded_identity(domain).expect("embedded identity");
        assert_eq!(embedded.module_digest.len(), 64);
        assert_eq!(embedded.package_digest.len(), 64);
        let first = current_domain_digests(domain).unwrap();
        let second = current_domain_digests(domain).unwrap();
        assert_eq!(first, second, "repeated recomputation must be stable");
        assert_eq!(first.module_digest, embedded.module_digest);
        assert_eq!(first.package_digest, embedded.package_digest);
        assert_ne!(first.module_digest, first.package_digest);
        let files = source_files(&roots, domain).unwrap();
        assert!(
            files
                .iter()
                .any(|file| file.label.starts_with("rust-package:"))
        );
        assert!(
            files
                .iter()
                .any(|file| file.label == "rust-shared:input.rs")
        );
        assert!(files.iter().any(|file| file.label == "rust-shared:lib.rs"));
        for dependency in LOCAL_DEPENDENCIES {
            assert!(
                files
                    .iter()
                    .any(|file| file.label == format!("rust-dependency:{dependency}/Cargo.toml")),
                "missing {dependency} manifest in source identity"
            );
            assert!(
                files
                    .iter()
                    .any(|file| file.label == format!("rust-dependency:{dependency}/src/lib.rs")),
                "missing {dependency} source in source identity"
            );
        }
    }
}

#[test]
fn evaluator_sibling_constant_and_shared_helper_mutations_change_native_digests() {
    let temp = tempfile::tempdir().unwrap();
    let roots = copy_sources(&temp);
    let src = &roots.src_root;
    let baseline_mechanical = digest_domain(&roots, "mechanical").unwrap();
    let baseline_building = digest_domain(&roots, "building").unwrap();
    let baseline_wing = digest_domain(&roots, "wing").unwrap();
    let baseline_implementation = mechanical_geometry(
        &baseline_mechanical.module_digest,
        &baseline_mechanical.package_digest,
    );

    // Evaluator source: module and package digests change; other domains do not.
    append(
        &src.join("mechanical/evaluation.rs"),
        "\nconst MUTATION_PROBE: u8 = 1;\n",
    );
    let after_evaluator = digest_domain(&roots, "mechanical").unwrap();
    assert_ne!(
        after_evaluator.module_digest,
        baseline_mechanical.module_digest
    );
    assert_ne!(
        after_evaluator.package_digest,
        baseline_mechanical.package_digest
    );
    assert_eq!(
        digest_domain(&roots, "building").unwrap(),
        baseline_building
    );
    assert_eq!(digest_domain(&roots, "wing").unwrap(), baseline_wing);
    let evaluator_identity = mechanical_geometry(
        &after_evaluator.module_digest,
        &after_evaluator.package_digest,
    );
    assert_ne!(
        evaluator_identity.digest(),
        baseline_implementation.digest()
    );
    assert_ne!(
        evaluator_identity.module_digest(),
        baseline_implementation.module_digest()
    );

    // Sibling simulation: the evaluator module digest stays, the package digest
    // and therefore the implementation digest change.
    let before_sibling = digest_domain(&roots, "mechanical").unwrap();
    append(
        &src.join("mechanical/simulation.rs"),
        "\nconst MUTATION_PROBE: u8 = 2;\n",
    );
    let after_sibling = digest_domain(&roots, "mechanical").unwrap();
    assert_eq!(after_sibling.module_digest, before_sibling.module_digest);
    assert_ne!(after_sibling.package_digest, before_sibling.package_digest);
    let sibling_identity =
        mechanical_geometry(&after_sibling.module_digest, &after_sibling.package_digest);
    assert_ne!(
        sibling_identity.digest(),
        mechanical_geometry(
            &before_sibling.module_digest,
            &before_sibling.package_digest
        )
        .digest()
    );

    // Constants module.
    let before_constants = digest_domain(&roots, "mechanical").unwrap();
    append(
        &src.join("mechanical/parameters.rs"),
        "\nconst MUTATION_PROBE: u8 = 3;\n",
    );
    let after_constants = digest_domain(&roots, "mechanical").unwrap();
    assert_eq!(
        after_constants.module_digest,
        before_constants.module_digest
    );
    assert_ne!(
        after_constants.package_digest,
        before_constants.package_digest
    );

    // Shared helper: every domain package digest changes.
    append(&src.join("input.rs"), "\nconst MUTATION_PROBE: u8 = 4;\n");
    for (domain, baseline) in [
        ("mechanical", baseline_mechanical),
        ("building", baseline_building),
        ("wing", baseline_wing),
    ] {
        let after_shared = digest_domain(&roots, domain).unwrap();
        assert_ne!(after_shared.package_digest, baseline.package_digest);
    }
}

#[test]
fn local_dependency_mutations_change_native_package_identity() {
    let temp = tempfile::tempdir().unwrap();
    let roots = copy_sources(&temp);
    let baseline: Vec<_> = ["mechanical", "building", "wing"]
        .into_iter()
        .map(|domain| (domain, digest_domain(&roots, domain).unwrap()))
        .collect();

    // The accepted score-rounding helper is part of every evaluator's behavior.
    let budget_compat = temp.path().join("kala-budget/src/compat.rs");
    let original = fs::read_to_string(&budget_compat).unwrap();
    append(&budget_compat, "\nconst MUTATION_PROBE: u8 = 5;\n");
    for (domain, before) in &baseline {
        let after = digest_domain(&roots, domain).unwrap();
        assert_eq!(after.module_digest, before.module_digest);
        assert_ne!(after.package_digest, before.package_digest);
        let implementation = mechanical_geometry(&after.module_digest, &after.package_digest);
        assert_ne!(
            implementation.digest(),
            mechanical_geometry(&before.module_digest, &before.package_digest).digest()
        );
    }
    fs::write(&budget_compat, original).unwrap();
    for (domain, before) in &baseline {
        assert_eq!(digest_domain(&roots, domain).unwrap(), *before);
    }

    // Canonical JSON / CPython float formatting source.
    let json_float = temp.path().join("kala-json/src/float.rs");
    let before = digest_domain(&roots, "mechanical").unwrap();
    append(&json_float, "\nconst MUTATION_PROBE: u8 = 6;\n");
    let after = digest_domain(&roots, "mechanical").unwrap();
    assert_eq!(after.module_digest, before.module_digest);
    assert_ne!(after.package_digest, before.package_digest);

    // Contract model source.
    let wire = temp.path().join("kala-contracts/src/wire.rs");
    let before = digest_domain(&roots, "building").unwrap();
    append(&wire, "\nconst MUTATION_PROBE: u8 = 7;\n");
    let after = digest_domain(&roots, "building").unwrap();
    assert_eq!(after.module_digest, before.module_digest);
    assert_ne!(after.package_digest, before.package_digest);

    // Dependency manifest identity.
    let manifest = temp.path().join("kala-contracts/Cargo.toml");
    let before = digest_domain(&roots, "wing").unwrap();
    append(&manifest, "\n# mutation probe\n");
    let after = digest_domain(&roots, "wing").unwrap();
    assert_eq!(after.module_digest, before.module_digest);
    assert_ne!(after.package_digest, before.package_digest);
}

#[test]
fn identical_source_copies_produce_identical_digests_after_restoration() {
    let first_temp = tempfile::tempdir().unwrap();
    let second_temp = tempfile::tempdir().unwrap();
    let first = copy_sources(&first_temp);
    let second = copy_sources(&second_temp);
    for domain in ["mechanical", "building", "wing"] {
        assert_eq!(
            digest_domain(&first, domain).unwrap(),
            digest_domain(&second, domain).unwrap()
        );
    }
    let original = fs::read_to_string(first.src_root.join("mechanical/evaluation.rs")).unwrap();
    let baseline = digest_domain(&first, "mechanical").unwrap();
    append(
        &first.src_root.join("mechanical/evaluation.rs"),
        "\nconst PROBE: u8 = 9;\n",
    );
    assert_ne!(
        digest_domain(&first, "mechanical").unwrap().module_digest,
        baseline.module_digest
    );
    fs::write(first.src_root.join("mechanical/evaluation.rs"), original).unwrap();
    assert_eq!(digest_domain(&first, "mechanical").unwrap(), baseline);
}

#[test]
fn native_scheme_is_explicit_and_capabilities_are_bound() {
    let identity = embedded_identity("mechanical").unwrap();
    let baseline = mechanical_geometry(identity.module_digest, identity.package_digest);
    let payload = baseline.digest_payload();
    let text = kala_domains::canonical_json(&payload).unwrap();
    assert!(text.contains(NATIVE_IMPLEMENTATION_DIGEST_VERSION));
    assert!(text.contains(NATIVE_IMPLEMENTATION_LANGUAGE));
    assert!(!text.contains("evaluator-implementation-digest-v2"));
    assert_eq!(
        baseline.qualified_name(),
        "kala_domains.mechanical.evaluation.GeometryEvaluator"
    );

    let capable = mechanical_geometry(identity.module_digest, identity.package_digest);
    let with_capability = EvaluatorImplementation::new(
        "mechanical.geometry-check",
        "kala_domains.mechanical.evaluation.GeometryEvaluator",
        "kala_domains.mechanical.evaluation",
        identity.module_digest,
        identity.package_digest,
        &["mechanical.surrogate.read"],
    )
    .unwrap();
    assert_ne!(capable.digest(), with_capability.digest());
    assert_eq!(
        EvaluatorImplementation::new(
            "mechanical.geometry-check",
            "other.module.GeometryEvaluator",
            "kala_domains.mechanical.evaluation",
            identity.module_digest,
            identity.package_digest,
            &[],
        )
        .unwrap_err()
        .class_name(),
        "DomainPackageError"
    );
}
