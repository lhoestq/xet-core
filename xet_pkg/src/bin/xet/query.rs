use std::sync::Arc;

use anyhow::Result;
use clap::Args;
use xet_client::cas_client::Client;
use xet_client::cas_types::{FileRange, QueryReconstructionResponseV2};
use xet_core_structures::merklehash::MerkleHash;

use super::Cli;

#[derive(Args)]
pub struct QueryArgs {
    /// Hex-encoded MerkleHash of the file.
    pub hash: String,

    /// Optional byte range "start-end" (exclusive end), e.g. "0-1048576".
    /// Uses CAS FileRange format (start inclusive, end exclusive).
    pub range: Option<String>,
}

pub async fn run(cli: &Cli, args: &QueryArgs) -> Result<()> {
    let client = super::session::build_cas_client(&cli.resolved_endpoint(), cli.resolved_token()).await?;
    let response = run_query(client, args).await?;
    match response {
        Some(r) => {
            println!("terms: {}", r.terms.len());
            println!("xorbs: {}", r.xorbs.len());
            let total_bytes: u64 = r.terms.iter().map(|t| t.unpacked_length as u64).sum();
            println!("total_uncompressed_bytes: {}", total_bytes);
            for (i, term) in r.terms.iter().enumerate() {
                println!("  term[{}]: xorb={} unpacked_len={}", i, term.hash, term.unpacked_length);
            }
        },
        None => println!("No reconstruction info found for that hash."),
    }
    Ok(())
}

fn parse_range(s: &str) -> Result<FileRange> {
    s.parse::<FileRange>()
        .map_err(|e| anyhow::anyhow!("range must be 'start-end', got: {s}: {e}"))
}

pub async fn run_query(client: Arc<dyn Client>, args: &QueryArgs) -> Result<Option<QueryReconstructionResponseV2>> {
    let hash = MerkleHash::from_hex(&args.hash).map_err(|e| anyhow::anyhow!("invalid hash '{}': {e}", args.hash))?;
    let range: Option<FileRange> = args.range.as_deref().map(parse_range).transpose()?;
    client.get_reconstruction(&hash, range).await.map_err(anyhow::Error::from)
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::session::{build_cas_client, build_xet_session};
    use crate::upload::{UploadArgs, run_upload};

    #[tokio::test(flavor = "multi_thread")]
    async fn test_query_after_upload() {
        let cas_dir = tempdir().unwrap();
        let endpoint = format!("local://{}", cas_dir.path().display());

        // Upload a file.
        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("query_test.bin");
        std::fs::write(&src, vec![1u8; 4096]).unwrap();

        let config = xet_runtime::config::XetConfig::new();
        let session = build_xet_session(&endpoint, None, config).await.unwrap();
        let results = run_upload(
            session,
            &UploadArgs {
                files: vec![src],
                sha256: false,
                output: None,
            },
        )
        .await
        .unwrap();
        let hash_str = &results[0].hash;

        // Query reconstruction info.
        let client = build_cas_client(&endpoint, None).await.unwrap();
        let args = QueryArgs {
            hash: hash_str.clone(),
            range: None,
        };
        let response = run_query(client, &args).await.unwrap();
        assert!(response.is_some(), "expected Some reconstruction response");
    }
}
