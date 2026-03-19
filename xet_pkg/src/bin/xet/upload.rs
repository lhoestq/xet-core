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

    /// Skip SHA-256 hash computation during upload.
    #[arg(long)]
    pub no_sha256: bool,

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
    let sha256 = if args.no_sha256 {
        Sha256Policy::Skip
    } else {
        Sha256Policy::Compute
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

    async fn local_session(cas_dir: &tempfile::TempDir) -> (String, XetSession) {
        let endpoint = format!("local://{}", cas_dir.path().display());
        let session = build_xet_session(&endpoint, None, XetConfig::new()).await.unwrap();
        (endpoint, session)
    }

    #[tokio::test]
    async fn test_upload_single_file() {
        let cas_dir = tempdir().unwrap();
        let (_endpoint, session) = local_session(&cas_dir).await;

        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("hello.txt");
        std::fs::write(&src, b"hello xet world").unwrap();

        let args = UploadArgs {
            files: vec![src],
            no_sha256: false,
            output: None,
        };
        let results = run_upload(session, &args).await.unwrap();

        assert_eq!(results.len(), 1);
        let meta = &results[0];
        assert_eq!(meta.file_size, 15);
        assert!(!meta.hash.is_empty());
        assert!(meta.sha256.is_some());
    }

    #[tokio::test]
    async fn test_upload_multiple_files() {
        let cas_dir = tempdir().unwrap();
        let (_endpoint, session) = local_session(&cas_dir).await;

        let src_dir = tempdir().unwrap();
        let files: Vec<PathBuf> = (0..5)
            .map(|i| {
                let path = src_dir.path().join(format!("file_{i}.bin"));
                std::fs::write(&path, format!("content for file {i}").as_bytes()).unwrap();
                path
            })
            .collect();

        let args = UploadArgs {
            files: files.clone(),
            no_sha256: true,
            output: None,
        };
        let results = run_upload(session, &args).await.unwrap();

        assert_eq!(results.len(), 5);
        for (i, meta) in results.iter().enumerate() {
            assert_eq!(meta.file_size, format!("content for file {i}").len() as u64);
            assert!(!meta.hash.is_empty());
        }
    }

    #[tokio::test]
    async fn test_upload_sha256_policy_propagation() {
        let cas_dir = tempdir().unwrap();
        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("data.bin");
        std::fs::write(&src, b"sha256 test data").unwrap();

        // SHA-256 enabled (default)
        let (_, session) = local_session(&cas_dir).await;
        let args = UploadArgs {
            files: vec![src.clone()],
            no_sha256: false,
            output: None,
        };
        let with_sha = run_upload(session, &args).await.unwrap();
        assert!(with_sha[0].sha256.is_some());

        // SHA-256 disabled
        let cas_dir2 = tempdir().unwrap();
        let (_, session2) = local_session(&cas_dir2).await;
        let args = UploadArgs {
            files: vec![src],
            no_sha256: true,
            output: None,
        };
        let without_sha = run_upload(session2, &args).await.unwrap();
        assert!(without_sha[0].sha256.is_none());
    }

    #[tokio::test]
    async fn test_upload_json_output() {
        let cas_dir = tempdir().unwrap();
        let (_endpoint, session) = local_session(&cas_dir).await;

        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("json_test.txt");
        std::fs::write(&src, b"json output test").unwrap();

        let out_dir = tempdir().unwrap();
        let json_path = out_dir.path().join("results.json");

        let args = UploadArgs {
            files: vec![src],
            no_sha256: false,
            output: Some(json_path.clone()),
        };
        let results = run_upload(session, &args).await.unwrap();
        let json = serde_json::to_string_pretty(&results).unwrap();
        std::fs::write(&json_path, &json).unwrap();

        let parsed: Vec<FileMetadata> = serde_json::from_str(&std::fs::read_to_string(&json_path).unwrap()).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].file_size, 16);
        assert!(!parsed[0].hash.is_empty());
    }
}
