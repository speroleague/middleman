//! Typed `.middleman/middleman.toml` configuration (spec section 12).
//!
//! Everything is defaulted, so an empty file is valid. Unknown fields are
//! rejected so `doctor` can surface typos instead of silently ignoring
//! them. This crate stays I/O-free: loading is a pure `TOML string
//! -> Config` parse performed at an edge.

use serde::{Deserialize, Serialize};

/// Output shape used when none is passed on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    Cir,
    Markdown,
    Json,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OutputConfig {
    /// Default packet format.
    pub format: OutputFormat,
    /// Default token budget applied to every packet.
    pub token_budget: u32,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            format: OutputFormat::Cir,
            token_budget: 4000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LimitsConfig {
    /// Files larger than this are never parsed (kb).
    pub max_file_kb: u32,
    /// Documents over this many lines are sampled, not parsed.
    pub max_lines: u64,
    /// Whole indexing run budget (seconds).
    pub time_budget_secs: u32,
    /// Directory depth bound for scans.
    pub max_depth: u8,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            max_file_kb: 100,
            max_lines: 2000,
            time_budget_secs: 30,
            max_depth: 12,
        }
    }
}

/// Globs evaluated relative to the repository root.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct IgnoreConfig {
    /// Additional paths to include (empty = all recognized files).
    pub include: Vec<String>,
    /// Paths to skip (always beats include).
    pub exclude: Vec<String>,
}

/// Retrieval-weight multipliers for scoring (spec section 9).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WeightConfig {
    pub test_overlap: f64,
    pub file_overlap: f64,
    pub expanded: f64,
    pub referenced: f64,
    pub accepted: f64,
    pub rejected: f64,
}

impl Default for WeightConfig {
    fn default() -> Self {
        Self {
            test_overlap: 2.5,
            file_overlap: 2.0,
            expanded: 1.0,
            referenced: 1.5,
            accepted: 3.0,
            rejected: 0.1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub project_name: String,
    pub output: OutputConfig,
    pub limits: LimitsConfig,
    pub ignore: IgnoreConfig,
    pub weight: WeightConfig,
    /// Harness the user most often works with: cline, kilo, pi,
    /// claude-code, codex, or cli.
    pub harness: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            project_name: String::new(),
            output: OutputConfig::default(),
            limits: LimitsConfig::default(),
            ignore: IgnoreConfig::default(),
            weight: WeightConfig::default(),
            harness: "cli".into(),
        }
    }
}

impl Config {
    /// Parses TOML; errors name the offending key/line.
    pub fn parse(toml: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(toml)
    }

    /// Serializes back to canonical TOML (used by `init`).
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_file_is_the_default_config() {
        let config = Config::parse("").expect("empty toml parses");
        assert_eq!(config, Config::default());
        assert_eq!(config.output.token_budget, 4000);
        assert!((config.weight.test_overlap - 2.5).abs() < f64::EPSILON);
    }

    #[test]
    fn explicit_values_are_respected_and_round_trip() {
        let toml = "
project_name = \"frontier\"
harness = \"cline\"
[output]
format = \"markdown\"
token_budget = 8000
[limits]
max_file_kb = 60
[ignore]
exclude = [\"target/**\"]
";
        let config = Config::parse(toml).expect("parses");
        assert_eq!(config.project_name, "frontier");
        assert_eq!(config.output.format, OutputFormat::Markdown);
        assert_eq!(config.limits.max_file_kb, 60);
        assert_eq!(config.ignore.exclude, vec!["target/**".to_string()]);

        let round_trip = Config::parse(&config.to_toml().expect("serializes")).expect("round trip");
        assert_eq!(config, round_trip);
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let error = Config::parse("project_nam = \"x\"").expect_err("typo must fail");
        assert!(error.to_string().contains("project_nam"));
    }
}
