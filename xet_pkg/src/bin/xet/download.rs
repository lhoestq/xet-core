use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;
use xet::xet_session::XetSession;
use xet_data::processing::XetFileInfo;
use xet_runtime::config::XetConfig;

use super::Cli;

#[derive(Args)]
pub struct DownloadArgs {
    /// Hex-encoded MerkleHash of the file to download.
    pub hash: String,

    /// File size in bytes (required to construct XetFileInfo).
    pub size: u64,

    /// Destination path.
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
    group.download_file_to_path(file_info, args.output.clone()).await?;
    group.finish().await?;

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

    #[tokio::test(flavor = "multi_thread")]
    async fn test_download_roundtrip() {
        let cas_dir = tempdir().unwrap();
        let endpoint = format!("local://{}", cas_dir.path().display());

        // Upload a file first.
        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("data.bin");
        let content = b"download test content 12345";
        std::fs::write(&src, content).unwrap();

        let config = xet_runtime::config::XetConfig::new();
        let session = build_xet_session(&endpoint, None, config.clone()).await.unwrap();
        let upload_args = UploadArgs {
            files: vec![src],
            sha256: false,
            output: None,
        };
        let results = run_upload(session, &upload_args).await.unwrap();
        let meta = &results[0];

        // Now download it.
        let dest_dir = tempdir().unwrap();
        let dest = dest_dir.path().join("out.bin");

        let session2 = build_xet_session(&endpoint, None, config).await.unwrap();
        let args = DownloadArgs {
            hash: meta.hash.clone(),
            size: meta.file_size,
            output: dest.clone(),
        };
        run_download(session2, &args).await.unwrap();

        let downloaded = std::fs::read(&dest).unwrap();
        assert_eq!(downloaded, content);
    }
}
