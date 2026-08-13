//! Disk cache for raw HTTP response bodies.
//!
//! Every response body is stored under `corpus/cache/<hash>.body`, where
//! `<hash>` is the SHA-256 hex digest of the full request URL (query
//! string included, so two different pages of the same feed get two
//! different cache entries). A manifest file
//! `corpus/cache/manifest.jsonl` records one JSON line per cached URL,
//! `{"hash": "...", "url": "..."}`, so a human can audit what a cache
//! file holds without decoding the hash.
//!
//! A cache hit never touches the network. This is the core correctness
//! requirement of this module: a second run against a warm cache must do
//! zero network requests.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::error::BuildError;

/// Number of retries after the first failed attempt, before giving up.
const MAX_RETRIES: u32 = 2;

/// Delay before the first retry. The delay doubles after each retry.
const RETRY_BACKOFF: Duration = Duration::from_millis(500);

/// A disk-backed cache for raw HTTP GET response bodies, keyed by URL hash.
pub struct HttpCache {
    cache_dir: PathBuf,
    agent: ureq::Agent,
    refresh: bool,
    offline: bool,
    network_requests: u64,
}

impl HttpCache {
    /// Open (creating if needed) a cache rooted at `cache_dir`.
    ///
    /// `refresh` forces every fetch to hit the network even on a cache
    /// hit. `offline` turns a cache miss into a clear error instead of a
    /// network call. Both should not be set at once; if they are,
    /// `refresh` wins on the cache-check but `offline` then rejects the
    /// network call, so the combination fails fast with a clear error
    /// rather than silently picking one.
    pub fn open(
        cache_dir: impl Into<PathBuf>,
        refresh: bool,
        offline: bool,
    ) -> Result<Self, BuildError> {
        let cache_dir = cache_dir.into();
        fs::create_dir_all(&cache_dir)?;
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(30))
            .build();
        Ok(Self {
            cache_dir,
            agent,
            refresh,
            offline,
            network_requests: 0,
        })
    }

    /// Number of real network GET requests made so far this run.
    pub fn network_requests(&self) -> u64 {
        self.network_requests
    }

    fn hash_url(url: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(url.as_bytes());
        let digest = hasher.finalize();
        let mut out = String::with_capacity(digest.len() * 2);
        for byte in digest {
            out.push_str(&format!("{byte:02x}"));
        }
        out
    }

    fn body_path(&self, hash: &str) -> PathBuf {
        self.cache_dir.join(format!("{hash}.body"))
    }

    fn manifest_path(&self) -> PathBuf {
        self.cache_dir.join("manifest.jsonl")
    }

    fn append_manifest(&self, hash: &str, url: &str) -> Result<(), BuildError> {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.manifest_path())?;
        let line = serde_json::json!({"hash": hash, "url": url});
        writeln!(file, "{line}")?;
        Ok(())
    }

    /// Fetch `url`, transparently using the disk cache.
    ///
    /// See [`HttpCache::open`] for how `refresh` and `offline` change this
    /// behaviour.
    pub fn fetch(&mut self, url: &str) -> Result<String, BuildError> {
        let hash = Self::hash_url(url);
        let path = self.body_path(&hash);

        if !self.refresh && path.exists() {
            return Ok(fs::read_to_string(&path)?);
        }

        if self.offline {
            return Err(BuildError::OfflineMiss(url.to_string()));
        }

        let body = self.fetch_with_retry(url)?;
        fs::write(&path, &body)?;
        self.append_manifest(&hash, url)?;
        Ok(body)
    }

    /// Fetch `url` from the network, retrying politely on a TRANSPORT
    /// failure only (DNS, connection refused, timeout, TLS).
    ///
    /// A non-2xx HTTP status is NOT retried: it is a deterministic
    /// response, not a transient failure (Open-Meteo's archive, for
    /// example, answers an out-of-range date with a well-formed JSON body
    /// on HTTP 400, not a network error). Its body is read and cached
    /// exactly like a 2xx body; the caller decides whether that status
    /// and body are acceptable. Retrying a deterministic 400 would just
    /// burn three requests to get the same answer.
    ///
    /// Tries up to `1 + MAX_RETRIES` times total on a transport failure,
    /// with a doubling backoff between attempts. Never panics: a failure
    /// after all attempts comes back as `Err`, for the caller to record.
    fn fetch_with_retry(&mut self, url: &str) -> Result<String, BuildError> {
        let mut attempt = 0;
        let mut backoff = RETRY_BACKOFF;
        loop {
            self.network_requests += 1;
            match self.agent.get(url).call() {
                Ok(response) | Err(ureq::Error::Status(_, response)) => {
                    return response.into_string().map_err(|e| {
                        BuildError::Http(format!("{url}: failed to read response body: {e}"))
                    });
                }
                Err(err @ ureq::Error::Transport(_)) => {
                    if attempt >= MAX_RETRIES {
                        return Err(BuildError::Http(format!(
                            "{url}: failed after {} attempts: {err}",
                            attempt + 1
                        )));
                    }
                    attempt += 1;
                    thread::sleep(backoff);
                    backoff *= 2;
                }
            }
        }
    }
}
