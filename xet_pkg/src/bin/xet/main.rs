mod download;
mod query;
mod session;
mod stats;
mod upload;

use anyhow::Result;
use clap::{Parser, Subcommand};
use download::DownloadArgs;
use query::QueryArgs;
use stats::StatsArgs;
use upload::UploadArgs;

const DEFAULT_HF_ENDPOINT: &str = "https://huggingface.co";

/// Xet CAS developer tool for uploading, downloading, and inspecting files.
#[derive(Parser)]
#[command(name = "xet", version)]
pub struct Cli {
    /// CAS endpoint URL or local path (env: HF_ENDPOINT).
    ///
    /// Accepts https:// URLs for remote servers, absolute filesystem
    /// paths (auto-prefixed with local://), or explicit local:// URLs.
    /// Defaults to HF_ENDPOINT env var, then https://huggingface.co.
    #[arg(long, global = true)]
    pub endpoint: Option<String>,

    /// Auth token for remote endpoints (env: HF_TOKEN).
    ///
    /// Falls back to HF_TOKEN env var. Not needed for local endpoints.
    #[arg(long, global = true)]
    pub token: Option<String>,

    /// Override a xet_config value. May be repeated.
    #[arg(short = 'c', long = "config", global = true, value_name = "KEY=VALUE")]
    pub config_overrides: Vec<String>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Upload one or more files to the CAS endpoint.
    Upload(UploadArgs),
    /// Download a file by its xet hash.
    Download(DownloadArgs),
    /// Dry-run dedup and compression analysis (no upload).
    Stats(StatsArgs),
    /// Show reconstruction metadata for a file hash.
    Query(QueryArgs),
}

impl Cli {
    /// Resolve the endpoint to a canonical form:
    /// - absolute paths are prefixed with "local://"
    /// - local:// URLs are returned as-is
    /// - https:// URLs are returned as-is
    /// - None falls back to HF_ENDPOINT env var or the HF default
    pub fn resolved_endpoint(&self) -> String {
        let raw = self
            .endpoint
            .clone()
            .unwrap_or_else(|| std::env::var("HF_ENDPOINT").unwrap_or_else(|_| DEFAULT_HF_ENDPOINT.to_owned()));
        normalize_endpoint(&raw)
    }

    /// Resolve the token: --token flag, then HF_TOKEN env var, then None.
    pub fn resolved_token(&self) -> Option<String> {
        self.token
            .clone()
            .or_else(|| std::env::var("HF_TOKEN").ok())
            .filter(|t| !t.is_empty())
    }
}

/// Normalizes an endpoint string: absolute filesystem paths get a `local://` prefix.
pub fn normalize_endpoint(raw: &str) -> String {
    if raw.contains("://") {
        raw.to_owned()
    } else if raw.starts_with('/') {
        format!("local://{raw}")
    } else {
        raw.to_owned()
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Apply -c config overrides before building the runtime.
    // The config is both installed globally (via XetRuntime) for xet_config()
    // callers (stats, query) AND forwarded explicitly to XetSessionBuilder
    // (upload, download) so that overrides take effect in both paths.
    let mut config = xet_runtime::config::XetConfig::new();
    for kv in &cli.config_overrides {
        let (key, val) = kv
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("--config must be KEY=VALUE, got: {kv}"))?;
        config = config.with_config(key, val)?;
    }

    // Install the config globally and spin up the thread pool.
    let runtime = xet_runtime::core::XetRuntime::new_with_config(config.clone())?;

    // Wrap cli in Arc so both the outer borrow (for resolved_endpoint/token)
    // and the inner move into the 'static async block can coexist.
    let cli = std::sync::Arc::new(cli);

    runtime.external_run_async_task({
        let cli = cli.clone();
        async move {
            match cli.command {
                Commands::Upload(ref args) => upload::run(&cli, config.clone(), args).await,
                Commands::Download(ref args) => download::run(&cli, config.clone(), args).await,
                Commands::Stats(ref args) => stats::run(&cli, args).await,
                Commands::Query(ref args) => query::run(&cli, args).await,
            }
        }
    })?
}

#[cfg(test)]
mod tests {
    use super::normalize_endpoint;

    #[test]
    fn test_normalize_endpoint() {
        let cases = [
            ("/tmp/cas", "local:///tmp/cas"),
            ("/", "local:///"),
            ("local:///tmp/cas", "local:///tmp/cas"),
            ("https://cas.example.com", "https://cas.example.com"),
            ("http://localhost:8080", "http://localhost:8080"),
            ("relative/path", "relative/path"),
        ];
        for (input, expected) in cases {
            assert_eq!(normalize_endpoint(input), expected, "input: {input}");
        }
    }
}
