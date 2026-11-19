//! Where image bytes come from: a file on disk, bytes in memory, or a file on a web server
//! read with HTTP range requests (cloud-optimised GeoTIFFs on AWS).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use crate::{err, Error, Result};

/// Random access to an image's bytes.
pub trait Source: Send + Sync {
    /// `len` bytes from `off` (fewer only at the end of the file).
    fn read_at(&self, off: u64, len: usize) -> Result<Vec<u8>>;
}

pub struct FileSource(Mutex<File>);

impl FileSource {
    pub fn open(path: &Path) -> Result<FileSource> {
        Ok(FileSource(Mutex::new(File::open(path).map_err(|e| Error(format!("{}: {e}", path.display())))?)))
    }
}

impl Source for FileSource {
    fn read_at(&self, off: u64, len: usize) -> Result<Vec<u8>> {
        let mut f = self.0.lock().unwrap_or_else(|e| e.into_inner());
        f.seek(SeekFrom::Start(off))?;
        let mut buf = Vec::with_capacity(len);
        f.by_ref().take(len as u64).read_to_end(&mut buf)?;
        Ok(buf)
    }
}

pub struct MemSource(pub Vec<u8>);

impl Source for MemSource {
    fn read_at(&self, off: u64, len: usize) -> Result<Vec<u8>> {
        let s = (off as usize).min(self.0.len());
        let e = s.saturating_add(len).min(self.0.len());
        Ok(self.0[s..e].to_vec())
    }
}

pub const USER_AGENT: &str = concat!("TweedPlot/", env!("CARGO_PKG_VERSION"), " (+https://tweedsmarine.nz)");

/// A blocking HTTP client for the image services.
pub fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| Error(format!("HTTP client: {e}")))
}

/// A file on a web server, read with range requests. The first bytes (where a
/// cloud-optimised GeoTIFF keeps its directories) are fetched once and kept.
pub struct HttpSource {
    url: String,
    client: reqwest::blocking::Client,
    head: Mutex<Vec<u8>>,
    /// Requests made (for tests and the activity line).
    pub requests: AtomicUsize,
}

const HEAD: usize = 64 * 1024;

impl HttpSource {
    pub fn new(url: &str) -> Result<HttpSource> {
        // plain http only for a server on this computer (tests)
        if !url.starts_with("https://") && !url.starts_with("http://127.0.0.1:") {
            return err(format!("not an https address: {url}"));
        }
        Ok(HttpSource { url: url.to_string(), client: client()?, head: Mutex::new(Vec::new()), requests: AtomicUsize::new(0) })
    }

    fn get(&self, off: u64, len: usize) -> Result<Vec<u8>> {
        if len == 0 {
            return Ok(Vec::new());
        }
        self.requests.fetch_add(1, Ordering::Relaxed);
        let range = format!("bytes={}-{}", off, off + len as u64 - 1);
        let mut last = String::new();
        // one retry: S3 occasionally drops a connection
        for _ in 0..2 {
            match self.client.get(&self.url).header("Range", &range).send() {
                Ok(r) => {
                    let status = r.status();
                    if status.as_u16() == 416 {
                        return Ok(Vec::new());
                    }
                    if !status.is_success() {
                        return err(format!("{} for {}", status, self.url));
                    }
                    let whole = status.as_u16() == 200;
                    let body = r.bytes().map_err(|e| Error(format!("reading {}: {e}", self.url)))?;
                    // a server that ignores Range sends the whole file
                    if whole {
                        let s = (off as usize).min(body.len());
                        let e = s.saturating_add(len).min(body.len());
                        return Ok(body[s..e].to_vec());
                    }
                    return Ok(body.to_vec());
                }
                Err(e) => last = e.to_string(),
            }
        }
        err(format!("could not reach {}: {last}", host_of(&self.url)))
    }
}

impl Source for HttpSource {
    fn read_at(&self, off: u64, len: usize) -> Result<Vec<u8>> {
        let mut head = self.head.lock().unwrap_or_else(|e| e.into_inner());
        if head.is_empty() {
            *head = self.get(0, HEAD)?;
        }
        if off as usize + len <= head.len() || (head.len() < HEAD && off as usize <= head.len()) {
            let s = off as usize;
            let e = (s + len).min(head.len());
            return Ok(head[s.min(e)..e].to_vec());
        }
        drop(head);
        self.get(off, len)
    }
}

pub fn host_of(url: &str) -> &str {
    url.split("://").nth(1).unwrap_or(url).split('/').next().unwrap_or(url)
}
