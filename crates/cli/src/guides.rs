use clap::{Args, ValueEnum};
use middleman_packet::guides;
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Target {
    AgentsMd,
    AgentContext,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Format {
    Markdown,
    Json,
}

#[derive(Debug, Args)]
pub struct Options {
    #[arg(value_enum)]
    target: Target,
    /// Update the Middleman block in the fixed repository file.
    #[arg(long)]
    write: bool,
    #[arg(long, value_enum, default_value = "markdown")]
    format: Format,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("input: {0}")]
    Input(#[from] super::retrieval::Error),
    #[error("guide: {0}")]
    Guide(#[from] guides::Error),
    #[error("file operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("output path must be an ordinary repository file, without links")]
    UnsafePath,
    #[error("destination changed during generation; retry")]
    Changed,
}

impl Error {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Input(error) => error.code(),
            Self::Guide(_) => "invalid_guide",
            Self::Io(_) => "io_error",
            Self::UnsafePath => "unsafe_output_path",
            Self::Changed => "destination_changed",
        }
    }
}

pub fn run(repo: &Path, options: &Options) -> Result<(), Error> {
    guard(repo, true)?;
    let relative = match options.target {
        Target::AgentsMd => "AGENTS.md",
        Target::AgentContext => "docs/agent-context.md",
    };
    let destination = repo.join(relative);
    if matches!(options.target, Target::AgentContext) {
        guard(&repo.join("docs"), true)?;
    }
    let existing = read(&destination)?;
    let body = match options.target {
        Target::AgentsMd => {
            let state_dir = repo.join(super::STATE_DIR);
            if !state_dir.join(super::STATE_DB).is_file() {
                return Err(super::retrieval::Error::NotInitialized.into());
            }
            middleman_store::Store::open_read_only(&state_dir)
                .map_err(super::retrieval::Error::from)?;
            guides::agents_md()
        }
        Target::AgentContext => {
            let (project, entities) = super::retrieval::guide_input(repo)?;
            guides::agent_context(&project, &entities)?
        }
    };
    let content = guides::merge(existing.as_deref().unwrap_or(""), &body)?;
    let changed = existing.as_deref() != Some(content.as_str());
    if options.write && changed {
        replace(repo, &destination, existing.as_deref(), &content)?;
    }
    match options.format {
        Format::Json => println!(
            "{}",
            if options.write {
                serde_json::json!({"path": relative, "changed": changed})
            } else {
                serde_json::json!({"path": relative, "content": content})
            }
        ),
        Format::Markdown if options.write => println!(
            "{} {relative}",
            if changed { "updated" } else { "unchanged" }
        ),
        Format::Markdown => print!("{content}"),
    }
    Ok(())
}

fn guard(path: &Path, directory: bool) -> Result<Option<fs::Metadata>, Error> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(Error::UnsafePath);
        }
    }
    if metadata.file_type().is_symlink()
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
    {
        return Err(Error::UnsafePath);
    }
    Ok(Some(metadata))
}

fn read(path: &Path) -> Result<Option<String>, Error> {
    let Some(metadata) = guard(path, false)? else {
        return Ok(None);
    };
    if metadata.len() > guides::MAX_BYTES as u64 {
        return Err(guides::Error::TooLarge.into());
    }
    let mut text = String::new();
    fs::File::open(path)?
        .take(guides::MAX_BYTES as u64 + 1)
        .read_to_string(&mut text)?;
    if text.len() > guides::MAX_BYTES {
        return Err(guides::Error::TooLarge.into());
    }
    Ok(Some(text))
}

fn replace(
    repo: &Path,
    destination: &Path,
    existing: Option<&str>,
    content: &str,
) -> Result<(), Error> {
    let parent = destination.parent().ok_or(Error::UnsafePath)?;
    guard(repo, true)?;
    guard(parent, true)?;
    fs::create_dir_all(parent)?;
    guard(parent, true)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(content.as_bytes())?;
    if let Some(metadata) = guard(destination, false)? {
        temporary
            .as_file()
            .set_permissions(metadata.permissions())?;
    }
    temporary.as_file().sync_all()?;
    if read(destination)?.as_deref() != existing {
        return Err(Error::Changed);
    }
    if existing.is_some() {
        temporary
            .persist(destination)
            .map_err(|error| error.error)?;
    } else {
        temporary
            .persist_noclobber(destination)
            .map_err(|error| error.error)?;
    }
    Ok(())
}
