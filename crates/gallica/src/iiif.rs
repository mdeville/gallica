//! The IIIF Image and Presentation APIs of Gallica.
//!
//! See <https://api.bnf.fr/fr/api-iiif-de-recuperation-des-images-de-gallica>
//! and the IIIF Image API 2.x specification.
//!
//! Image URLs look like
//! `https://gallica.bnf.fr/iiif/ark:/12148/<name>/f<view>/<region>/<size>/<rotation>/<quality>.<format>`.

use std::fmt;
use std::path::Path;

use bytes::Bytes;
use serde::Deserialize;
use serde_json::Value;
use url::Url;

use crate::ark::Ark;
use crate::client::Client;
use crate::download::{DownloadOptions, DownloadOutcome};
use crate::error::{Error, Result};

/// The region of the image to return.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Region {
    /// The whole image.
    Full,
    /// The largest centred square.
    Square,
    /// A rectangle in pixels of the full image.
    Pixels {
        /// Left edge.
        x: u32,
        /// Top edge.
        y: u32,
        /// Width.
        w: u32,
        /// Height.
        h: u32,
    },
    /// A rectangle in percent of the full image.
    Percent {
        /// Left edge.
        x: f32,
        /// Top edge.
        y: f32,
        /// Width.
        w: f32,
        /// Height.
        h: f32,
    },
}

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Region::Full => f.write_str("full"),
            Region::Square => f.write_str("square"),
            Region::Pixels { x, y, w, h } => write!(f, "{x},{y},{w},{h}"),
            Region::Percent { x, y, w, h } => write!(
                f,
                "pct:{},{},{},{}",
                fmt_f(*x),
                fmt_f(*y),
                fmt_f(*w),
                fmt_f(*h)
            ),
        }
    }
}

/// The size of the returned image.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Size {
    /// The full size of the region (`full`).
    Full,
    /// The maximum size the server allows (`max`, IIIF 2.1).
    Max,
    /// Scale to this width, keeping the aspect ratio.
    Width(u32),
    /// Scale to this height, keeping the aspect ratio.
    Height(u32),
    /// Scale to this percentage.
    Percent(f32),
    /// Scale to exactly this size, distorting if needed.
    Exact(u32, u32),
    /// Scale to fit inside this box, keeping the aspect ratio.
    Fit(u32, u32),
}

impl fmt::Display for Size {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Size::Full => f.write_str("full"),
            Size::Max => f.write_str("max"),
            Size::Width(w) => write!(f, "{w},"),
            Size::Height(h) => write!(f, ",{h}"),
            Size::Percent(p) => write!(f, "pct:{}", fmt_f(*p)),
            Size::Exact(w, h) => write!(f, "{w},{h}"),
            Size::Fit(w, h) => write!(f, "!{w},{h}"),
        }
    }
}

/// Rotation of the returned image.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rotation {
    /// Degrees clockwise, 0-360.
    pub degrees: f32,
    /// Mirror horizontally before rotating.
    pub mirror: bool,
}

impl Default for Rotation {
    fn default() -> Self {
        Rotation {
            degrees: 0.0,
            mirror: false,
        }
    }
}

impl fmt::Display for Rotation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mirror {
            f.write_str("!")?;
        }
        f.write_str(&fmt_f(self.degrees))
    }
}

/// Quality of the returned image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Quality {
    /// The image as stored (Gallica's default, IIIF 1.x/2.0 name).
    Native,
    /// The image as stored (IIIF 2.1 name).
    Default,
    /// Colour.
    Color,
    /// Greyscale.
    Gray,
    /// Black and white.
    Bitonal,
}

impl fmt::Display for Quality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Quality::Native => "native",
            Quality::Default => "default",
            Quality::Color => "color",
            Quality::Gray => "gray",
            Quality::Bitonal => "bitonal",
        })
    }
}

/// Output format of the returned image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    /// JPEG.
    Jpg,
    /// PNG.
    Png,
    /// TIFF.
    Tif,
    /// GIF.
    Gif,
    /// WebP.
    Webp,
}

impl Format {
    /// The file extension (and URL suffix).
    pub fn extension(self) -> &'static str {
        match self {
            Format::Jpg => "jpg",
            Format::Png => "png",
            Format::Tif => "tif",
            Format::Gif => "gif",
            Format::Webp => "webp",
        }
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.extension())
    }
}

/// The parameters of an IIIF image request.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageRequest {
    /// Region.
    pub region: Region,
    /// Size.
    pub size: Size,
    /// Rotation.
    pub rotation: Rotation,
    /// Quality.
    pub quality: Quality,
    /// Format.
    pub format: Format,
}

impl Default for ImageRequest {
    /// Full image, full size, no rotation, native quality, JPEG.
    fn default() -> Self {
        ImageRequest {
            region: Region::Full,
            size: Size::Full,
            rotation: Rotation::default(),
            quality: Quality::Native,
            format: Format::Jpg,
        }
    }
}

impl ImageRequest {
    /// Full image at full resolution as JPEG.
    pub fn full() -> Self {
        Self::default()
    }

    /// Sets the region.
    pub fn region(mut self, region: Region) -> Self {
        self.region = region;
        self
    }

    /// Sets the size.
    pub fn size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }

    /// Sets the rotation.
    pub fn rotation(mut self, rotation: Rotation) -> Self {
        self.rotation = rotation;
        self
    }

    /// Sets the quality.
    pub fn quality(mut self, quality: Quality) -> Self {
        self.quality = quality;
        self
    }

    /// Sets the format.
    pub fn format(mut self, format: Format) -> Self {
        self.format = format;
        self
    }

    /// The path suffix `region/size/rotation/quality.format`.
    pub fn path(&self) -> String {
        format!(
            "{}/{}/{}/{}.{}",
            self.region, self.size, self.rotation, self.quality, self.format
        )
    }
}

fn fmt_f(v: f32) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// The `info.json` of one view (IIIF Image API 2.x).
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct ImageInfo {
    /// The image service identifier.
    #[serde(rename = "@id", default)]
    pub id: String,
    /// Full image width in pixels.
    pub width: u32,
    /// Full image height in pixels.
    pub height: u32,
    /// Sizes the server pre-computes, if any.
    #[serde(default)]
    pub sizes: Vec<ImageSize>,
    /// Tile specifications, if any.
    #[serde(default)]
    pub tiles: Vec<TileSpec>,
    /// Compliance profile (raw).
    #[serde(default)]
    pub profile: Value,
}

/// A width/height pair.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
pub struct ImageSize {
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

/// Tile specification from `info.json`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct TileSpec {
    /// Tile width.
    pub width: u32,
    /// Tile height (defaults to the width).
    #[serde(default)]
    pub height: Option<u32>,
    /// Available scale factors.
    #[serde(rename = "scaleFactors", default)]
    pub scale_factors: Vec<u32>,
}

/// A IIIF Presentation 2.x manifest, reduced to what a downloader needs.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct Manifest {
    /// Manifest identifier (URL).
    pub id: String,
    /// Label.
    pub label: Option<String>,
    /// Descriptive metadata as `(label, value)` pairs.
    pub metadata: Vec<(String, String)>,
    /// Attribution / rights statement.
    pub attribution: Option<String>,
    /// Licence URL.
    pub license: Option<String>,
    /// Thumbnail URL.
    pub thumbnail: Option<String>,
    /// Canvases (views) of the first sequence, in order.
    pub canvases: Vec<Canvas>,
}

/// One canvas (view) of a manifest.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct Canvas {
    /// 1-based view number (`f<N>`), inferred from the canvas identifier or
    /// from its position.
    pub view: u32,
    /// Canvas identifier.
    pub id: String,
    /// Label (printed page number, caption...).
    pub label: Option<String>,
    /// Canvas width.
    pub width: Option<u32>,
    /// Canvas height.
    pub height: Option<u32>,
    /// The image service base URL, when present.
    pub service_id: Option<String>,
    /// The URL of the image resource as given by the manifest.
    pub image_url: Option<String>,
}

impl Manifest {
    /// Parses a manifest from its JSON.
    pub fn from_json(json: &str) -> Result<Manifest> {
        let v: Value = serde_json::from_str(json)?;
        Ok(Self::from_value(&v))
    }

    /// Reads a manifest from a JSON value, tolerating the various shapes
    /// labels and metadata values take in IIIF 2.x.
    pub fn from_value(v: &Value) -> Manifest {
        let mut m = Manifest {
            id: str_at(v, "@id").unwrap_or_default(),
            label: label_text(v.get("label")),
            attribution: label_text(v.get("attribution")),
            license: str_at(v, "license"),
            thumbnail: v.get("thumbnail").and_then(id_or_str),
            ..Default::default()
        };
        if let Some(Value::Array(items)) = v.get("metadata") {
            for item in items {
                if let (Some(l), Some(val)) =
                    (label_text(item.get("label")), label_text(item.get("value")))
                {
                    m.metadata.push((l, val));
                }
            }
        }
        let canvases = v
            .get("sequences")
            .and_then(|s| s.as_array())
            .and_then(|s| s.first())
            .and_then(|s| s.get("canvases"))
            .or_else(|| v.get("items"))
            .and_then(|c| c.as_array());
        if let Some(canvases) = canvases {
            for (i, c) in canvases.iter().enumerate() {
                let id = str_at(c, "@id")
                    .or_else(|| str_at(c, "id"))
                    .unwrap_or_default();
                let image = c
                    .get("images")
                    .and_then(|im| im.as_array())
                    .and_then(|im| im.first())
                    .and_then(|im| im.get("resource"));
                m.canvases.push(Canvas {
                    view: view_from_id(&id).unwrap_or(i as u32 + 1),
                    id,
                    label: label_text(c.get("label")),
                    width: c.get("width").and_then(Value::as_u64).map(|w| w as u32),
                    height: c.get("height").and_then(Value::as_u64).map(|h| h as u32),
                    service_id: image.and_then(|r| r.get("service")).and_then(id_or_str),
                    image_url: image.and_then(|r| str_at(r, "@id")),
                });
            }
        }
        m
    }
}

fn str_at(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn id_or_str(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Object(_) => str_at(v, "@id").or_else(|| str_at(v, "id")),
        Value::Array(a) => a.first().and_then(id_or_str),
        _ => None,
    }
}

fn label_text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Array(a) => {
            let parts: Vec<String> = a.iter().filter_map(|x| label_text(Some(x))).collect();
            (!parts.is_empty()).then(|| parts.join("; "))
        }
        Value::Object(o) => {
            if let Some(s) = o.get("@value") {
                return label_text(Some(s));
            }
            // IIIF 3 style language map: { "fr": ["..."] }
            o.values().find_map(|x| label_text(Some(x)))
        }
        _ => None,
    }
}

fn view_from_id(id: &str) -> Option<u32> {
    // .../ark:/12148/bpt6k.../f12 or .../f12/canvas
    id.split('/')
        .filter_map(|seg| seg.strip_prefix('f'))
        .rfind(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
        .and_then(|s| s.parse().ok())
}

impl Client {
    /// The IIIF image service base URL for one view: `iiif/ark:/12148/<name>/f<view>`.
    pub fn iiif_service_url(&self, ark: &Ark, view: u32) -> Result<Url> {
        self.url(&format!("iiif/{}/f{view}", ark.id()))
    }

    /// The URL of an IIIF image.
    pub fn iiif_image_url(&self, ark: &Ark, view: u32, request: &ImageRequest) -> Result<Url> {
        self.url(&format!("iiif/{}/f{view}/{}", ark.id(), request.path()))
    }

    /// The URL of the `info.json` of one view.
    pub fn iiif_info_url(&self, ark: &Ark, view: u32) -> Result<Url> {
        self.url(&format!("iiif/{}/f{view}/info.json", ark.id()))
    }

    /// The URL of the presentation manifest of a document.
    pub fn iiif_manifest_url(&self, ark: &Ark) -> Result<Url> {
        self.url(&format!("iiif/{}/manifest.json", ark.id()))
    }

    /// Fetches the `info.json` of one view.
    pub async fn iiif_info(&self, ark: &Ark, view: u32) -> Result<ImageInfo> {
        self.get_json(self.iiif_info_url(ark, view)?).await
    }

    /// Fetches the presentation manifest of a document.
    pub async fn iiif_manifest(&self, ark: &Ark) -> Result<Manifest> {
        let v: Value = self.get_json(self.iiif_manifest_url(ark)?).await?;
        Ok(Manifest::from_value(&v))
    }

    /// Fetches an IIIF image into memory.
    pub async fn iiif_image(&self, ark: &Ark, view: u32, request: &ImageRequest) -> Result<Bytes> {
        let url = self.iiif_image_url(ark, view, request)?;
        let _permit = self.download_permit().await;
        let resp = self.get(url.clone()).await?;
        check_image_content_type(&resp, &url)?;
        Ok(resp.bytes().await?)
    }

    /// Downloads an IIIF image to a file. See [`Client::download`] for the
    /// behaviour of `options`.
    pub async fn download_iiif_image(
        &self,
        ark: &Ark,
        view: u32,
        request: &ImageRequest,
        path: impl AsRef<Path>,
        options: &DownloadOptions,
    ) -> Result<DownloadOutcome> {
        let url = self.iiif_image_url(ark, view, request)?;
        self.download(url, path, options).await
    }
}

pub(crate) fn check_image_content_type(resp: &reqwest::Response, url: &Url) -> Result<()> {
    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if ct.starts_with("text/html") {
        return Err(Error::Http {
            url: url.to_string(),
            status: resp.status().as_u16(),
            body: Some("server returned an HTML page instead of an image".into()),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_paths() {
        assert_eq!(ImageRequest::full().path(), "full/full/0/native.jpg");
        let r = ImageRequest::full()
            .region(Region::Pixels {
                x: 0,
                y: 1900,
                w: 2400,
                h: 1200,
            })
            .size(Size::Width(800))
            .rotation(Rotation {
                degrees: 90.0,
                mirror: true,
            })
            .quality(Quality::Gray)
            .format(Format::Png);
        assert_eq!(r.path(), "0,1900,2400,1200/800,/!90/gray.png");
        assert_eq!(Size::Fit(100, 200).to_string(), "!100,200");
        assert_eq!(Size::Percent(12.5).to_string(), "pct:12.5");
        assert_eq!(
            Region::Percent {
                x: 0.0,
                y: 0.0,
                w: 50.0,
                h: 50.0
            }
            .to_string(),
            "pct:0,0,50,50"
        );
    }

    #[test]
    fn urls() {
        let c = Client::new();
        let ark = Ark::parse("btv1b90017179").unwrap();
        assert_eq!(
            c.iiif_image_url(&ark, 15, &ImageRequest::full())
                .unwrap()
                .as_str(),
            "https://gallica.bnf.fr/iiif/ark:/12148/btv1b90017179/f15/full/full/0/native.jpg"
        );
        assert_eq!(
            c.iiif_manifest_url(&ark).unwrap().as_str(),
            "https://gallica.bnf.fr/iiif/ark:/12148/btv1b90017179/manifest.json"
        );
        assert_eq!(
            c.iiif_info_url(&ark, 1).unwrap().as_str(),
            "https://gallica.bnf.fr/iiif/ark:/12148/btv1b90017179/f1/info.json"
        );
    }

    #[test]
    fn manifest_parsing() {
        let json = r#"{
          "@context": "http://iiif.io/api/presentation/2.0/context.json",
          "@id": "https://gallica.bnf.fr/iiif/ark:/12148/btv1b550076223/manifest.json",
          "@type": "sc:Manifest",
          "label": "BnF, Estampes",
          "attribution": "Bibliothèque nationale de France",
          "license": "https://gallica.bnf.fr/html/und/conditions-dutilisation-des-contenus-de-gallica",
          "thumbnail": {"@id": "https://gallica.bnf.fr/ark:/12148/btv1b550076223.thumbnail"},
          "metadata": [
            {"label": "Title", "value": "Some title"},
            {"label": "Date", "value": [{"@value": "1900", "@language": "fr"}]}
          ],
          "sequences": [{"canvases": [
            {"@id": "https://gallica.bnf.fr/iiif/ark:/12148/btv1b550076223/canvas/f1", "label": "NP", "width": 1000, "height": 1500,
             "images": [{"resource": {"@id": "https://gallica.bnf.fr/iiif/ark:/12148/btv1b550076223/f1/full/full/0/native.jpg",
                                      "service": {"@id": "https://gallica.bnf.fr/iiif/ark:/12148/btv1b550076223/f1"}}}]},
            {"@id": "https://gallica.bnf.fr/iiif/ark:/12148/btv1b550076223/canvas/f2", "label": "2"}
          ]}]
        }"#;
        let m = Manifest::from_json(json).unwrap();
        assert_eq!(m.label.as_deref(), Some("BnF, Estampes"));
        assert_eq!(
            m.metadata,
            vec![
                ("Title".into(), "Some title".into()),
                ("Date".into(), "1900".into())
            ]
        );
        assert_eq!(
            m.thumbnail.as_deref(),
            Some("https://gallica.bnf.fr/ark:/12148/btv1b550076223.thumbnail")
        );
        assert_eq!(m.canvases.len(), 2);
        assert_eq!(m.canvases[0].view, 1);
        assert_eq!(m.canvases[0].width, Some(1000));
        assert_eq!(
            m.canvases[0].service_id.as_deref(),
            Some("https://gallica.bnf.fr/iiif/ark:/12148/btv1b550076223/f1")
        );
        assert_eq!(m.canvases[1].view, 2);
        assert_eq!(m.canvases[1].label.as_deref(), Some("2"));
    }
}
