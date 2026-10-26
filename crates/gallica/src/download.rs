//! Streaming downloads to disk.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt;
use tokio::io::AsyncWriteExt;
use url::Url;

use crate::client::Client;
use crate::error::{Error, Result};
use crate::quota::{BandwidthLimiter, QuotaClass};

/// Progress callback: `(bytes received so far, total bytes if known)`.
pub type ProgressFn = Arc<dyn Fn(u64, Option<u64>) + Send + Sync>;

/// Options for [`Client::download`].
#[derive(Clone, Default)]
pub struct DownloadOptions {
    /// Overwrite an existing file. When `false` (the default) an existing
    /// non-empty file is left alone and the download is reported as skipped.
    pub overwrite: bool,
    /// Called as bytes arrive.
    pub progress: Option<ProgressFn>,
}

impl std::fmt::Debug for DownloadOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DownloadOptions")
            .field("overwrite", &self.overwrite)
            .field("progress", &self.progress.is_some())
            .finish()
    }
}

impl DownloadOptions {
    /// Default options: do not overwrite, no progress reporting.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets whether existing files are overwritten.
    pub fn overwrite(mut self, overwrite: bool) -> Self {
        self.overwrite = overwrite;
        self
    }

    /// Sets a progress callback.
    pub fn progress(mut self, f: impl Fn(u64, Option<u64>) + Send + Sync + 'static) -> Self {
        self.progress = Some(Arc::new(f));
        self
    }
}

/// The result of a download.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DownloadOutcome {
    /// Where the file was written.
    pub path: PathBuf,
    /// Number of bytes written (or already present when skipped).
    pub bytes: u64,
    /// `true` when the file already existed and was left untouched.
    pub skipped: bool,
    /// The `Content-Type` of the answer, when the download happened.
    pub content_type: Option<String>,
}

impl Client {
    /// Downloads `url` to `path`, streaming through a temporary file that is
    /// renamed into place once complete, so a crash never leaves a truncated
    /// file at `path`. Parent directories are created as needed. Downloads are
    /// limited to the concurrency configured on the client.
    pub async fn download(
        &self,
        url: Url,
        path: impl AsRef<Path>,
        options: &DownloadOptions,
    ) -> Result<DownloadOutcome> {
        self.download_classed(url, path, options, QuotaClass::Default)
            .await
    }

    /// Like [`Client::download`], counting the request against the quota of
    /// the given class.
    pub async fn download_classed(
        &self,
        url: Url,
        path: impl AsRef<Path>,
        options: &DownloadOptions,
        class: QuotaClass,
    ) -> Result<DownloadOutcome> {
        let path = path.as_ref().to_path_buf();
        if !options.overwrite
            && let Ok(meta) = tokio::fs::metadata(&path).await
            && meta.is_file()
            && meta.len() > 0
        {
            return Ok(DownloadOutcome {
                path,
                bytes: meta.len(),
                skipped: true,
                content_type: None,
            });
        }
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| Error::io(Some(parent.to_path_buf()), e))?;
        }

        let _permit = self.download_permit().await;
        let resp = self.get_classed(url.clone(), class).await?;
        crate::iiif::check_image_content_type(&resp, &url)?;
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let total = resp.content_length();

        let tmp = temp_path(&path);
        let result = write_stream(
            resp,
            &tmp,
            total,
            options.progress.as_ref(),
            self.bandwidth_limiter(),
        )
        .await;
        match result {
            Ok(bytes) => {
                tokio::fs::rename(&tmp, &path)
                    .await
                    .map_err(|e| Error::io(Some(path.clone()), e))?;
                Ok(DownloadOutcome {
                    path,
                    bytes,
                    skipped: false,
                    content_type,
                })
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&tmp).await;
                Err(e)
            }
        }
    }
}

async fn write_stream(
    resp: reqwest::Response,
    tmp: &Path,
    total: Option<u64>,
    progress: Option<&ProgressFn>,
    bandwidth: Option<&BandwidthLimiter>,
) -> Result<u64> {
    let mut file = tokio::fs::File::create(tmp)
        .await
        .map_err(|e| Error::io(Some(tmp.to_path_buf()), e))?;
    let mut stream = resp.bytes_stream();
    let mut written = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if let Some(b) = bandwidth {
            b.consume(chunk.len()).await;
        }
        file.write_all(&chunk)
            .await
            .map_err(|e| Error::io(Some(tmp.to_path_buf()), e))?;
        written += chunk.len() as u64;
        if let Some(p) = progress {
            p(written, total);
        }
    }
    file.flush()
        .await
        .map_err(|e| Error::io(Some(tmp.to_path_buf()), e))?;
    if let Some(t) = total
        && written != t
    {
        return Err(Error::Http {
            url: String::new(),
            status: 0,
            body: Some(format!("truncated download: got {written} of {t} bytes")),
        });
    }
    Ok(written)
}

fn temp_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".part");
    path.with_file_name(name)
}
