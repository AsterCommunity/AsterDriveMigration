use super::*;

impl AsterDriveWriter<'_> {
    pub async fn write_blob(&self, resolved: ResolvedBlob) -> Result<i64> {
        let ResolvedBlob { blob, policy_id } = resolved;
        let source_id = blob.source_id;
        let target = aster_drive_schema::entities::file_blob::ActiveModel {
            hash: Set(blob.opaque_key),
            size: Set(blob.size),
            policy_id: Set(policy_id),
            storage_path: Set(blob.storage_path),
            // Cloudreve thumbnails do not carry AsterDrive's processor/version cache contract.
            thumbnail_path: Set(None),
            thumbnail_processor: Set(None),
            thumbnail_version: Set(None),
            ref_count: Set(blob.reference_count),
            created_at: Set(blob.created_at),
            updated_at: Set(blob.updated_at),
            ..Default::default()
        }
        .insert(self.transaction)
        .await
        .wrap_err_with(|| format!("migrate Cloudreve entity {source_id}"))?;
        Ok(target.id)
    }
}
