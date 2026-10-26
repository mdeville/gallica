//! The HTTP client shared by every API.
//!
//! [`Client`] wraps a `reqwest` client with the politeness features a bulk
//! downloader needs: a minimum interval between requests, bounded download
//! concurrency, and automatic retries with exponential backoff on transient
//! failures (HTTP 429/5xx, timeouts, connection resets).

use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue, RETRY_AFTER, USER_AGENT};
use reqwest::{Response, StatusCode};
use tokio::sync::{Mutex, Semaphore};
use url::Url;

use crate::ark::GALLICA_BASE_URL;
use crate::error::{Error, Result};
use crate::quota::{QuotaClass, QuotaSet, Quotas};

/// Default `User-Agent` sent to Gallica.
pub const DEFAULT_USER_AGENT: &str = concat!(
    "gallica-rs/",
    env!("CARGO_PKG_VERSION"),
    " (Rust client for the Gallica APIs)"
);

/// Retry policy for transient failures.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Maximum number of attempts (1 = no retry).
    pub max_attempts: u32,
    /// Delay before the first retry; doubled after each failure.
    pub initial_backoff: Duration,
    /// Upper bound for the backoff delay.
    pub max_backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        RetryPolicy {
            max_attempts: 5,
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(60),
        }
    }
}

impl RetryPolicy {
    /// A policy that never retries.
    pub fn none() -> Self {
        RetryPolicy {
            max_attempts: 1,
            ..Default::default()
        }
    }

    fn backoff_for(&self, attempt: u32) -> Duration {
        let factor = 2u32.saturating_pow(attempt.saturating_sub(1));
        self.initial_backoff
            .saturating_mul(factor)
            .min(self.max_backoff)
    }
}

/// Builder for [`Client`].
#[derive(Debug, Clone)]
pub struct ClientBuilder {
    base_url: Url,
    user_agent: String,
    timeout: Duration,
    connect_timeout: Duration,
    min_request_interval: Duration,
    max_concurrent_downloads: usize,
    retry: RetryPolicy,
    quotas: Quotas,
}

impl Default for ClientBuilder {
    fn default() -> Self {
        ClientBuilder {
            base_url: Url::parse(GALLICA_BASE_URL).expect("constant URL"),
            user_agent: DEFAULT_USER_AGENT.to_string(),
            timeout: Duration::from_secs(120),
            connect_timeout: Duration::from_secs(20),
            min_request_interval: Duration::from_millis(250),
            max_concurrent_downloads: 2,
            retry: RetryPolicy::default(),
            quotas: Quotas::default(),
        }
    }
}

impl ClientBuilder {
    /// Creates a builder with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Overrides the Gallica base URL (useful for tests and mirrors).
    pub fn base_url(mut self, url: impl AsRef<str>) -> Result<Self> {
        let mut url = Url::parse(url.as_ref())?;
        if !url.path().ends_with('/') {
            let p = format!("{}/", url.path());
            url.set_path(&p);
        }
        self.base_url = url;
        Ok(self)
    }

    /// Sets the `User-Agent` header. Gallica asks that automated clients
    /// identify themselves; a contact address in the string is a good idea.
    pub fn user_agent(mut self, ua: impl Into<String>) -> Self {
        self.user_agent = ua.into();
        self
    }

    /// Total timeout for a single request (default 120 s).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Minimum delay between two requests, across all tasks sharing the
    /// client (default 250 ms, i.e. at most 4 requests per second).
    pub fn min_request_interval(mut self, interval: Duration) -> Self {
        self.min_request_interval = interval;
        self
    }

    /// Maximum number of file downloads in flight at once (default 2).
    pub fn max_concurrent_downloads(mut self, n: usize) -> Self {
        self.max_concurrent_downloads = n.max(1);
        self
    }

    /// Retry policy for transient failures.
    pub fn retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// Per-endpoint quotas (default: the limits published by the BnF, see
    /// [`Quotas::bnf`]).
    pub fn quotas(mut self, quotas: Quotas) -> Self {
        self.quotas = quotas;
        self
    }

    /// Builds the client.
    pub fn build(self) -> Result<Client> {
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_str(&self.user_agent)
                .map_err(|e| Error::InvalidArgument(format!("user agent: {e}")))?,
        );
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(self.timeout)
            .connect_timeout(self.connect_timeout)
            .gzip(true)
            .build()?;
        Ok(Client {
            inner: Arc::new(Inner {
                http,
                base_url: self.base_url,
                retry: self.retry,
                throttle: Throttle::new(self.min_request_interval),
                download_permits: Semaphore::new(self.max_concurrent_downloads),
                quotas: QuotaSet::new(&self.quotas),
            }),
        })
    }
}

/// A Gallica API client. Cheap to clone; clones share connections, the
/// request throttle and the download concurrency limit.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    base_url: Url,
    retry: RetryPolicy,
    throttle: Throttle,
    download_permits: Semaphore,
    quotas: QuotaSet,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("base_url", &self.inner.base_url.as_str())
            .finish_non_exhaustive()
    }
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    /// Creates a client with default settings.
    pub fn new() -> Client {
        ClientBuilder::default()
            .build()
            .expect("default client configuration is valid")
    }

    /// Returns a builder to customise the client.
    pub fn builder() -> ClientBuilder {
        ClientBuilder::default()
    }

    /// The base URL requests are made against.
    pub fn base_url(&self) -> &Url {
        &self.inner.base_url
    }

    /// Builds a URL relative to the base URL.
    ///
    /// Gallica paths start with `ark:/`, which `Url::join` would read as a
    /// scheme, so the path is appended by hand.
    pub fn url(&self, path: &str) -> Result<Url> {
        let mut url = self.inner.base_url.clone();
        let joined = format!("{}{}", url.path(), path.trim_start_matches('/'));
        url.set_path(&joined);
        Ok(url)
    }

    /// Builds a `services/<name>` URL with query parameters.
    pub fn service_url(&self, service: &str, params: &[(&str, &str)]) -> Result<Url> {
        let mut url = self.url(&format!("services/{service}"))?;
        {
            let mut q = url.query_pairs_mut();
            for (k, v) in params {
                q.append_pair(k, v);
            }
        }
        Ok(url)
    }

    /// Performs a GET request with throttling and retries, returning the
    /// response once a success status has been received.
    pub async fn get(&self, url: Url) -> Result<Response> {
        self.request(url, QuotaClass::Default, None).await
    }

    /// Like [`Client::get`], also counting the request against the quota of
    /// the given class.
    pub async fn get_classed(&self, url: Url, class: QuotaClass) -> Result<Response> {
        self.request(url, class, None).await
    }

    pub(crate) async fn request(
        &self,
        url: Url,
        class: QuotaClass,
        accept: Option<&str>,
    ) -> Result<Response> {
        let policy = &self.inner.retry;
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            self.inner.quotas.acquire(class).await;
            self.inner.throttle.wait().await;
            tracing::debug!(%url, attempt, "GET");
            let mut req = self.inner.http.get(url.clone());
            if let Some(a) = accept {
                req = req.header(ACCEPT, a);
            }
            let outcome = match req.send().await {
                Ok(resp) => check_status(resp).await,
                Err(e) => Err((Error::Transport(e), None)),
            };
            match outcome {
                Ok(resp) => return Ok(resp),
                Err((err, retry_after)) => {
                    if !err.is_retryable() || attempt >= policy.max_attempts {
                        if attempt > 1 {
                            return Err(Error::RetriesExhausted {
                                url: url.to_string(),
                                attempts: attempt,
                                last: Box::new(err),
                            });
                        }
                        return Err(err);
                    }
                    let delay = retry_after.unwrap_or_else(|| policy.backoff_for(attempt));
                    tracing::warn!(%url, attempt, ?delay, error = %err, "retrying");
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    /// GET and return the body as text.
    pub async fn get_text(&self, url: Url) -> Result<String> {
        Ok(self.get(url).await?.text().await?)
    }

    /// GET and return the body as bytes.
    pub async fn get_bytes(&self, url: Url) -> Result<Bytes> {
        Ok(self.get(url).await?.bytes().await?)
    }

    /// GET and deserialise the body as JSON.
    pub async fn get_json<T: serde::de::DeserializeOwned>(&self, url: Url) -> Result<T> {
        let text = self
            .request(
                url,
                QuotaClass::Default,
                Some("application/json, application/ld+json"),
            )
            .await?
            .text()
            .await?;
        Ok(serde_json::from_str(&text)?)
    }

    /// Reads a whole body into memory, paced by the bandwidth cap.
    pub(crate) async fn read_body(&self, resp: Response) -> Result<Bytes> {
        use futures::StreamExt;
        let mut out = bytes::BytesMut::with_capacity(resp.content_length().unwrap_or(0) as usize);
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if let Some(b) = self.inner.quotas.bandwidth() {
                b.consume(chunk.len()).await;
            }
            out.extend_from_slice(&chunk);
        }
        Ok(out.freeze())
    }

    pub(crate) fn bandwidth_limiter(&self) -> Option<&crate::quota::BandwidthLimiter> {
        self.inner.quotas.bandwidth()
    }

    /// Acquires a download slot; dropped when the download finishes.
    pub(crate) async fn download_permit(&self) -> tokio::sync::SemaphorePermit<'_> {
        self.inner
            .download_permits
            .acquire()
            .await
            .expect("semaphore is never closed")
    }
}

/// Turns a response into an error unless the status is a success.
async fn check_status(resp: Response) -> std::result::Result<Response, (Error, Option<Duration>)> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let url = resp.url().to_string();
    let retry_after = resp
        .headers()
        .get(RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
    let is_text = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.starts_with("text/") || ct.contains("json") || ct.contains("xml"))
        .unwrap_or(false);
    let body = if is_text {
        resp.text().await.ok().map(|t| snippet(&t))
    } else {
        None
    };
    let err = if status == StatusCode::NOT_FOUND {
        Error::NotFound(url)
    } else {
        Error::Http {
            url,
            status: status.as_u16(),
            body,
        }
    };
    Err((err, retry_after))
}

fn snippet(text: &str) -> String {
    // Strip tags so HTML error pages stay readable.
    let mut out = String::new();
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let collapsed: String = out.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(200).collect()
}

/// Enforces a minimum interval between requests across tasks.
struct Throttle {
    interval: Duration,
    next_allowed: Mutex<Instant>,
}

impl Throttle {
    fn new(interval: Duration) -> Self {
        Throttle {
            interval,
            next_allowed: Mutex::new(Instant::now()),
        }
    }

    async fn wait(&self) {
        if self.interval.is_zero() {
            return;
        }
        let mut next = self.next_allowed.lock().await;
        let now = Instant::now();
        if *next > now {
            tokio::time::sleep(*next - now).await;
        }
        *next = Instant::now() + self.interval;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ark_paths_stay_relative() {
        let c = Client::new();
        assert_eq!(
            c.url("ark:/12148/bpt6k5619759j.pdf").unwrap().as_str(),
            "https://gallica.bnf.fr/ark:/12148/bpt6k5619759j.pdf"
        );
        assert_eq!(
            c.url("/SRU").unwrap().as_str(),
            "https://gallica.bnf.fr/SRU"
        );
        let c = Client::builder()
            .base_url("http://127.0.0.1:1234/prefix")
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(
            c.url("ark:/12148/x/f1n2.texteBrut").unwrap().as_str(),
            "http://127.0.0.1:1234/prefix/ark:/12148/x/f1n2.texteBrut"
        );
        assert_eq!(
            c.service_url("Issues", &[("ark", "ark:/12148/cb1/date")])
                .unwrap()
                .as_str(),
            "http://127.0.0.1:1234/prefix/services/Issues?ark=ark%3A%2F12148%2Fcb1%2Fdate"
        );
    }
}
