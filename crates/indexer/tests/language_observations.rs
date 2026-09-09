#![allow(clippy::unwrap_used, clippy::expect_used)]

use middleman_core::Config;
use middleman_indexer::{
    language::{self, Language, Visibility},
    scan,
};
use std::path::Path;

#[test]
fn fixture_declarations_imports_and_test_hints_are_stable() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let config = Config::default();
    for (fixture, filename, expected, imported) in [
        ("rust-workspace", "frontier.rs", "renew", None),
        (
            "laravel-app",
            "PostController.php",
            "index",
            Some("App\\Models\\Post"),
        ),
        ("elm-frontend", "Main.elm", "update", Some("Model")),
    ] {
        let files = scan::scan(&root.join(fixture), &config).unwrap().files;
        let file = files
            .iter()
            .find(|f| f.path.file_name().unwrap() == filename)
            .unwrap();
        let lang = Language::for_path(&file.path).unwrap();
        let result = language::parse(lang, &file.content, &config.limits).unwrap();
        assert_eq!(
            result,
            language::parse(lang, &file.content, &config.limits).unwrap()
        );
        let declaration = result
            .declarations
            .iter()
            .find(|d| d.name == expected)
            .unwrap();
        assert_eq!(declaration.visibility, Visibility::Public);
        assert!(declaration.line > 0);
        if let Some(imported) = imported {
            assert!(result.imports.iter().any(|i| i.target == imported));
        }
    }
    let result = language::parse(
        Language::Rust,
        "#[test]\nfn active_owner() {}\nfn helper() {}",
        &config.limits,
    )
    .unwrap();
    assert!(result.declarations[0].is_test);
    assert!(!result.declarations[1].is_test);
}

#[test]
fn supported_languages_extract_small_public_surfaces() {
    let limits = Config::default().limits;
    for (lang, text, name, import) in [
        (
            Language::Python,
            "from leases import Lease\nasync def renew(lease):\n    return lease",
            "renew",
            "leases",
        ),
        (
            Language::TypeScript,
            "import { Lease } from './lease';\nexport function renew(lease: Lease) {}",
            "renew",
            "./lease",
        ),
        (
            Language::JavaScript,
            "import Lease from './lease.js';\nexport const renew = lease => lease;",
            "renew",
            "./lease.js",
        ),
        (
            Language::Go,
            "package lease\nimport \"time\"\nfunc Renew() {}",
            "Renew",
            "time",
        ),
    ] {
        let result = language::parse(lang, text, &limits).unwrap();
        assert_eq!(result.declarations[0].name, name);
        assert_eq!(result.declarations[0].visibility, Visibility::Public);
        assert!(result.exports.iter().any(|export| export == name));
        assert_eq!(result.imports[0].target, import);
    }
}

#[test]
fn comments_and_multiline_strings_do_not_become_declarations() {
    let limits = Config::default().limits;
    for (lang, text) in [
        (
            Language::Rust,
            "/* outer\n/* nested */\npub fn fake() {}\n*/\nconst DOC: &str = r###\"\npub fn fake() {}\n\"###;\npub fn real() {}",
        ),
        (
            Language::Python,
            "\"\"\"\ndef fake():\n    pass\n\"\"\"\ndef real():\n    pass",
        ),
        (
            Language::Elm,
            "{- outer\n{- nested -}\nfake : Int\n-}\nreal : Int\nreal = 1",
        ),
        (
            Language::TypeScript,
            "const doc = `\nexport function fake() {}\n`;\nexport function real() {}",
        ),
    ] {
        let result = language::parse(lang, text, &limits).unwrap();
        assert!(!result.declarations.iter().any(|d| d.name == "fake"));
        assert!(result.declarations.iter().any(|d| d.name == "real"));
    }
}

#[test]
fn method_receivers_grouped_imports_and_restricted_visibility_are_preserved() {
    let limits = Config::default().limits;
    let go = language::parse(
        Language::Go,
        "import (\n\"time\"\n\"fmt\"\n)\nfunc (l Lease) Renew() {}",
        &limits,
    )
    .unwrap();
    assert_eq!(go.imports.len(), 2);
    assert_eq!(go.declarations[0].name, "Renew");
    let rust = language::parse(Language::Rust, "pub(crate) fn renew() {}", &limits).unwrap();
    assert_eq!(rust.declarations[0].name, "renew");
    assert_eq!(rust.declarations[0].visibility, Visibility::Internal);
}

#[test]
fn parser_limits_and_signature_redaction_are_explicit() {
    let mut limits = Config::default().limits;
    let parsed = language::parse(
        Language::Python,
        "def sample(value = 'synthetic value'):",
        &limits,
    )
    .unwrap();
    assert!(!parsed.declarations[0].signature.contains("synthetic value"));
    assert!(language::parse(Language::Python, "\0", &limits).is_err());
    limits.max_lines = 1;
    assert!(language::parse(Language::Rust, "fn a() {}\nfn b() {}", &limits).is_err());
}

#[test]
fn exposing_type_constructors_does_not_export_private_elm_functions() {
    let result = language::parse(Language::Elm, "module Model exposing (Model(..), count)\ntype Model = Empty\ncount : Int\ncount = 0\nprivate : Int\nprivate = 1", &Config::default().limits).unwrap();
    let private = result
        .declarations
        .iter()
        .find(|d| d.name == "private")
        .unwrap();
    assert_eq!(private.visibility, Visibility::Internal);
}
