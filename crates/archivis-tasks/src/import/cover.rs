use std::io::Cursor;
use std::path::Path;
use std::sync::OnceLock;

use archivis_formats::CoverData;
use archivis_storage::StorageBackend;
use image::imageops::FilterType;
use image::DynamicImage;
use tokio::fs;
use tracing::debug;
use uuid::Uuid;

use super::types::ThumbnailSizes;

/// Store the primary cover image alongside the book in the storage backend.
///
/// Returns the relative storage path of the stored cover.
pub async fn store_cover(
    storage: &impl StorageBackend,
    book_path_dir: &str,
    cover: &CoverData,
) -> Result<String, String> {
    let cover_path = cover_storage_path(book_path_dir, &cover.media_type);

    storage
        .store(&cover_path, &cover.bytes)
        .await
        .map_err(|e| format!("failed to store cover: {e}"))?;

    Ok(cover_path)
}

/// Build the canonical storage path for a book cover from its directory and media type.
pub fn cover_storage_path(book_path_dir: &str, media_type: &str) -> String {
    let ext = media_type_to_extension(media_type);
    format!("{book_path_dir}/cover.{ext}")
}

/// Generate small and medium WebP thumbnails from cover image data.
///
/// Thumbnails are written to `{cache_dir}/covers/{book_id}/sm.webp` and `md.webp`.
/// Handles both raster images (JPEG, PNG, etc.) and SVG covers via rasterization.
pub async fn generate_thumbnails(
    cover: &CoverData,
    book_id: Uuid,
    cache_dir: &Path,
    sizes: &ThumbnailSizes,
) -> Result<(), String> {
    let img = load_cover_image(&cover.bytes, Some(&cover.media_type))?;

    let covers_dir = cache_dir.join("covers").join(book_id.to_string());
    fs::create_dir_all(&covers_dir)
        .await
        .map_err(|e| format!("failed to create thumbnail directory: {e}"))?;

    for (name, target_height) in [("sm", sizes.sm_height), ("md", sizes.md_height)] {
        let resized = resize_to_height(&img, target_height);
        let webp_bytes = encode_webp(&resized)?;
        let thumb_path = covers_dir.join(format!("{name}.webp"));
        fs::write(&thumb_path, &webp_bytes)
            .await
            .map_err(|e| format!("failed to write thumbnail {name}: {e}"))?;
    }

    Ok(())
}

/// Generate a single WebP thumbnail at a given target height.
///
/// Reads the source image from `source_path`, resizes, and writes to
/// `{cache_dir}/covers/{book_id}/{name}.webp`.
pub async fn generate_thumbnail(
    source_path: &Path,
    book_id: Uuid,
    cache_dir: &Path,
    name: &str,
    target_height: u32,
) -> Result<std::path::PathBuf, String> {
    let source_bytes = fs::read(source_path)
        .await
        .map_err(|e| format!("failed to read source image: {e}"))?;

    let img = load_cover_image(&source_bytes, None)?;

    let resized = resize_to_height(&img, target_height);
    let webp_bytes = encode_webp(&resized)?;

    let covers_dir = cache_dir.join("covers").join(book_id.to_string());
    fs::create_dir_all(&covers_dir)
        .await
        .map_err(|e| format!("failed to create thumbnail directory: {e}"))?;

    let thumb_path = covers_dir.join(format!("{name}.webp"));
    fs::write(&thumb_path, &webp_bytes)
        .await
        .map_err(|e| format!("failed to write thumbnail {name}: {e}"))?;

    Ok(thumb_path)
}

/// Load cover image data, handling both raster formats and SVG.
///
/// If `media_type` is provided, it's used as a hint; otherwise the content is
/// sniffed for SVG markers.
fn load_cover_image(bytes: &[u8], media_type: Option<&str>) -> Result<DynamicImage, String> {
    let is_svg = media_type.is_some_and(|mt| mt == "image/svg+xml") || is_svg_data(bytes);

    if is_svg {
        debug!("detected SVG cover, rasterizing for thumbnails");
        rasterize_svg(bytes)
    } else {
        image::load_from_memory(bytes).map_err(|e| format!("failed to decode cover image: {e}"))
    }
}

/// Detect whether raw bytes look like SVG content.
fn is_svg_data(bytes: &[u8]) -> bool {
    // Skip UTF-8 BOM if present
    let content = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let preview = std::str::from_utf8(content.get(..256).unwrap_or(content)).unwrap_or("");
    let trimmed = preview.trim_start();
    trimmed.starts_with("<?xml") || trimmed.starts_with("<svg")
}

/// Return a lazily-initialized `usvg::Options` with system fonts pre-loaded.
///
/// System font enumeration is expensive (100-500ms), so we do it once and
/// reuse the result for all subsequent SVG rasterizations.
fn svg_options() -> &'static resvg::usvg::Options<'static> {
    static OPTIONS: OnceLock<resvg::usvg::Options<'static>> = OnceLock::new();
    OPTIONS.get_or_init(|| {
        let mut opt = resvg::usvg::Options::default();
        opt.fontdb_mut().load_system_fonts();
        opt
    })
}

/// Rasterize an SVG to a `DynamicImage` using resvg.
fn rasterize_svg(svg_data: &[u8]) -> Result<DynamicImage, String> {
    let opt = svg_options();

    let tree = resvg::usvg::Tree::from_data(svg_data, opt)
        .map_err(|e| format!("failed to parse SVG: {e}"))?;

    let size = tree.size().to_int_size();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size.width(), size.height())
        .ok_or("failed to create pixmap for SVG rendering (zero-size SVG?)")?;

    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::default(),
        &mut pixmap.as_mut(),
    );

    // Convert premultiplied RGBA (tiny-skia) to straight RGBA (image crate).
    // The division `(channel * 255) / alpha` always fits in u8 since channel <= alpha.
    #[allow(clippy::cast_possible_truncation)]
    let rgba = {
        let mut data = pixmap.take();
        for pixel in data.chunks_exact_mut(4) {
            let a = pixel[3];
            if a > 0 && a < 255 {
                let a16 = u16::from(a);
                pixel[0] = ((u16::from(pixel[0]) * 255) / a16) as u8;
                pixel[1] = ((u16::from(pixel[1]) * 255) / a16) as u8;
                pixel[2] = ((u16::from(pixel[2]) * 255) / a16) as u8;
            }
        }
        data
    };

    let img = image::RgbaImage::from_raw(size.width(), size.height(), rgba)
        .ok_or("failed to create image from SVG pixel data")?;

    Ok(DynamicImage::ImageRgba8(img))
}

/// Resize an image to fit a target height, preserving the aspect ratio.
fn resize_to_height(img: &image::DynamicImage, target_height: u32) -> image::DynamicImage {
    let (w, h) = (img.width(), img.height());
    if h == 0 {
        return img.clone();
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let new_width = ((f64::from(w) / f64::from(h)) * f64::from(target_height)) as u32;
    img.resize(new_width, target_height, FilterType::Lanczos3)
}

/// Encode a `DynamicImage` as WebP bytes.
fn encode_webp(img: &image::DynamicImage) -> Result<Vec<u8>, String> {
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::WebP)
        .map_err(|e| format!("failed to encode WebP: {e}"))?;
    Ok(buf.into_inner())
}

/// Map a MIME media type to a file extension.
fn media_type_to_extension(media_type: &str) -> &str {
    match media_type {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        _ => "img",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal valid SVG for testing.
    fn simple_svg() -> Vec<u8> {
        br##"<?xml version="1.0" encoding="utf-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 150">
  <rect width="100" height="150" fill="#336699"/>
</svg>"##
            .to_vec()
    }

    /// SVG with an embedded raster image via data URI (mimics Standard Ebooks covers).
    fn svg_with_embedded_image() -> Vec<u8> {
        // Minimal 1x1 red PNG as base64
        let png_b64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==";
        format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 200 300">
  <image width="200" height="300" xlink:href="data:image/png;base64,{png_b64}"/>
  <rect x="10" y="250" width="180" height="40" fill="rgba(0,0,0,0.7)"/>
</svg>"#
        )
        .into_bytes()
    }

    #[test]
    fn is_svg_data_detects_xml_declaration() {
        let svg = simple_svg();
        assert!(is_svg_data(&svg));
    }

    #[test]
    fn is_svg_data_detects_svg_root() {
        let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"><rect/></svg>";
        assert!(is_svg_data(svg));
    }

    #[test]
    fn is_svg_data_with_bom() {
        let mut bom_svg = vec![0xEF, 0xBB, 0xBF]; // UTF-8 BOM
        bom_svg.extend_from_slice(b"<svg><rect/></svg>");
        assert!(is_svg_data(&bom_svg));
    }

    #[test]
    fn is_svg_data_rejects_png() {
        // PNG magic bytes
        let png = b"\x89PNG\r\n\x1a\n";
        assert!(!is_svg_data(png));
    }

    #[test]
    fn is_svg_data_rejects_jpeg() {
        let jpeg = b"\xFF\xD8\xFF\xE0";
        assert!(!is_svg_data(jpeg));
    }

    #[test]
    fn rasterize_simple_svg() {
        let svg = simple_svg();
        let img = rasterize_svg(&svg).unwrap();
        assert_eq!(img.width(), 100);
        assert_eq!(img.height(), 150);
    }

    #[test]
    fn rasterize_svg_with_embedded_image() {
        let svg = svg_with_embedded_image();
        let img = rasterize_svg(&svg).unwrap();
        assert_eq!(img.width(), 200);
        assert_eq!(img.height(), 300);
    }

    #[test]
    fn rasterize_invalid_svg_returns_error() {
        let result = rasterize_svg(b"<not-valid-svg>");
        assert!(result.is_err());
    }

    #[test]
    fn load_cover_image_svg_by_media_type() {
        let svg = simple_svg();
        let img = load_cover_image(&svg, Some("image/svg+xml")).unwrap();
        assert_eq!(img.width(), 100);
        assert_eq!(img.height(), 150);
    }

    #[test]
    fn load_cover_image_svg_by_sniffing() {
        let svg = simple_svg();
        // No media type provided — should detect SVG from content
        let img = load_cover_image(&svg, None).unwrap();
        assert_eq!(img.width(), 100);
        assert_eq!(img.height(), 150);
    }

    #[test]
    fn load_cover_image_raster() {
        // Create a minimal valid 1x1 PNG
        let mut buf = Cursor::new(Vec::new());
        let img = image::RgbImage::from_pixel(1, 1, image::Rgb([255, 0, 0]));
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();

        let result = load_cover_image(&buf.into_inner(), Some("image/png")).unwrap();
        assert_eq!(result.width(), 1);
        assert_eq!(result.height(), 1);
    }

    #[tokio::test]
    async fn generate_thumbnails_from_svg_cover() {
        let svg = simple_svg();
        let cover = CoverData {
            bytes: svg,
            media_type: "image/svg+xml".into(),
        };

        let tmp = tempfile::tempdir().unwrap();
        let book_id = Uuid::new_v4();
        let sizes = ThumbnailSizes::default();

        generate_thumbnails(&cover, book_id, tmp.path(), &sizes)
            .await
            .unwrap();

        let covers_dir = tmp.path().join("covers").join(book_id.to_string());
        assert!(covers_dir.join("sm.webp").exists());
        assert!(covers_dir.join("md.webp").exists());

        // Verify the generated thumbnails are valid images
        let sm_bytes = std::fs::read(covers_dir.join("sm.webp")).unwrap();
        let sm_img = image::load_from_memory(&sm_bytes).unwrap();
        assert_eq!(sm_img.height(), sizes.sm_height);
    }

    #[tokio::test]
    async fn generate_thumbnail_from_svg_file() {
        let svg = simple_svg();
        let tmp = tempfile::tempdir().unwrap();
        let source_path = tmp.path().join("cover.svg");
        std::fs::write(&source_path, &svg).unwrap();

        let book_id = Uuid::new_v4();
        let result = generate_thumbnail(&source_path, book_id, tmp.path(), "lg", 600)
            .await
            .unwrap();

        assert!(result.exists());
        let img = image::load_from_memory(&std::fs::read(&result).unwrap()).unwrap();
        assert_eq!(img.height(), 600);
    }
}
