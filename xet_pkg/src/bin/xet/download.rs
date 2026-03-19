use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;
use xet::xet_session::XetSession;
use xet_data::processing::XetFileInfo;
use xet_runtime::config::XetConfig;

use super::Cli;

#[derive(Args)]
pub struct DownloadArgs {
    /// Hex-encoded MerkleHash of the file (from `xet upload` output).
    pub hash: String,

    /// Expected file size in bytes (from `xet upload` or `xet query` output).
    pub size: u64,

    /// Destination file path. Parent directories are created if needed.
    pub output: PathBuf,
}

pub async fn run(cli: &Cli, config: XetConfig, args: &DownloadArgs) -> Result<()> {
    let session = super::session::build_xet_session(&cli.resolved_endpoint(), cli.resolved_token(), config).await?;
    run_download(session, args).await?;
    Ok(())
}

pub async fn run_download(session: XetSession, args: &DownloadArgs) -> Result<()> {
    if let Some(parent) = args.output.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let file_info = XetFileInfo {
        hash: args.hash.clone(),
        file_size: args.size,
        sha256: None,
    };

    let group = session.new_download_group().await?;
    let handle = group.download_file_to_path(file_info, args.output.clone()).await?;
    let results = group.finish().await?;

    // Check the per-task result so download errors surface with their real cause
    // rather than a confusing "file not found" from the metadata check below.
    let task_result = results.get(&handle.task_id).context("no download result returned")?;
    if let Err(e) = task_result.as_ref() {
        anyhow::bail!("download failed for {}: {e}", args.hash);
    }

    let bytes = std::fs::metadata(&args.output)
        .context("output file not found after download")?
        .len();
    eprintln!("Downloaded {} → {} ({} bytes)", args.hash, args.output.display(), bytes);

    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::session::build_xet_session;
    use crate::upload::{UploadArgs, run_upload};

    /// Helper: upload content, return (endpoint, hash, file_size).
    async fn upload_test_file(cas_dir: &tempfile::TempDir, name: &str, content: &[u8]) -> (String, String, u64) {
        let endpoint = format!("local://{}", cas_dir.path().display());
        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join(name);
        std::fs::write(&src, content).unwrap();

        let config = XetConfig::new();
        let session = build_xet_session(&endpoint, None, config).await.unwrap();
        let upload_args = UploadArgs {
            files: vec![src],
            no_sha256: true,
            output: None,
        };
        let results = run_upload(session, &upload_args).await.unwrap();
        let meta = &results[0];
        (endpoint, meta.hash.clone(), meta.file_size)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_download_roundtrip() {
        let cas_dir = tempdir().unwrap();
        let content = b"download test content 12345";
        let (endpoint, hash, size) = upload_test_file(&cas_dir, "data.bin", content).await;

        let dest_dir = tempdir().unwrap();
        let dest = dest_dir.path().join("out.bin");

        let session = build_xet_session(&endpoint, None, XetConfig::new()).await.unwrap();
        let args = DownloadArgs {
            hash,
            size,
            output: dest.clone(),
        };
        run_download(session, &args).await.unwrap();

        assert_eq!(std::fs::read(&dest).unwrap(), content);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_download_invalid_hash_returns_error() {
        let cas_dir = tempdir().unwrap();
        let endpoint = format!("local://{}", cas_dir.path().display());

        let dest_dir = tempdir().unwrap();
        let dest = dest_dir.path().join("should_not_exist.bin");

        let session = build_xet_session(&endpoint, None, XetConfig::new()).await.unwrap();
        let args = DownloadArgs {
            hash: "0".repeat(64),
            size: 100,
            output: dest.clone(),
        };
        let result = run_download(session, &args).await;
        assert!(result.is_err());
    }
}
