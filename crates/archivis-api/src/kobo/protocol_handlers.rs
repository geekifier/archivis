//! Kobo protocol handlers — these are NOT documented via `OpenAPI`; they
//! mirror the Kobo store's wire format for use by paired devices.

use std::collections::HashMap;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde::Serialize;
use uuid::Uuid;

use archivis_core::errors::DbError;
use archivis_core::models::BookFormat;
use archivis_core::public_url::PublicBaseUrl;
use archivis_db::{
    BookFileRepository, BookRepository, KoboDeviceSyncItemRepository, KoboSyncSelectionRepository,
};
use archivis_storage::StorageBackend;

use crate::errors::ApiError;
use crate::kobo::extractor::KoboDeviceAuth;
use crate::kobo::sync_protocol::{
    compute_revision_hash, diff_desired, diff_removal, DesiredItem, DiffOutcome, LedgerWrite,
    SyncEntry,
};
use crate::state::AppState;

// ── /v1/initialization ──────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct InitializationResponse {
    resources: HashMap<&'static str, String>,
}

pub async fn initialization(
    State(state): State<AppState>,
    auth: KoboDeviceAuth,
) -> Result<Json<InitializationResponse>, ApiError> {
    let base = require_public_base_url(&state)?;
    let token = auth.raw_token.as_str();

    let mut resources: HashMap<&'static str, String> = HashMap::new();

    // Routes Archivis owns. Override the canonical Kobo URLs.
    let ours = |suffix: &str| join_kobo_path(&base, token, suffix);
    resources.insert("library_sync", ours("/v1/library/sync"));
    resources.insert("library_metadata", ours("/v1/library/{ProductId}/metadata"));
    resources.insert("user_profile", ours("/v1/user/profile"));
    resources.insert(
        "image_host",
        base.to_string().trim_end_matches('/').to_string(),
    );
    resources.insert(
        "image_url_template",
        ours("/v1/products/images/{ImageId}/{Width}/{Height}/false/image.jpg"),
    );
    resources.insert(
        "image_url_quality_template",
        ours("/v1/products/images/{ImageId}/{Width}/{Height}/{Quality}/{IsGreyscale}/image.jpg"),
    );

    // Firmware uses REPLACE semantics on `[OneStoreServices]`: any key we
    // omit is wiped to empty on the device, breaking URL construction. Send
    // the full standard set with sensible defaults for everything we don't
    // intercept. Storeapi URLs end up redirected to our `api_endpoint` and
    // are caught by stub handlers; everything else points at the real Kobo
    // origin so the device retains its normal store/account UX.
    for (key, value) in STANDARD_KOBO_RESOURCES {
        resources.entry(key).or_insert_with(|| (*value).to_string());
    }

    Ok(Json(InitializationResponse { resources }))
}

const STANDARD_KOBO_RESOURCES: &[(&str, &str)] = &[
    ("account_page", "https://www.kobo.com/account/settings"),
    ("account_page_rakuten", "https://my.rakuten.co.jp/"),
    (
        "authentication_provider",
        "https://authorize.kobo.com/AuthenticationProvider",
    ),
    (
        "autocomplete",
        "https://storeapi.kobo.com/v1/products/autocomplete",
    ),
    (
        "book",
        "https://storeapi.kobo.com/v1/products/books/{ProductId}",
    ),
    (
        "book_detail_page",
        "https://www.kobo.com/{region}/{language}/ebook/{slug}",
    ),
    (
        "book_detail_page_rakuten",
        "http://books.rakuten.co.jp/rk/{crossrevisionid}",
    ),
    ("book_landing_page", "https://www.kobo.com/ebooks"),
    ("categories", "https://storeapi.kobo.com/v1/categories"),
    (
        "category",
        "https://storeapi.kobo.com/v1/categories/{CategoryId}",
    ),
    (
        "category_products",
        "https://storeapi.kobo.com/v1/categories/{CategoryId}/products",
    ),
    (
        "client_authd_referral",
        "https://authorize.kobo.com/api/AuthenticatedReferral/client/v1/getLink",
    ),
    (
        "daily_deal",
        "https://storeapi.kobo.com/v1/products/dailydeal",
    ),
    ("deals", "https://storeapi.kobo.com/v1/deals"),
    ("dictionary_host", "https://ereaderfiles.kobo.com"),
    ("display_accessibility_enabled", "False"),
    ("display_parental_controls_enabled", "False"),
    (
        "dropbox_link_account_poll",
        "https://authorize.kobo.com/{region}/{language}/LinkDropbox",
    ),
    (
        "eula_page",
        "https://www.kobo.com/termsofuse?style=onestore",
    ),
    (
        "featured_list",
        "https://storeapi.kobo.com/v1/products/featured/{FeaturedListId}",
    ),
    (
        "featured_lists",
        "https://storeapi.kobo.com/v1/products/featured",
    ),
    ("feedback", ""),
    (
        "free_books_page",
        "https://www.kobo.com/{region}/{language}/p/freebooks",
    ),
    (
        "fte_feedback",
        "https://storeapi.kobo.com/v1/products/ftefeedback",
    ),
    ("instapaper_enabled", "True"),
    ("instapaper_env_url", "https://www.instapaper.com/api/kobo"),
    (
        "instapaper_link_account_start",
        "https://authorize.kobo.com/{region}/{language}/linkinstapaper",
    ),
    (
        "googledrive_link_account_start",
        "https://authorize.kobo.com/{region}/{language}/linkcloudstorage/provider/google_drive",
    ),
    ("kda_store_browser_redirect_url", ""),
    ("kobo_audiobooks_credit_redemption", "False"),
    ("kobo_audiobooks_enabled", "True"),
    ("kobo_dropbox_link_account_enabled", "False"),
    ("kobo_googledrive_link_account_enabled", "False"),
    ("kobo_nativeborrow_enabled", "True"),
    ("kobo_privacyCentre_url", "https://www.kobo.com/privacy"),
    ("kobo_redeem_enabled", "True"),
    ("kobo_subscriptions_enabled", "True"),
    ("kobo_superpoints_enabled", "True"),
    ("kobo_wishlist_enabled", "True"),
    (
        "love_points_redemption_page",
        "https://www.kobo.com/{region}/{language}/KoboSuperPointsRedemption?productId={ProductId}",
    ),
    // Opaque, firmware-owned QSettings payload. It can be account/region
    // specific in live Kobo responses; this fallback keeps the key typed as
    // a QByteArray instead of letting initialization remove it entirely.
    (
        "love_data",
        r"@ByteArray(\0\0\0\x1\0\0\0\x10\0\x42\0\x65\0n\0\x65\0\x66\0i\0t\0s\0\0\0\b\0\0\0\0\0)",
    ),
    ("oauth_host", "https://oauth.kobo.com"),
    (
        "password_retrieval_page",
        "https://www.kobo.com/passwordretrieval.html",
    ),
    (
        "pocket_link_account_start",
        "https://authorize.kobo.com/{region}/{language}/linkpocket",
    ),
    (
        "post_analytics_event",
        "https://storeapi.kobo.com/v1/analytics/event",
    ),
    (
        "privacy_page",
        "https://www.kobo.com/privacypolicy?style=onestore",
    ),
    (
        "product_recommendations",
        "https://storeapi.kobo.com/v1/products/{ProductId}/recommendations",
    ),
    (
        "product_reviews",
        "https://storeapi.kobo.com/v1/products/{ProductIds}/reviews",
    ),
    (
        "purchase_buy_templated",
        "https://www.kobo.com/{region}/{language}/checkoutoption/{ProductId}",
    ),
    ("reading_services_host", "https://readingservices.kobo.com"),
    (
        "registration_page",
        "https://authorize.kobo.com/signup?returnUrl=https://kobo.com/",
    ),
    (
        "review",
        "https://storeapi.kobo.com/v1/products/reviews/{ReviewId}",
    ),
    (
        "review_sentiment",
        "https://storeapi.kobo.com/v1/products/reviews/{ReviewId}/sentiment/{Sentiment}",
    ),
    ("sign_in_page", "https://auth.kobobooks.com/ActivateOnWeb"),
    ("social_host", "https://social.kobobooks.com"),
    ("store_home", "www.kobo.com/{region}/{language}"),
    ("store_host", "www.kobo.com"),
    (
        "store_search",
        "https://www.kobo.com/{region}/{language}/Search?Query={query}",
    ),
    (
        "subs_landing_page",
        "https://www.kobo.com/{region}/{language}/plus",
    ),
    (
        "subs_management_page",
        "https://www.kobo.com/{region}/{language}/account/subscriptions",
    ),
    (
        "subs_plans_page",
        "https://www.kobo.com/{region}/{language}/plus/plans",
    ),
    (
        "subs_purchase_buy_templated",
        "https://www.kobo.com/{region}/{language}/Checkoutoption/{ProductId}/{TierId}",
    ),
    ("subscription_publisher_price_page", ""),
    (
        "taste_profile",
        "https://storeapi.kobo.com/v1/products/tasteprofile",
    ),
    ("user_ratings", "https://storeapi.kobo.com/v1/user/ratings"),
    (
        "user_recommendations",
        "https://storeapi.kobo.com/v1/user/recommendations",
    ),
    ("user_reviews", "https://storeapi.kobo.com/v1/user/reviews"),
    ("userguide_host", "https://ereaderfiles.kobo.com"),
];

// ── /v1/library/sync ────────────────────────────────────────────────

#[allow(clippy::too_many_lines)]
pub async fn library_sync(
    State(state): State<AppState>,
    auth: KoboDeviceAuth,
) -> Result<Response, ApiError> {
    let pool = state.db_pool();
    let now = Utc::now();

    if !crate::kobo::sync_enabled(&state) {
        let body = serde_json::to_vec(&Vec::<SyncEntry>::new())
            .map_err(|e| ApiError::Internal(format!("serialize sync response: {e}")))?;
        return build_sync_response(body, now, false);
    }

    let base = require_public_base_url(&state)?;
    let token = auth.raw_token.as_str();
    let page_limit = crate::kobo::sync_page_size(&state);
    let fetch_limit = i64::try_from(page_limit + 1)
        .map_err(|e| ApiError::Internal(format!("invalid Kobo sync page limit: {e}")))?;

    // 1. Load at most one page of active selections that may need to emit a
    // New/Changed entitlement. The ledger remains the cursor, so interrupted
    // syncs resume from rows not yet written.
    let mut candidate_selections = KoboSyncSelectionRepository::list_sync_candidate_page(
        pool,
        auth.user.id,
        auth.device.id,
        fetch_limit,
    )
    .await?;
    let mut has_more = candidate_selections.len() > page_limit;
    candidate_selections.truncate(page_limit);

    // 2. Diff selected items.
    let mut entries: Vec<SyncEntry> = Vec::new();
    let mut writes: Vec<LedgerWrite> = Vec::new();

    for sel in candidate_selections {
        let Some(file_id) = sel.selected_book_file_id else {
            continue;
        };
        let book = match BookRepository::get_by_id(pool, sel.book_id).await {
            Ok(b) => b,
            Err(DbError::NotFound { .. }) => continue,
            Err(e) => return Err(e.into()),
        };
        let file = match BookFileRepository::get_by_id(pool, file_id).await {
            Ok(f) if f.format == BookFormat::Epub && f.book_id == sel.book_id => f,
            Ok(_) | Err(DbError::NotFound { .. }) => continue,
            Err(e) => return Err(e.into()),
        };
        let item = DesiredItem {
            selection: sel,
            book,
            book_file: file,
        };
        let download_url = join_kobo_path(
            &base,
            token,
            &format!("/download/{}/{}", item.book.id, item.book_file.id),
        );
        let existing = KoboDeviceSyncItemRepository::find(pool, auth.device.id, item.book.id)
            .await?
            .filter(|row| row.book_id == item.book.id);
        match diff_desired(&item, existing.as_ref(), download_url, now) {
            DiffOutcome::Emit(entry, write) => {
                entries.push(entry);
                writes.push(write);
            }
            DiffOutcome::Skip => {
                // Candidate SQL deliberately over-selects rows whose book
                // updated_at is newer than delivered_at. If the canonical
                // revision hash did not change, refresh the ledger so the same
                // non-protocol-visible update does not block future pages.
                let revision_hash = compute_revision_hash(&item);
                writes.push(LedgerWrite::Upsert {
                    book_id: item.book.id,
                    book_file_id: Some(item.book_file.id),
                    file_hash: Some(item.book_file.hash.clone()),
                    desired_revision_hash: revision_hash,
                    selection_updated_at: item.selection.updated_at,
                    delivered_at: now,
                });
            }
        }
    }

    // 3. If the desired page did not fill the response, use remaining page
    // capacity for removals. Desired changes take priority so a re-selected
    // book is restored before old tombstones are drained.
    if !has_more && entries.len() < page_limit {
        let remaining = page_limit - entries.len();
        let removal_fetch_limit = i64::try_from(remaining + 1)
            .map_err(|e| ApiError::Internal(format!("invalid Kobo removal page limit: {e}")))?;
        let mut removal_rows = KoboDeviceSyncItemRepository::list_removal_candidate_page(
            pool,
            auth.device.id,
            auth.user.id,
            removal_fetch_limit,
        )
        .await?;
        has_more = removal_rows.len() > remaining;
        removal_rows.truncate(remaining);

        for row in &removal_rows {
            if let DiffOutcome::Emit(entry, write) = diff_removal(row, now) {
                entries.push(entry);
                writes.push(write);
            }
        }
    }

    // 4. Serialize before mutating the ledger. We still cannot know whether the
    // device receives the response, but failures up to this point leave the
    // ledger untouched and retriable.
    let body = serde_json::to_vec(&entries)
        .map_err(|e| ApiError::Internal(format!("serialize sync response: {e}")))?;

    // 5. Apply ledger writes for every emitted or validated row in this page.
    for write in writes {
        match write {
            LedgerWrite::Upsert {
                book_id,
                book_file_id,
                file_hash,
                desired_revision_hash,
                selection_updated_at,
                delivered_at,
            } => {
                KoboDeviceSyncItemRepository::upsert_delivered(
                    pool,
                    auth.device.id,
                    book_id,
                    book_file_id,
                    file_hash.as_deref(),
                    &desired_revision_hash,
                    selection_updated_at,
                    delivered_at,
                )
                .await?;
            }
            LedgerWrite::Tombstone { book_id, when } => {
                KoboDeviceSyncItemRepository::mark_tombstoned(pool, auth.device.id, book_id, when)
                    .await?;
            }
        }
    }

    // 6. Build the protocol response.
    build_sync_response(body, now, has_more)
}

fn build_sync_response(
    body: Vec<u8>,
    now: chrono::DateTime<Utc>,
    has_more: bool,
) -> Result<Response, ApiError> {
    // Header `x-kobo-synctoken` is opaque to the device; we use a plain
    // timestamp as the cursor.
    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .header(
            "x-kobo-synctoken",
            HeaderValue::from_str(&now.timestamp_millis().to_string()).unwrap(),
        )
        .header(CONTENT_LENGTH, body.len().to_string())
        .body(Body::from(body))
        .map_err(|e| ApiError::Internal(format!("build sync response: {e}")))?;
    let sync_state = if has_more { "continue" } else { "done" };
    response
        .headers_mut()
        .insert("x-kobo-sync", HeaderValue::from_static(sync_state));
    Ok(response)
}

// ── /v1/library/{book_id}/metadata ──────────────────────────────────

pub async fn library_metadata(
    State(state): State<AppState>,
    auth: KoboDeviceAuth,
    Path((_token, book_id)): Path<(String, Uuid)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !crate::kobo::sync_enabled(&state) {
        return Err(crate::kobo::sync_disabled_error());
    }

    let pool = state.db_pool();

    // Confirm the user has selected this book; otherwise 404.
    let selection = KoboSyncSelectionRepository::find(pool, auth.user.id, book_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("book not in user's Kobo library".into()))?;
    let Some(file_id) = selection.selected_book_file_id else {
        return Err(ApiError::NotFound("selection is stale".into()));
    };

    let book = BookRepository::get_by_id(pool, book_id).await?;
    let file = BookFileRepository::get_by_id(pool, file_id).await?;

    let base = require_public_base_url(&state)?;
    let token = auth.raw_token.as_str();
    let download_url = join_kobo_path(&base, token, &format!("/download/{}/{}", book.id, file.id));

    let item = DesiredItem {
        selection,
        book,
        book_file: file,
    };
    let revision_hash = crate::kobo::sync_protocol::compute_revision_hash(&item);
    let bundle = crate::kobo::sync_protocol::build_bundle(&item, &revision_hash, download_url);

    Ok(Json(serde_json::json!([bundle.book_metadata])))
}

// ── /v1/products/images/{image_id}/.../image.jpg ────────────────────

pub async fn product_image(
    State(state): State<AppState>,
    auth: KoboDeviceAuth,
    Path((_token, image_id, _width, _height, _is_greyscale)): Path<(
        String,
        Uuid,
        u32,
        u32,
        String,
    )>,
) -> Result<Response, ApiError> {
    serve_kobo_cover(state, auth, image_id).await
}

pub async fn product_image_quality(
    State(state): State<AppState>,
    auth: KoboDeviceAuth,
    Path((_token, image_id, _width, _height, _quality, _is_greyscale)): Path<(
        String,
        Uuid,
        u32,
        u32,
        String,
        String,
    )>,
) -> Result<Response, ApiError> {
    serve_kobo_cover(state, auth, image_id).await
}

async fn serve_kobo_cover(
    state: AppState,
    auth: KoboDeviceAuth,
    book_id: Uuid,
) -> Result<Response, ApiError> {
    if !crate::kobo::sync_enabled(&state) {
        return Err(crate::kobo::sync_disabled_error());
    }

    let pool = state.db_pool();

    let selection = KoboSyncSelectionRepository::find(pool, auth.user.id, book_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("book not in user's Kobo library".into()))?;
    if selection.selected_book_file_id.is_none() {
        return Err(ApiError::NotFound("selection is stale".into()));
    }

    let book = BookRepository::get_by_id(pool, book_id).await?;
    let cover_path = book
        .cover_path
        .as_deref()
        .ok_or_else(|| ApiError::NotFound("book has no cover".into()))?;
    let cover_bytes = state.storage().read(cover_path).await?;
    let content_type = cover_content_type(cover_path);

    Ok((
        [
            (CONTENT_TYPE, content_type.to_string()),
            (CONTENT_LENGTH, cover_bytes.len().to_string()),
            (CACHE_CONTROL, "public, max-age=86400".to_string()),
        ],
        cover_bytes,
    )
        .into_response())
}

fn cover_content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

// ── /v1/auth/device + /v1/auth/refresh — first-contact stubs ───────

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct AuthDeviceResponse {
    access_token: String,
    refresh_token: String,
    token_type: &'static str,
    tracking_id: String,
    user_key: String,
}

pub async fn auth_device(auth: KoboDeviceAuth) -> Json<AuthDeviceResponse> {
    Json(stub_auth_response(&auth))
}

pub async fn auth_refresh(auth: KoboDeviceAuth) -> Json<AuthDeviceResponse> {
    Json(stub_auth_response(&auth))
}

fn stub_auth_response(auth: &KoboDeviceAuth) -> AuthDeviceResponse {
    let device_id = auth.device.id;
    AuthDeviceResponse {
        access_token: format!("archivis-stub-access-{device_id}"),
        refresh_token: format!("archivis-stub-refresh-{device_id}"),
        token_type: "Bearer",
        tracking_id: device_id.to_string(),
        user_key: auth.user.id.to_string(),
    }
}

// ── /download/{book_id}/{book_file_id} ──────────────────────────────

pub async fn download(
    State(state): State<AppState>,
    auth: KoboDeviceAuth,
    Path((_token, book_id, book_file_id)): Path<(String, Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    if !crate::kobo::sync_enabled(&state) {
        return Err(crate::kobo::sync_disabled_error());
    }

    let pool = state.db_pool();

    // Authorization: either currently selected OR delivered + not tombstoned.
    let selection = KoboSyncSelectionRepository::find(pool, auth.user.id, book_id).await?;
    let currently_selected = selection
        .as_ref()
        .and_then(|s| s.selected_book_file_id)
        .is_some_and(|id| id == book_file_id);

    if !currently_selected {
        let ledger = KoboDeviceSyncItemRepository::find(pool, auth.device.id, book_id).await?;
        let delivered_match = ledger.as_ref().is_some_and(|row| {
            row.delivered_at.is_some()
                && row.removed_at.is_none()
                && row.book_file_id == Some(book_file_id)
        });
        if !delivered_match {
            return Err(ApiError::Forbidden);
        }
    }

    let file = BookFileRepository::get_by_id(pool, book_file_id).await?;
    if file.book_id != book_id {
        return Err(ApiError::NotFound(
            "file does not belong to that book".into(),
        ));
    }
    if file.format != BookFormat::Epub {
        return Err(ApiError::Validation(
            "Kobo download is only supported for EPUB sources".into(),
        ));
    }

    let book = BookRepository::get_by_id(pool, book_id).await?;
    let storage = state.storage();
    let data = storage.read(&file.storage_path).await?;

    let transformer = state
        .transformers()
        .lookup("kepub")
        .ok_or_else(|| ApiError::Internal("kepub transformer not registered".into()))?;

    let permits = state.transformers().permits();
    let _permit = permits
        .acquire_owned()
        .await
        .map_err(|e| ApiError::Internal(format!("transform permit closed: {e}")))?;

    let t = transformer.clone();
    let bytes = data.clone();
    let kepub_bytes = tokio::task::spawn_blocking(move || t.transform(&bytes))
        .await
        .map_err(|e| ApiError::Internal(format!("transform task: {e}")))?
        .map_err(|e| ApiError::Internal(format!("kepub transform failed: {e}")))?;

    let safe_name = sanitize_kepub_filename(&book.title);
    Ok((
        [
            (CONTENT_TYPE, "application/kepub+zip".to_string()),
            (
                CONTENT_DISPOSITION,
                format!("attachment; filename=\"{safe_name}\""),
            ),
            (CONTENT_LENGTH, kepub_bytes.len().to_string()),
        ],
        kepub_bytes,
    )
        .into_response())
}

fn sanitize_kepub_filename(title: &str) -> String {
    let mut stem = String::with_capacity(title.len());
    for ch in title.chars() {
        let allow = (ch.is_ascii_graphic() && ch != '"' && ch != '\\' && ch != '/')
            || ch == ' '
            || ch.is_alphanumeric();
        if allow {
            stem.push(ch);
        } else {
            stem.push('_');
        }
    }
    if stem.is_empty() {
        stem.push_str("book");
    }
    format!("{stem}.kepub.epub")
}

// ── store-side stubs ────────────────────────────────────────────────
//
// The device rewrites `storeapi.kobo.com/v1/...` URLs against
// `api_endpoint`, so any 404 on a store path during sync makes the
// firmware abort with "Sync failed". Return minimal JSON to keep it
// moving.

pub async fn stub_user_profile(auth: KoboDeviceAuth) -> Json<serde_json::Value> {
    let user_id = auth.user.id.to_string();
    Json(serde_json::json!({
        "UserId": user_id,
        "UserKey": user_id,
        "DisplayName": auth.user.username,
        "Email": "",
        "AcceptedKoboPrivacyPolicy": true,
        "AcceptedKoboTermsOfUse": true,
        "Country": "US",
        "PreferredLanguage": "en",
    }))
}

pub async fn stub_empty_object(_auth: KoboDeviceAuth) -> Json<serde_json::Value> {
    Json(serde_json::json!({}))
}

pub async fn stub_empty_array(_auth: KoboDeviceAuth) -> Json<serde_json::Value> {
    Json(serde_json::json!([]))
}

pub async fn stub_no_content(_auth: KoboDeviceAuth) -> StatusCode {
    StatusCode::NO_CONTENT
}

pub async fn stub_configuration(_auth: KoboDeviceAuth) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "Configuration": {} }))
}

pub async fn stub_analytics_gettests(_auth: KoboDeviceAuth) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "Tests": [] }))
}

pub async fn stub_store_fallback(_auth: KoboDeviceAuth) -> Json<serde_json::Value> {
    Json(serde_json::json!({}))
}

// ── helpers ─────────────────────────────────────────────────────────

fn require_public_base_url(state: &AppState) -> Result<PublicBaseUrl, ApiError> {
    state
        .config()
        .public_base_url
        .clone()
        .ok_or_else(|| ApiError::Conflict("public_base_url is not configured".into()))
}

fn join_kobo_path(base: &PublicBaseUrl, token: &str, suffix: &str) -> String {
    // Plain concat: `Url::join` percent-encodes `{`/`}`, but the device
    // requires literal `{ProductId}`/`{ImageId}` placeholders to substitute.
    let suffix = suffix.trim_start_matches('/');
    format!("{}/kobo/{token}/{suffix}", base.as_origin())
}
