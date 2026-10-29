//! Resumable downloads over `https://` (and `file://`, for local testing).
//!
//! This module is the only place in Second Dawn that opens network connections.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::error::{ModelError, Result};

const CHUNK: usize = 64 * 1024;

/// Download `url` into `dest`, appending to a partial file if one exists.
///
/// `on_progress(done, total)` gets bytes written so far and the full size (0 if unknown).
/// Setting `cancel` stops the transfer with [`ModelError::Cancelled`] and keeps the partial file.
pub fn download(
    url: &str,
    dest: &Path,
    proxy: Option<&str>,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let existing = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
    if let Some(path) = file_url_to_path(url) {
        copy_local(&path, dest, existing, cancel, on_progress)
    } else if url.starts_with("https://") || url.starts_with("http://") {
        fetch_http(url, dest, existing, proxy, cancel, on_progress)
    } else {
        Err(ModelError::Network(format!("unsupported url: {url}")))
    }
}

/// `file:///C:/a%20b/c.zip` -> `C:/a b/c.zip`
pub fn file_url_to_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let decoded = percent_decode(rest);
    // `file:///C:/x` leaves "/C:/x": drop the slash before a drive letter.
    let bytes = decoded.as_bytes();
    let path = if bytes.len() > 2 && bytes[0] == b'/' && bytes[2] == b':' { &decoded[1..] } else { &decoded[..] };
    Some(PathBuf::from(path))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn open_for_append(dest: &Path, resume_from: u64) -> Result<File> {
    let mut file = OpenOptions::new().create(true).write(true).open(dest)?;
    file.set_len(resume_from)?;
    file.seek(SeekFrom::Start(resume_from))?;
    Ok(file)
}

fn pump(
    reader: &mut dyn Read,
    file: &mut File,
    mut done: u64,
    total: u64,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    let mut buf = vec![0u8; CHUNK];
    on_progress(done, total);
    loop {
        if cancel.load(Ordering::Relaxed) {
            file.flush()?;
            return Err(ModelError::Cancelled);
        }
        let n = reader.read(&mut buf).map_err(|e| ModelError::Network(e.to_string()))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        done += n as u64;
        on_progress(done, total);
    }
    file.flush()?;
    Ok(())
}

fn copy_local(src: &Path, dest: &Path, existing: u64, cancel: &AtomicBool, on_progress: &mut dyn FnMut(u64, u64)) -> Result<()> {
    let mut input = File::open(src).map_err(|e| ModelError::Network(format!("{}: {e}", src.display())))?;
    let total = input.metadata()?.len();
    let resume = if existing <= total { existing } else { 0 };
    input.seek(SeekFrom::Start(resume))?;
    let mut file = open_for_append(dest, resume)?;
    pump(&mut input, &mut file, resume, total, cancel, on_progress)
}

fn fetch_http(
    url: &str,
    dest: &Path,
    existing: u64,
    proxy: Option<&str>,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    let mut builder = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(20)).timeout_read(Duration::from_secs(60));
    if let Some(proxy) = proxy.filter(|p| !p.trim().is_empty()) {
        builder = builder.proxy(ureq::Proxy::new(proxy).map_err(|e| ModelError::Network(format!("bad proxy: {e}")))?);
    }
    let agent = builder.build();

    let mut request = agent.get(url);
    if existing > 0 {
        request = request.set("Range", &format!("bytes={existing}-"));
    }
    let response = match request.call() {
        Ok(r) => r,
        // The partial file is already complete (or stale): start over once.
        Err(ureq::Error::Status(416, _)) if existing > 0 => {
            std::fs::remove_file(dest)?;
            return fetch_http(url, dest, 0, proxy, cancel, on_progress);
        }
        Err(e) => return Err(ModelError::Network(e.to_string())),
    };

    let length: u64 = response.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(0);
    // 206 = the server honored the range; 200 = it sent everything from the start.
    let (resume, total) = if response.status() == 206 { (existing, existing + length) } else { (0, length) };
    let mut file = open_for_append(dest, resume)?;
    let mut reader = response.into_reader();
    pump(&mut reader, &mut file, resume, total, cancel, on_progress)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_file_urls() {
        assert_eq!(file_url_to_path("file:///C:/models/a%20b/m.zip").unwrap(), PathBuf::from("C:/models/a b/m.zip"));
        assert_eq!(file_url_to_path("file:///tmp/m.zip").unwrap(), PathBuf::from("/tmp/m.zip"));
        assert!(file_url_to_path("https://example.org/m.zip").is_none());
    }

    #[test]
    fn rejects_unknown_schemes() {
        let cancel = AtomicBool::new(false);
        let err = download("ftp://x/y", &std::env::temp_dir().join("dawn-never"), None, &cancel, &mut |_, _| {}).unwrap_err();
        assert_eq!(err.code(), "network");
    }
}
