//! Usage quotas published by the BnF for the Gallica APIs.
//!
//! While its API manager is being deployed, the BnF enforces per-minute
//! limits on the heavier endpoints and a download bandwidth cap, answering
//! HTTP 429 beyond them (see <https://api.bnf.fr/fr/node/232>). The client
//! applies these limits itself so that a well-behaved bulk download never
//! trips them.

use std::collections::VecDeque;
use std::time::Duration;

use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::time::Instant;

/// Called when a request has to wait for a quota: `(class, wait)`.
pub type QuotaWaitHook = Arc<dyn Fn(QuotaClass, Duration) + Send + Sync>;

/// The family a request belongs to for quota purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum QuotaClass {
    /// Metadata and small answers: SRU, Document services, IIIF `info.json`
    /// and manifests, small IIIF images. Only the global request interval
    /// applies.
    Default,
    /// IIIF images at full size, or wider/taller than 1000 px.
    FullResImage,
    /// Pre-computed `highres` preview images.
    HighResPreview,
    /// PDF exports.
    Pdf,
    /// Plain text (`texteBrut`) exports.
    Text,
}

/// Per-minute request limits and bandwidth cap.
///
/// The defaults are the figures published by the BnF. Use
/// [`Quotas::unlimited`] to disable them (for a mock server, or if the BnF
/// grants you a higher quota).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Quotas {
    /// IIIF images in `full/full` mode or larger than 1000 px (BnF: 5).
    pub full_res_images_per_minute: Option<u32>,
    /// `highres` preview images (BnF: 50).
    pub highres_previews_per_minute: Option<u32>,
    /// PDF exports (BnF: 4).
    pub pdfs_per_minute: Option<u32>,
    /// `texteBrut` exports (BnF: 5).
    pub texts_per_minute: Option<u32>,
    /// Download bandwidth in bytes per second (BnF: 832 KB/s).
    pub bandwidth_bytes_per_second: Option<u64>,
}

impl Quotas {
    /// The limits published by the BnF.
    pub fn bnf() -> Self {
        Quotas {
            full_res_images_per_minute: Some(5),
            highres_previews_per_minute: Some(50),
            pdfs_per_minute: Some(4),
            texts_per_minute: Some(5),
            bandwidth_bytes_per_second: Some(832 * 1024),
        }
    }

    /// No limits at all.
    pub fn unlimited() -> Self {
        Quotas {
            full_res_images_per_minute: None,
            highres_previews_per_minute: None,
            pdfs_per_minute: None,
            texts_per_minute: None,
            bandwidth_bytes_per_second: None,
        }
    }
}

impl Quotas {
    /// The per-minute limit that applies to a class, if any.
    pub fn per_minute(&self, class: QuotaClass) -> Option<u32> {
        match class {
            QuotaClass::Default => None,
            QuotaClass::FullResImage => self.full_res_images_per_minute,
            QuotaClass::HighResPreview => self.highres_previews_per_minute,
            QuotaClass::Pdf => self.pdfs_per_minute,
            QuotaClass::Text => self.texts_per_minute,
        }
    }

    /// The least time `count` requests of a class can take under the
    /// per-minute limit, ignoring transfer time. `None` when unlimited.
    pub fn minimum_duration(&self, class: QuotaClass, count: u64) -> Option<Duration> {
        let limit = self.per_minute(class)?.max(1) as u64;
        if count <= limit {
            return Some(Duration::ZERO);
        }
        // Each full window of `limit` requests after the first costs one minute.
        let windows = count.div_ceil(limit) - 1;
        Some(Duration::from_secs(60 * windows))
    }
}

impl Default for Quotas {
    fn default() -> Self {
        Self::bnf()
    }
}

/// Safety margin added to the one minute window, so that a sliding window
/// on our side never exceeds a fixed window on the server's side.
const WINDOW: Duration = Duration::from_secs(61);

/// Allows at most `limit` acquisitions per sliding [`WINDOW`].
pub(crate) struct RateLimiter {
    limit: u32,
    history: Mutex<VecDeque<Instant>>,
}

impl RateLimiter {
    pub(crate) fn new(limit: u32) -> Self {
        RateLimiter {
            limit: limit.max(1),
            history: Mutex::new(VecDeque::new()),
        }
    }

    /// Waits until a request may be made, then records it.
    pub(crate) async fn acquire(&self, class: QuotaClass, hook: Option<&QuotaWaitHook>) {
        loop {
            let wait = {
                let mut history = self.history.lock().await;
                let now = Instant::now();
                while history
                    .front()
                    .is_some_and(|t| now.duration_since(*t) >= WINDOW)
                {
                    history.pop_front();
                }
                if (history.len() as u32) < self.limit {
                    history.push_back(now);
                    return;
                }
                WINDOW - now.duration_since(*history.front().expect("non-empty"))
            };
            tracing::debug!(?class, ?wait, "quota reached, waiting");
            if let Some(h) = hook {
                h(class, wait);
            }
            tokio::time::sleep(wait).await;
        }
    }
}

/// A token bucket limiting bytes per second, shared by all downloads.
pub(crate) struct BandwidthLimiter {
    rate: f64,
    state: Mutex<(Instant, f64)>,
}

impl BandwidthLimiter {
    pub(crate) fn new(bytes_per_second: u64) -> Self {
        let rate = bytes_per_second.max(1) as f64;
        BandwidthLimiter {
            rate,
            state: Mutex::new((Instant::now(), rate)),
        }
    }

    /// Accounts for `bytes` just received and sleeps if we are ahead of the
    /// allowed rate.
    pub(crate) async fn consume(&self, bytes: usize) {
        let debt = {
            let mut state = self.state.lock().await;
            let (last, tokens) = *state;
            let now = Instant::now();
            let refilled =
                (tokens + now.duration_since(last).as_secs_f64() * self.rate).min(self.rate);
            let remaining = refilled - bytes as f64;
            *state = (now, remaining);
            remaining
        };
        if debt < 0.0 {
            tokio::time::sleep(Duration::from_secs_f64(-debt / self.rate)).await;
        }
    }
}

/// The limiters built from a [`Quotas`].
pub(crate) struct QuotaSet {
    full_res: Option<RateLimiter>,
    highres: Option<RateLimiter>,
    pdf: Option<RateLimiter>,
    text: Option<RateLimiter>,
    bandwidth: Option<BandwidthLimiter>,
    hook: Option<QuotaWaitHook>,
}

impl QuotaSet {
    pub(crate) fn new(q: &Quotas, hook: Option<QuotaWaitHook>) -> Self {
        QuotaSet {
            hook,
            full_res: q.full_res_images_per_minute.map(RateLimiter::new),
            highres: q.highres_previews_per_minute.map(RateLimiter::new),
            pdf: q.pdfs_per_minute.map(RateLimiter::new),
            text: q.texts_per_minute.map(RateLimiter::new),
            bandwidth: q.bandwidth_bytes_per_second.map(BandwidthLimiter::new),
        }
    }

    pub(crate) async fn acquire(&self, class: QuotaClass) {
        let limiter = match class {
            QuotaClass::Default => None,
            QuotaClass::FullResImage => self.full_res.as_ref(),
            QuotaClass::HighResPreview => self.highres.as_ref(),
            QuotaClass::Pdf => self.pdf.as_ref(),
            QuotaClass::Text => self.text.as_ref(),
        };
        if let Some(l) = limiter {
            l.acquire(class, self.hook.as_ref()).await;
        }
    }

    pub(crate) fn bandwidth(&self) -> Option<&BandwidthLimiter> {
        self.bandwidth.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimum_duration_estimates() {
        let q = Quotas::bnf();
        assert_eq!(q.minimum_duration(QuotaClass::Default, 10_000), None);
        assert_eq!(
            q.minimum_duration(QuotaClass::FullResImage, 5),
            Some(Duration::ZERO)
        );
        assert_eq!(
            q.minimum_duration(QuotaClass::FullResImage, 6),
            Some(Duration::from_secs(60))
        );
        assert_eq!(
            q.minimum_duration(QuotaClass::FullResImage, 10),
            Some(Duration::from_secs(60))
        );
        assert_eq!(
            q.minimum_duration(QuotaClass::FullResImage, 11),
            Some(Duration::from_secs(120))
        );
        // 1186 issues x 8 pages at full resolution: about a day and a half.
        let d = q
            .minimum_duration(QuotaClass::FullResImage, 1186 * 8)
            .unwrap();
        assert!(d > Duration::from_secs(30 * 3600));
        assert_eq!(
            Quotas::unlimited().minimum_duration(QuotaClass::Pdf, 100),
            None
        );
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limiter_blocks_past_the_limit() {
        let limiter = RateLimiter::new(3);
        let start = Instant::now();
        for _ in 0..3 {
            limiter.acquire(QuotaClass::Pdf, None).await;
        }
        assert_eq!(Instant::now(), start);
        limiter.acquire(QuotaClass::Pdf, None).await;
        assert!(Instant::now() - start >= WINDOW);
        // The window has slid: two more go through, the third waits again.
        let t = Instant::now();
        limiter.acquire(QuotaClass::Pdf, None).await;
        limiter.acquire(QuotaClass::Pdf, None).await;
        assert_eq!(Instant::now(), t);
        limiter.acquire(QuotaClass::Pdf, None).await;
        assert!(Instant::now() - t >= Duration::from_secs(1));
    }

    #[tokio::test(start_paused = true)]
    async fn bandwidth_limiter_paces_bytes() {
        let limiter = BandwidthLimiter::new(1000);
        let start = Instant::now();
        // One second of burst is free.
        limiter.consume(1000).await;
        assert_eq!(Instant::now(), start);
        // The next 2000 bytes must take about two seconds.
        limiter.consume(2000).await;
        let elapsed = Instant::now() - start;
        assert!(
            elapsed >= Duration::from_secs(2) && elapsed < Duration::from_millis(2100),
            "{elapsed:?}"
        );
    }
}
