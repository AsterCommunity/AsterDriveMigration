use super::*;

impl AsterDriveWriter<'_> {
    pub async fn write_file(&self, resolved: ResolvedFile) -> Result<WrittenFile> {
        let ResolvedFile {
            file,
            folder_id,
            owner_id,
            owner_username,
            primary_blob_id,
            primary_blob_size,
            historical_versions,
        } = resolved;
        let source_id = file.source_id;
        // Cloudreve v4 has no stable MIME column. Use the filename only as the MIME hint;
        // AsterDrive's ActiveModelBehavior delegates classification to the Forge crate.
        let mime_type = mime_guess::from_path(&file.name)
            .first_or_octet_stream()
            .essence_str()
            .to_string();
        let target = aster_drive_schema::entities::file::ActiveModel {
            name: Set(file.name),
            folder_id: Set(folder_id),
            team_id: Set(None),
            blob_id: Set(primary_blob_id),
            size: Set(primary_blob_size),
            owner_user_id: Set(Some(owner_id)),
            created_by_user_id: Set(Some(owner_id)),
            created_by_username: Set(owner_username),
            mime_type: Set(mime_type),
            created_at: Set(file.created_at),
            updated_at: Set(file.updated_at),
            deleted_at: Set(None),
            is_locked: Set(false),
            ..Default::default()
        }
        .insert(self.transaction)
        .await
        .wrap_err_with(|| format!("migrate Cloudreve file {source_id}"))?;

        let version_count = historical_versions.len();
        for (index, version) in historical_versions.into_iter().enumerate() {
            aster_drive_schema::entities::file_version::ActiveModel {
                file_id: Set(target.id),
                blob_id: Set(version.blob_id),
                version: Set(i32::try_from(index + 1).wrap_err("file version exceeds i32")?),
                size: Set(version.size),
                created_at: Set(version.created_at),
                ..Default::default()
            }
            .insert(self.transaction)
            .await
            .wrap_err_with(|| format!("migrate version {} for file {source_id}", index + 1))?;
        }
        Ok(WrittenFile {
            target_id: target.id,
            version_count,
        })
    }
}
