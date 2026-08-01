use super::*;

impl AsterDriveWriter<'_> {
    pub async fn write_folder(&self, resolved: ResolvedFolder) -> Result<i64> {
        let ResolvedFolder {
            folder,
            parent_id,
            owner_id,
            owner_username,
            policy_id,
        } = resolved;
        let source_id = folder.source_id;
        let target = aster_drive_schema::entities::folder::ActiveModel {
            name: Set(folder.name),
            parent_id: Set(parent_id),
            team_id: Set(None),
            owner_user_id: Set(Some(owner_id)),
            created_by_user_id: Set(Some(owner_id)),
            created_by_username: Set(owner_username),
            policy_id: Set(policy_id),
            created_at: Set(folder.created_at),
            updated_at: Set(folder.updated_at),
            deleted_at: Set(None),
            is_locked: Set(false),
            ..Default::default()
        }
        .insert(self.transaction)
        .await
        .wrap_err_with(|| format!("migrate Cloudreve folder {source_id}"))?;
        Ok(target.id)
    }
}
