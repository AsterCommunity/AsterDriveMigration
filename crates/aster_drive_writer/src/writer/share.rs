use super::*;

impl AsterDriveWriter<'_> {
    pub async fn write_share(&self, resolved: ResolvedShare) -> Result<i64> {
        let ResolvedShare {
            source_id,
            owner_id,
            target,
            password_hash,
            expires_at,
            max_downloads,
            download_count,
            view_count,
            created_at,
            updated_at,
        } = resolved;
        let (file_id, folder_id) = match target {
            ResolvedShareTarget::File { target_id } => (Some(target_id), None),
            ResolvedShareTarget::Folder { target_id } => (None, Some(target_id)),
        };
        let target = aster_drive_schema::entities::share::ActiveModel {
            token: Set(uuid::Uuid::new_v4().simple().to_string()),
            user_id: Set(owner_id),
            team_id: Set(None),
            file_id: Set(file_id),
            folder_id: Set(folder_id),
            password: Set(password_hash),
            expires_at: Set(expires_at),
            max_downloads: Set(max_downloads),
            download_count: Set(download_count),
            view_count: Set(view_count),
            created_at: Set(created_at),
            updated_at: Set(updated_at),
            ..Default::default()
        }
        .insert(self.transaction)
        .await
        .wrap_err_with(|| format!("migrate Cloudreve share {source_id}"))?;
        Ok(target.id)
    }
}
