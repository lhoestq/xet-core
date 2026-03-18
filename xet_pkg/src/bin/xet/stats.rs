use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::Args;
use walkdir::WalkDir;
use xet_data::deduplication::DeduplicationMetrics;
use xet_data::processing::configurations::TranslatorConfig;
use xet_data::processing::{FileUploadSession, Sha256Policy};

use super::Cli;

#[derive(Args)]
pub struct StatsArgs {
    /// Files or directories to analyze.
    pub files: Vec<PathBuf>,

    /// Process directories recursively.
    #[arg(short, long)]
    pub recursive: bool,

    /// Write JSON results to this file instead of stdout.
    #[arg(long)]
    pub output: Option<PathBuf>,
}

pub async fn run(cli: &Cli, args: &StatsArgs) -> Result<()> {
    let config = super::session::build_translator_config(&cli.resolved_endpoint())?;
    let metrics = run_stats(config, args).await?;
    if let Some(output_path) = &args.output {
        let json = serde_json::to_string_pretty(&serde_json::json!({
            "total_bytes": metrics.total_bytes,
            "new_bytes": metrics.new_bytes,
            "deduped_bytes": metrics.deduped_bytes,
            "deduped_bytes_by_global_dedup": metrics.deduped_bytes_by_global_dedup,
            "defrag_prevented_dedup_bytes": metrics.defrag_prevented_dedup_bytes,
            "total_chunks": metrics.total_chunks,
            "new_chunks": metrics.new_chunks,
            "deduped_chunks": metrics.deduped_chunks,
            "deduped_chunks_by_global_dedup": metrics.deduped_chunks_by_global_dedup,
            "defrag_prevented_dedup_chunks": metrics.defrag_prevented_dedup_chunks,
            "xorb_bytes_uploaded": metrics.xorb_bytes_uploaded,
            "shard_bytes_uploaded": metrics.shard_bytes_uploaded,
            "total_bytes_uploaded": metrics.total_bytes_uploaded,
        }))?;
        std::fs::write(output_path, json)?;
    } else {
        println!(
            "total_bytes={}  new_bytes={}  deduped_bytes={}  uploaded_bytes={}",
            metrics.total_bytes, metrics.new_bytes, metrics.deduped_bytes, metrics.total_bytes_uploaded
        );
        if metrics.total_bytes > 0 {
            let dedup_pct = 100.0 * metrics.deduped_bytes as f64 / metrics.total_bytes as f64;
            let compression_pct = if metrics.new_bytes > 0 {
                100.0 * (1.0 - metrics.total_bytes_uploaded as f64 / metrics.new_bytes as f64)
            } else {
                0.0
            };
            println!("dedup_ratio={dedup_pct:.1}%  compression_ratio={compression_pct:.1}%");
        }
    }
    Ok(())
}

pub async fn run_stats(config: Arc<TranslatorConfig>, args: &StatsArgs) -> Result<DeduplicationMetrics> {
    let session: Arc<FileUploadSession> = FileUploadSession::dry_run(config).await?;
    let files = collect_files(&args.files, args.recursive);
    let file_entries: Vec<(PathBuf, Sha256Policy)> = files.into_iter().map(|p| (p, Sha256Policy::Skip)).collect();
    session.upload_files(file_entries).await?;
    let metrics = session.finalize().await?;
    Ok(metrics)
}

fn collect_files(inputs: &[PathBuf], recursive: bool) -> Vec<PathBuf> {
    if !recursive {
        return inputs.to_vec();
    }
    inputs
        .iter()
        .flat_map(|p| {
            WalkDir::new(p)
                .follow_links(false)
                .into_iter()
                .flatten()
                .filter(|e| e.file_type().is_file())
                .map(|e| e.path().to_path_buf())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[tokio::test]
    async fn test_stats_dry_run() {
        let work_dir = tempdir().unwrap();

        // 3 copies of the same 1KB block to trigger dedup.
        let src = work_dir.path().join("data.bin");
        let block = vec![42u8; 1024];
        let mut content = block.clone();
        content.extend_from_slice(&block);
        content.extend_from_slice(&block);
        std::fs::write(&src, &content).unwrap();

        let cas_dir = tempdir().unwrap();
        let endpoint = format!("local://{}", cas_dir.path().display());
        let config = crate::session::build_translator_config(&endpoint).unwrap();

        let args = StatsArgs {
            files: vec![src],
            recursive: false,
            output: None,
        };
        let metrics = run_stats(config, &args).await.unwrap();
        assert!(metrics.total_bytes > 0);
    }
}
