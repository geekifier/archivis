use std::path::PathBuf;

use archivis_db::{
    BookFileRepository, BookRepository, BookWithRelations, CandidateRepository,
    DuplicateRepository, IdentifierRepository,
};
use archivis_storage::StorageBackend;
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::types::{MergeError, MergeOptions, MergePreference};

/// Orchestrates the merging of duplicate books.
///
/// Given a primary and secondary book, the service absorbs all data from the
/// secondary into the primary and then deletes the secondary.
pub struct MergeService<S: StorageBackend> {
    db_pool: archivis_db::DbPool,
    #[allow(dead_code)]
    storage: S,
    data_dir: PathBuf,
}

impl<S: StorageBackend> MergeService<S> {
    pub fn new(db_pool: archivis_db::DbPool, storage: S, data_dir: PathBuf) -> Self {
        Self {
            db_pool,
            storage,
            data_dir,
        }
    }

    /// Merge `secondary_id` into `primary_id`.
    ///
    /// The primary book absorbs all data from the secondary, which is then
    /// deleted. Returns the updated primary `BookWithRelations`.
    pub async fn merge_books(
        &self,
        primary_id: Uuid,
        secondary_id: Uuid,
        options: MergeOptions,
    ) -> Result<BookWithRelations, MergeError> {
        // Guard: cannot merge a book with itself
        if primary_id == secondary_id {
            return Err(MergeError::SameBook);
        }

        // 1. Load both books with all relations
        let primary = BookRepository::get_with_relations(&self.db_pool, primary_id)
            .await
            .map_err(|e| match &e {
                archivis_core::errors::DbError::NotFound { .. } => {
                    MergeError::BookNotFound(primary_id)
                }
                _ => MergeError::Database(e),
            })?;

        let secondary = BookRepository::get_with_relations(&self.db_pool, secondary_id)
            .await
            .map_err(|e| match &e {
                archivis_core::errors::DbError::NotFound { .. } => {
                    MergeError::BookNotFound(secondary_id)
                }
                _ => MergeError::Database(e),
            })?;

        info!(
            primary_id = %primary_id,
            secondary_id = %secondary_id,
            preference = %options.prefer_metadata_from,
            "starting book merge"
        );

        // 2. Move files: reassign all book_files from secondary to primary
        let moved =
            BookFileRepository::reassign_to_book(&self.db_pool, secondary_id, primary_id).await?;
        debug!(count = moved, "moved files from secondary to primary");

        // 2b. Deduplicate files with identical hashes on the primary book
        self.deduplicate_files(primary_id).await?;

        // 3. Merge identifiers: copy from secondary, skip duplicates
        self.merge_identifiers(primary_id, &primary, &secondary)
            .await?;

        // 4. Merge authors
        self.merge_authors(primary_id, &primary, &secondary).await?;

        // 5. Merge series
        self.merge_series(primary_id, &primary, &secondary).await?;

        // 6. Merge tags
        self.merge_tags(primary_id, &primary, &secondary).await?;

        // 7. Merge metadata fields based on preference
        self.merge_metadata(primary_id, &primary, &secondary, &options)
            .await?;

        // 8. Handle cover transfer
        self.handle_cover_transfer(primary_id, &primary, &secondary)
            .await?;

        // 9. Update duplicate links referencing secondary
        self.update_duplicate_links(primary_id, secondary_id)
            .await?;

        // 10. Reassign or delete identification candidates from secondary
        self.handle_candidates(secondary_id).await?;

        // 11. Delete secondary book (cascades to join tables)
        BookRepository::delete(&self.db_pool, secondary_id).await?;

        info!(
            primary_id = %primary_id,
            secondary_id = %secondary_id,
            "merge complete — secondary book deleted"
        );

        // 12. Return updated primary
        let result = BookRepository::get_with_relations(&self.db_pool, primary_id).await?;
        Ok(result)
    }

    /// Remove duplicate files on a book (same hash), keeping the earliest entry.
    async fn deduplicate_files(&self, book_id: Uuid) -> Result<(), MergeError> {
        let files = BookFileRepository::get_by_book_id(&self.db_pool, book_id).await?;

        let mut seen: std::collections::HashMap<&str, &archivis_core::models::BookFile> =
            std::collections::HashMap::new();
        let mut to_delete = Vec::new();

        for file in &files {
            match seen.entry(&file.hash) {
                std::collections::hash_map::Entry::Occupied(existing) => {
                    // Keep the one with the earlier added_at
                    if file.added_at < existing.get().added_at {
                        to_delete.push(existing.get().id);
                        *existing.into_mut() = file;
                    } else {
                        to_delete.push(file.id);
                    }
                }
                std::collections::hash_map::Entry::Vacant(slot) => {
                    slot.insert(file);
                }
            }
        }

        for id in &to_delete {
            BookFileRepository::delete(&self.db_pool, *id).await?;
            debug!(file_id = %id, book_id = %book_id, "removed duplicate file entry");
        }

        if !to_delete.is_empty() {
            info!(
                book_id = %book_id,
                removed = to_delete.len(),
                "deduplicated files after merge"
            );
        }

        Ok(())
    }

    /// Copy identifiers from secondary that don't already exist on primary.
    async fn merge_identifiers(
        &self,
        primary_id: Uuid,
        primary: &BookWithRelations,
        secondary: &BookWithRelations,
    ) -> Result<(), MergeError> {
        for sec_ident in &secondary.identifiers {
            let already_exists = primary.identifiers.iter().any(|p| {
                p.identifier_type == sec_ident.identifier_type && p.value == sec_ident.value
            });

            if !already_exists {
                let new_ident = archivis_core::models::Identifier::new(
                    primary_id,
                    sec_ident.identifier_type,
                    &sec_ident.value,
                    sec_ident.source.clone(),
                    sec_ident.confidence,
                );
                IdentifierRepository::create(&self.db_pool, &new_ident).await?;
                debug!(
                    identifier_type = ?sec_ident.identifier_type,
                    value = %sec_ident.value,
                    "transferred identifier to primary"
                );
            }
        }

        // Delete remaining identifiers on secondary before book deletion
        IdentifierRepository::delete_by_book(&self.db_pool, secondary.book.id).await?;

        Ok(())
    }

    /// Add secondary's authors that primary doesn't have.
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    async fn merge_authors(
        &self,
        primary_id: Uuid,
        primary: &BookWithRelations,
        secondary: &BookWithRelations,
    ) -> Result<(), MergeError> {
        let primary_author_ids: std::collections::HashSet<Uuid> =
            primary.authors.iter().map(|a| a.author.id).collect();

        let next_position = primary
            .authors
            .iter()
            .map(|a| a.position)
            .max()
            .unwrap_or(0)
            + 1;

        for (i, sec_author) in secondary
            .authors
            .iter()
            .filter(|a| !primary_author_ids.contains(&a.author.id))
            .enumerate()
        {
            BookRepository::add_author(
                &self.db_pool,
                primary_id,
                sec_author.author.id,
                &sec_author.role,
                (next_position + i as i64) as i32,
            )
            .await?;
            debug!(author = %sec_author.author.name, "transferred author to primary");
        }

        Ok(())
    }

    /// Add secondary's series links that primary doesn't have.
    async fn merge_series(
        &self,
        primary_id: Uuid,
        primary: &BookWithRelations,
        secondary: &BookWithRelations,
    ) -> Result<(), MergeError> {
        let primary_series_ids: std::collections::HashSet<Uuid> =
            primary.series.iter().map(|s| s.series.id).collect();

        for sec_series in &secondary.series {
            if !primary_series_ids.contains(&sec_series.series.id) {
                BookRepository::add_series(
                    &self.db_pool,
                    primary_id,
                    sec_series.series.id,
                    sec_series.position,
                )
                .await?;
                debug!(series = %sec_series.series.name, "transferred series to primary");
            }
        }

        Ok(())
    }

    /// Add secondary's tags that primary doesn't have.
    async fn merge_tags(
        &self,
        primary_id: Uuid,
        primary: &BookWithRelations,
        secondary: &BookWithRelations,
    ) -> Result<(), MergeError> {
        let primary_tag_ids: std::collections::HashSet<Uuid> =
            primary.tags.iter().map(|t| t.id).collect();

        for sec_tag in &secondary.tags {
            if !primary_tag_ids.contains(&sec_tag.id) {
                BookRepository::add_tag(&self.db_pool, primary_id, sec_tag.id).await?;
                debug!(tag = %sec_tag.name, "transferred tag to primary");
            }
        }

        Ok(())
    }

    /// Merge metadata fields based on the configured preference.
    async fn merge_metadata(
        &self,
        primary_id: Uuid,
        primary: &BookWithRelations,
        secondary: &BookWithRelations,
        options: &MergeOptions,
    ) -> Result<(), MergeError> {
        let mut book = primary.book.clone();

        let use_secondary = match options.prefer_metadata_from {
            MergePreference::Primary => false,
            MergePreference::Secondary => true,
            MergePreference::HigherIngestQuality => {
                secondary.book.ingest_quality_score > primary.book.ingest_quality_score
            }
        };

        if use_secondary {
            // Use secondary's values when present
            if !secondary.book.title.is_empty() {
                book.set_title(secondary.book.title.clone());
            }
            if secondary.book.description.is_some() {
                book.description.clone_from(&secondary.book.description);
            }
            if secondary.book.language.is_some() {
                book.language.clone_from(&secondary.book.language);
            }
            if secondary.book.publisher_id.is_some() {
                book.publisher_id = secondary.book.publisher_id;
            }
            if secondary.book.publication_year.is_some() {
                book.publication_year = secondary.book.publication_year;
            }
            if secondary.book.page_count.is_some() {
                book.page_count = secondary.book.page_count;
            }
            if secondary.book.rating.is_some() {
                book.rating = secondary.book.rating;
            }
        } else {
            // Primary preference: fill in NULL/empty fields from secondary
            if book.description.is_none() {
                book.description.clone_from(&secondary.book.description);
            }
            if book.language.is_none() {
                book.language.clone_from(&secondary.book.language);
            }
            if book.publisher_id.is_none() {
                book.publisher_id = secondary.book.publisher_id;
            }
            if book.publication_year.is_none() {
                book.publication_year = secondary.book.publication_year;
            }
            if book.page_count.is_none() {
                book.page_count = secondary.book.page_count;
            }
            if book.rating.is_none() {
                book.rating = secondary.book.rating;
            }
        }

        // Always keep the highest import-time quality score.
        book.ingest_quality_score = book
            .ingest_quality_score
            .max(secondary.book.ingest_quality_score);

        BookRepository::update(&self.db_pool, &book).await?;
        debug!(primary_id = %primary_id, preference = %options.prefer_metadata_from, "merged metadata fields");

        Ok(())
    }

    /// If primary has no cover but secondary does, move it.
    async fn handle_cover_transfer(
        &self,
        primary_id: Uuid,
        primary: &BookWithRelations,
        secondary: &BookWithRelations,
    ) -> Result<(), MergeError> {
        if primary.book.cover_path.is_some() || secondary.book.cover_path.is_none() {
            return Ok(());
        }

        let secondary_id = secondary.book.id;

        // Update cover_path on primary
        let mut book = primary.book.clone();
        book.cover_path.clone_from(&secondary.book.cover_path);
        BookRepository::update(&self.db_pool, &book).await?;

        // Move thumbnail directory
        let src_dir = self.data_dir.join("covers").join(secondary_id.to_string());
        let dst_dir = self.data_dir.join("covers").join(primary_id.to_string());

        if src_dir.exists() {
            if let Err(e) = tokio::fs::rename(&src_dir, &dst_dir).await {
                // Fall back to copy + delete if rename fails (cross-device)
                warn!(error = %e, "rename failed, attempting copy");
                if let Err(copy_err) = copy_dir_recursive(&src_dir, &dst_dir).await {
                    warn!(error = %copy_err, "thumbnail copy failed");
                } else if let Err(rm_err) = tokio::fs::remove_dir_all(&src_dir).await {
                    warn!(error = %rm_err, "failed to remove old thumbnail dir");
                }
            }
            debug!(
                from = %src_dir.display(),
                to = %dst_dir.display(),
                "moved thumbnail directory"
            );
        }

        info!(primary_id = %primary_id, "transferred cover from secondary");
        Ok(())
    }

    /// Mark the link as Merged and redirect other links referencing secondary.
    async fn update_duplicate_links(
        &self,
        primary_id: Uuid,
        secondary_id: Uuid,
    ) -> Result<(), MergeError> {
        let links = DuplicateRepository::find_for_book(&self.db_pool, secondary_id).await?;

        for link in &links {
            if (link.book_id_a == primary_id && link.book_id_b == secondary_id)
                || (link.book_id_a == secondary_id && link.book_id_b == primary_id)
            {
                // This is the link being merged — mark as Merged
                DuplicateRepository::update_status(&self.db_pool, link.id, "merged").await?;
            } else {
                // This link references the secondary and another book —
                // mark as dismissed since the secondary is being deleted.
                DuplicateRepository::update_status(&self.db_pool, link.id, "dismissed").await?;
            }
        }

        Ok(())
    }

    /// Delete identification candidates from the secondary book.
    async fn handle_candidates(&self, secondary_id: Uuid) -> Result<(), MergeError> {
        CandidateRepository::delete_by_book(&self.db_pool, secondary_id).await?;
        Ok(())
    }
}

/// Recursively copy a directory.
async fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    tokio::fs::create_dir_all(dst).await?;
    let mut entries = tokio::fs::read_dir(src).await?;
    while let Some(entry) = entries.next_entry().await? {
        let ft = entry.file_type().await?;
        let dest_path = dst.join(entry.file_name());
        if ft.is_dir() {
            Box::pin(copy_dir_recursive(&entry.path(), &dest_path)).await?;
        } else {
            tokio::fs::copy(entry.path(), &dest_path).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_book_error() {
        let err = MergeError::SameBook;
        assert_eq!(err.to_string(), "cannot merge a book with itself");
    }

    #[test]
    fn book_not_found_error() {
        let id = Uuid::new_v4();
        let err = MergeError::BookNotFound(id);
        assert_eq!(err.to_string(), format!("book not found: {id}"));
    }
}
