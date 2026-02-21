use std::io::Cursor;
use std::path::Path;

use archivis_formats::CoverData;
use archivis_storage::StorageBackend;
use image::imageops::FilterType;
use tokio::fs;
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
    let ext = media_type_to_extension(&cover.media_type);
    let cover_path = format!("{book_path_dir}/cover.{ext}");

    storage
        .store(&cover_path, &cover.bytes)
        .await
        .map_err(|e| format!("failed to store cover: {e}"))?;

    Ok(cover_path)
}

/// Generate small and medium WebP thumbnails from cover image data.
///
/// Thumbnails are written to `{cache_dir}/covers/{book_id}/sm.webp` and `md.webp`.
pub async fn generate_thumbnails(
    cover: &CoverData,
    book_id: Uuid,
    cache_dir: &Path,
    sizes: &ThumbnailSizes,
) -> Result<(), String> {
    let img = image::load_from_memory(&cover.bytes)
        .map_err(|e| format!("failed to decode cover image: {e}"))?;

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
