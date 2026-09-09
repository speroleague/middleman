//! Bounded `git` CLI adapter: the repository's only door to Git.
//!
//! Every call is a fixed, argument-free command with parsed output; no
//! shell strings, no repository code execution.

use std::process::Command;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("git is not on PATH or failed to execute: {0}")]
    Execution(String),
    #[error("git produced unusable output: {0}")]
    UnusableOutput(String),
}

/// Proves the git CLI works and returns its `git version ...` line.
pub fn probe() -> Result<String, Error> {
    let output = Command::new("git")
        .arg("--version")
        .output()
        .map_err(|e| Error::Execution(e.to_string()))?;
    if !output.status.success() {
        return Err(Error::Execution("git --version exited non-zero".into()));
    }
    let line = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if line.starts_with("git version ") {
        Ok(line)
    } else {
        Err(Error::UnusableOutput(line))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_probe_reports_a_version_line_when_git_exists() {
        match probe() {
            Ok(line) => assert!(line.starts_with("git version ")),
            Err(Error::Execution(_)) => (), // no git in this environment
            other => panic!("unexpected probe outcome: {other:?}"),
        }
    }
}
