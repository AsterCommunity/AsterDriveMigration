use super::*;

impl AsterDriveWriter<'_> {
    pub async fn write_user(&self, resolved: ResolvedUser, password_hash: &str) -> Result<i64> {
        let ResolvedUser {
            user,
            policy_group_id,
        } = resolved;
        let source_id = user.source_id;
        let target = aster_drive_schema::entities::user::ActiveModel {
            username: Set(user.username),
            email: Set(user.email),
            password_hash: Set(password_hash.to_string()),
            role: Set(match user.role {
                MigrationUserRole::Admin => UserRole::Admin,
                MigrationUserRole::User => UserRole::User,
            }),
            status: Set(match user.status {
                MigrationUserStatus::Active => UserStatus::Active,
                MigrationUserStatus::Disabled => UserStatus::Disabled,
            }),
            session_version: Set(1),
            email_verified_at: Set(
                (user.status == MigrationUserStatus::Active).then_some(user.created_at)
            ),
            pending_email: Set(None),
            storage_used: Set(user.storage_used),
            storage_quota: Set(user.storage_quota),
            policy_group_id: Set(policy_group_id),
            created_at: Set(user.created_at),
            updated_at: Set(user.updated_at),
            config: Set(user
                .config
                .map(|config| StoredUserConfig::from(config.to_string()))),
            must_change_password: Set(true),
            ..Default::default()
        }
        .insert(self.transaction)
        .await
        .wrap_err_with(|| format!("migrate Cloudreve user {source_id}"))?;

        aster_drive_schema::entities::user_profile::ActiveModel {
            user_id: Set(target.id),
            display_name: Set(Some(user.display_name)),
            wopi_user_info: Set(None),
            avatar_source: Set(match user.avatar_source {
                MigrationAvatarSource::None => AvatarSource::None,
                MigrationAvatarSource::Gravatar => AvatarSource::Gravatar,
                MigrationAvatarSource::Upload => AvatarSource::Upload,
            }),
            avatar_key: Set(user.avatar_key),
            avatar_version: Set(0),
            created_at: Set(user.created_at),
            updated_at: Set(user.updated_at),
        }
        .insert(self.transaction)
        .await
        .wrap_err_with(|| format!("create profile for Cloudreve user {source_id}"))?;
        Ok(target.id)
    }
}
