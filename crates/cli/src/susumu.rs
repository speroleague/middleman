//! Optional, bounded delegation to a repository-local Susumu workflow.
//!
//! Middleman owns task routing and reviewed broker memory. Susumu owns its
//! evidence, expectation, verification, decision, and review-record model.
//! This adapter deliberately invokes Susumu's JSON digest rather than parsing
//! `.susu` files or reproducing its scanner and review rules.

use std::{path::Path, process::Command, time::Duration};

use serde::Deserialize;

const MARKERS: &[&str] = &[
    "susumu.toml",
    ".susumu/project.susu",
    ".susumu/review.susu",
    ".susumu/check.json",
    "expectations.susu",
    "verifications.susu",
    "decisions.susu",
    "work.susu",
    "reviews.susu",
];
const TIMEOUT: Duration = Duration::from_secs(10);
const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const DIGEST_SCHEMA_VERSION: &str = "susumu.digest.v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Detection {
    markers: Vec<&'static str>,
}

impl Detection {
    pub(super) fn marker_summary(&self) -> String {
        self.markers.join(", ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Status {
    Absent,
    Unavailable(Detection),
    Available {
        detection: Detection,
        digest: Digest,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct Digest {
    schema_version: String,
    project: serde_json::Value,
    review: Review,
    result: ResultSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct Review {
    critical: usize,
    warning: usize,
    attention: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct ResultSummary {
    status: String,
}

impl Digest {
    fn project_name(&self) -> Option<&str> {
        self.project
            .as_str()
            .or_else(|| self.project.get("name").and_then(serde_json::Value::as_str))
    }
}

pub(super) fn detect(repo: &Path) -> Option<Detection> {
    let markers: Vec<_> = MARKERS
        .iter()
        .copied()
        .filter(|marker| repo.join(marker).is_file())
        .collect();
    (!markers.is_empty()).then_some(Detection { markers })
}

fn status(repo: &Path) -> Status {
    let Some(detection) = detect(repo) else {
        return Status::Absent;
    };
    let mut command = Command::new("susumu");
    command.args(["digest", ".", "--json"]).current_dir(repo);
    let Ok(output) = middleman_indexer::process::run(&mut command, TIMEOUT, MAX_OUTPUT_BYTES)
    else {
        return Status::Unavailable(detection);
    };
    if output.code != Some(0) {
        return Status::Unavailable(detection);
    }
    match serde_json::from_slice::<Digest>(&output.stdout) {
        Ok(digest) if digest.schema_version == DIGEST_SCHEMA_VERSION => {
            Status::Available { detection, digest }
        }
        Err(_) | Ok(_) => Status::Unavailable(detection),
    }
}

pub(super) fn print_status(repo: &Path) {
    match status(repo) {
        Status::Absent => {}
        Status::Unavailable(detection) => println!(
            "susumu:    detected ({}) — native digest unavailable",
            detection.marker_summary()
        ),
        Status::Available { detection, digest } => {
            println!(
                "susumu:    {} — result={} ({} attention)",
                digest.project_name().unwrap_or("project"),
                digest.result.status,
                digest.review.attention
            );
            println!(
                "susumu:    review: {} critical, {} warning, {} attention ({})",
                digest.review.critical,
                digest.review.warning,
                digest.review.attention,
                detection.marker_summary()
            );
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn detects_each_documented_susumu_marker_without_reading_artifacts() {
        let repo = tempfile::tempdir().expect("temporary repository");
        fs::create_dir(repo.path().join(".susumu")).expect("state directory");
        fs::write(repo.path().join("susumu.toml"), "[portal]\n").expect("config marker");
        fs::write(repo.path().join(".susumu/project.susu"), "not parsed").expect("artifact marker");

        let detection = detect(repo.path()).expect("Susumu should be detected");

        assert_eq!(
            detection.marker_summary(),
            "susumu.toml, .susumu/project.susu"
        );
    }

    #[test]
    fn ignores_susumu_marker_names_when_they_are_directories() {
        let repo = tempfile::tempdir().expect("temporary repository");
        fs::create_dir(repo.path().join("susumu.toml")).expect("marker directory");

        assert_eq!(detect(repo.path()), None);
    }

    #[test]
    fn parses_the_versioned_native_digest_contract_without_requiring_extra_fields() {
        let digest: Digest = serde_json::from_str(
            r#"{"schema_version":"susumu.digest.v1","project":{"name":"fixture"},"evidence":{"files":3},"records":{"expectations":2},"review":{"critical":1,"warning":2,"attention":3},"result":{"status":"failed"},"review_items":[],"next_actions":[],"future_field":true}"#,
        )
        .expect("current Susumu digest");

        assert_eq!(digest.schema_version, DIGEST_SCHEMA_VERSION);
        assert_eq!(digest.project_name(), Some("fixture"));
        assert_eq!(digest.review.attention, 3);
        assert_eq!(digest.result.status, "failed");
    }
}
