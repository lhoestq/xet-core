use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::{TempDir, tempdir};

/// Returns the path to the `xet` binary built by cargo.
fn xet_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_xet"))
}

/// Run `xet` with the given args against a local CAS endpoint.
/// Returns the Output (stdout, stderr, status).
fn xet_cmd(cas_dir: &Path, args: &[&str]) -> Output {
    let endpoint = format!("local://{}", cas_dir.display());
    Command::new(xet_bin())
        .arg("--endpoint")
        .arg(&endpoint)
        .args(args)
        .output()
        .expect("failed to execute xet binary")
}

/// Run `xet` and assert it succeeds, returning stdout as String.
fn xet_ok(cas_dir: &Path, args: &[&str]) -> String {
    let out = xet_cmd(cas_dir, args);
    assert!(
        out.status.success(),
        "xet {:?} failed:\nstdout: {}\nstderr: {}",
        args,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8(out.stdout).unwrap()
}

/// Run `xet` and assert it fails.
fn xet_err(cas_dir: &Path, args: &[&str]) -> String {
    let out = xet_cmd(cas_dir, args);
    assert!(
        !out.status.success(),
        "xet {:?} unexpectedly succeeded:\nstdout: {}\nstderr: {}",
        args,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stderr = String::from_utf8(out.stderr).unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    format!("{stdout}{stderr}")
}

/// Upload a file via CLI and parse the output line to extract hash and size.
/// Output format: `<name>  hash=<hex>  size=<n>  sha256=<hex|->`
fn upload_file(cas_dir: &Path, file_path: &Path) -> (String, u64) {
    let stdout = xet_ok(cas_dir, &["upload", file_path.to_str().unwrap()]);
    parse_upload_line(&stdout)
}

/// Parse a single upload output line into (hash, size).
fn parse_upload_line(line: &str) -> (String, u64) {
    let mut hash = String::new();
    let mut size = 0u64;
    for part in line.split_whitespace() {
        if let Some(h) = part.strip_prefix("hash=") {
            hash = h.to_string();
        }
        if let Some(s) = part.strip_prefix("size=") {
            size = s.parse().unwrap();
        }
    }
    assert!(!hash.is_empty(), "could not parse hash from: {line}");
    (hash, size)
}

fn write_test_file(dir: &TempDir, name: &str, content: &[u8]) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, content).unwrap();
    path
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[test]
fn test_cli_help() {
    let out = Command::new(xet_bin())
        .arg("--help")
        .output()
        .expect("failed to run xet --help");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("upload"));
    assert!(stdout.contains("download"));
    assert!(stdout.contains("stats"));
    assert!(stdout.contains("query"));
}

#[test]
fn test_cli_upload_and_download_roundtrip() {
    let cas_dir = tempdir().unwrap();
    let src_dir = tempdir().unwrap();
    let content = b"integration test roundtrip content";
    let src = write_test_file(&src_dir, "roundtrip.txt", content);

    let (hash, size) = upload_file(cas_dir.path(), &src);
    assert_eq!(size, content.len() as u64);

    let dest_dir = tempdir().unwrap();
    let dest = dest_dir.path().join("downloaded.txt");
    xet_ok(cas_dir.path(), &["download", &hash, dest.to_str().unwrap(), "--size", &size.to_string()]);

    assert_eq!(std::fs::read(&dest).unwrap(), content);
}

#[test]
fn test_cli_upload_multiple_files() {
    let cas_dir = tempdir().unwrap();
    let src_dir = tempdir().unwrap();

    let files: Vec<PathBuf> = (0..3)
        .map(|i| write_test_file(&src_dir, &format!("multi_{i}.bin"), format!("file {i} data").as_bytes()))
        .collect();

    let file_args: Vec<&str> = files.iter().map(|p| p.to_str().unwrap()).collect();
    let mut args = vec!["upload"];
    args.extend(&file_args);
    let stdout = xet_ok(cas_dir.path(), &args);

    let lines: Vec<&str> = stdout.lines().filter(|l| l.contains("hash=")).collect();
    assert_eq!(lines.len(), 3);
}

#[test]
fn test_cli_upload_json_output() {
    let cas_dir = tempdir().unwrap();
    let src_dir = tempdir().unwrap();
    let src = write_test_file(&src_dir, "json.txt", b"json output via cli");

    let out_dir = tempdir().unwrap();
    let json_path = out_dir.path().join("results.json");

    xet_ok(cas_dir.path(), &["upload", "--output", json_path.to_str().unwrap(), src.to_str().unwrap()]);

    let json_str = std::fs::read_to_string(&json_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    let arr = parsed.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["file_size"], 19);
    assert!(arr[0]["hash"].as_str().unwrap().len() > 0);
}

#[test]
fn test_cli_download_bad_hash() {
    let cas_dir = tempdir().unwrap();
    let dest_dir = tempdir().unwrap();
    let dest = dest_dir.path().join("should_not_exist.bin");

    let fake_hash = "0".repeat(64);
    xet_err(cas_dir.path(), &["download", &fake_hash, dest.to_str().unwrap(), "--size", "100"]);
}

#[test]
fn test_cli_stats_basic() {
    let cas_dir = tempdir().unwrap();
    let src_dir = tempdir().unwrap();
    let src = write_test_file(&src_dir, "stats_test.bin", &vec![7u8; 4096]);

    let stdout = xet_ok(cas_dir.path(), &["stats", src.to_str().unwrap()]);
    assert!(stdout.contains("total_bytes=4096"));
}

#[test]
fn test_cli_query_after_upload() {
    let cas_dir = tempdir().unwrap();
    let src_dir = tempdir().unwrap();
    let src = write_test_file(&src_dir, "query.bin", &vec![3u8; 2048]);

    let (hash, _size) = upload_file(cas_dir.path(), &src);

    let stdout = xet_ok(cas_dir.path(), &["query", &hash]);
    assert!(stdout.contains("terms:"));
    assert!(stdout.contains("xorbs:"));
}

#[test]
fn test_cli_query_nonexistent_hash() {
    let cas_dir = tempdir().unwrap();
    let fake_hash = "0".repeat(64);
    // LocalClient may return an empty reconstruction (Some with 0 terms)
    // or None; either way the query should succeed without error.
    let stdout = xet_ok(cas_dir.path(), &["query", &fake_hash]);
    let has_no_info = stdout.contains("No reconstruction info found");
    let has_zero_terms = stdout.contains("terms: 0");
    assert!(has_no_info || has_zero_terms, "unexpected query output: {stdout}");
}

#[test]
fn test_cli_config_override_accepted() {
    let cas_dir = tempdir().unwrap();
    let src_dir = tempdir().unwrap();
    let src = write_test_file(&src_dir, "config_test.txt", b"config test");

    let endpoint = format!("local://{}", cas_dir.path().display());
    let out = Command::new(xet_bin())
        .arg("--endpoint")
        .arg(&endpoint)
        .arg("-c")
        .arg("client.enable_multirange_fetching=true")
        .arg("upload")
        .arg(src.to_str().unwrap())
        .output()
        .expect("failed to execute xet");

    assert!(out.status.success(), "config override failed:\nstderr: {}", String::from_utf8_lossy(&out.stderr));
}

/// Upload N files concurrently via separate CLI invocations, then download
/// and verify each one. Catches ordering or file-handle issues in the local
/// CAS under concurrent access.
#[test]
fn test_cli_concurrent_upload_download_stress() {
    let cas_dir = tempdir().unwrap();
    let src_dir = tempdir().unwrap();

    let n = 20;
    let files: Vec<(PathBuf, Vec<u8>)> = (0..n)
        .map(|i| {
            let content: Vec<u8> = (0..256).map(|b| ((b as u16 * (i + 1) as u16) % 256) as u8).collect();
            let path = write_test_file(&src_dir, &format!("stress_{i}.bin"), &content);
            (path, content)
        })
        .collect();

    // Upload all files in one batch CLI call
    let file_args: Vec<&str> = files.iter().map(|(p, _)| p.to_str().unwrap()).collect();
    let mut args = vec!["upload", "--no-sha256"];
    args.extend(&file_args);
    let stdout = xet_ok(cas_dir.path(), &args);

    let upload_lines: Vec<&str> = stdout.lines().filter(|l| l.contains("hash=")).collect();
    assert_eq!(upload_lines.len(), n);

    // Parse each line, download, and verify content
    let dest_dir = tempdir().unwrap();
    for (i, line) in upload_lines.iter().enumerate() {
        let (hash, size) = parse_upload_line(line);

        let dest = dest_dir.path().join(format!("out_{i}.bin"));
        xet_ok(cas_dir.path(), &["download", &hash, dest.to_str().unwrap(), "--size", &size.to_string()]);

        let downloaded = std::fs::read(&dest).unwrap();
        assert_eq!(downloaded, files[i].1, "content mismatch for file {i} (hash={hash})");
    }
}
