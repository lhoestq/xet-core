use std::path::PathBuf;

use anyhow::Result;
use clap::Args;
use xet::xet_session::{FileMetadata, XetSession};
use xet_data::processing::Sha256Policy;
use xet_runtime::config::XetConfig;

use super::Cli;

#[derive(Args)]
pub struct UploadArgs {
    /// Files to upload.
    pub files: Vec<PathBuf>,

    /// Compute SHA256 hash during upload (default: true).
    #[arg(long = "sha256", default_value_t = true, action = clap::ArgAction::Set)]
    pub sha256: bool,

    /// Write JSON results to this file instead of stdout.
    #[arg(long)]
    pub output: Option<PathBuf>,
}

pub async fn run(cli: &Cli, config: XetConfig, args: &UploadArgs) -> Result<()> {
    let session = super::session::build_xet_session(&cli.resolved_endpoint(), cli.resolved_token(), config).await?;
    let results = run_upload(session, args).await?;
    if let Some(ref output_path) = args.output {
        let json = serde_json::to_string_pretty(&results)?;
        std::fs::write(output_path, json)?;
    } else {
        for meta in &results {
            println!(
                "{}  hash={}  size={}  sha256={}",
                meta.tracking_name.as_deref().unwrap_or("<unknown>"),
                meta.hash,
                meta.file_size,
                meta.sha256.as_deref().unwrap_or("-")
            );
        }
    }
    Ok(())
}

pub async fn run_upload(session: XetSession, args: &UploadArgs) -> Result<Vec<FileMetadata>> {
    let sha256 = if args.sha256 {
        Sha256Policy::Compute
    } else {
        Sha256Policy::Skip
    };

    let commit = session.new_upload_commit().await?;
    let mut handles = vec![];
    for path in &args.files {
        let handle = commit.upload_from_path(path.clone(), sha256).await?;
        handles.push((path.clone(), handle));
    }

    let results_map = commit.commit().await?;
    let mut output = vec![];
    let mut had_error = false;

    for (path, handle) in handles {
        let id = handle.task_id;
        match results_map.get(&id) {
            Some(result) => match result.as_ref() {
                Ok(meta) => output.push(meta.clone()),
                Err(e) => {
                    eprintln!("ERROR: {}: {e}", path.display());
                    had_error = true;
                },
            },
            None => {
                eprintln!("ERROR: {}: no result returned", path.display());
                had_error = true;
            },
        }
    }

    if had_error {
        anyhow::bail!("one or more files failed to upload");
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::session::build_xet_session;

    #[tokio::test]
    async fn test_upload_roundtrip() {
        let cas_dir = tempdir().unwrap();
        let endpoint = format!("local://{}", cas_dir.path().display());
        let config = xet_runtime::config::XetConfig::new();
        let session = build_xet_session(&endpoint, None, config).await.unwrap();

        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("hello.txt");
        std::fs::write(&src, b"hello xet world").unwrap();

        let args = UploadArgs {
            files: vec![src.clone()],
            sha256: true,
            output: None,
        };
        let results = run_upload(session, &args).await.unwrap();

        assert_eq!(results.len(), 1);
        let meta = &results[0];
        assert_eq!(meta.file_size, 15);
        assert!(!meta.hash.is_empty());
        assert!(meta.sha256.is_some());
    }
}
