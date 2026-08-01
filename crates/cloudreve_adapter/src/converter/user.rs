use super::*;

impl SourceConverter<CloudreveUserRecord> for CloudreveConverter {
    type Output = MigrationUser;
    type Error = color_eyre::Report;

    fn convert(
        &self,
        source: CloudreveUserRecord,
        _: &ConversionContext,
    ) -> Result<Conversion<Self::Output>> {
        let user = source.user;
        let role = if source.group.as_ref().is_some_and(|group| {
            group
                .permissions
                .first()
                .is_some_and(|permissions| permissions & 1 == 1)
        }) {
            MigrationUserRole::Admin
        } else {
            MigrationUserRole::User
        };
        let status = if user.status == "active" && user.deleted_at.is_none() {
            MigrationUserStatus::Active
        } else {
            MigrationUserStatus::Disabled
        };
        let avatar = user.avatar.filter(|avatar| !avatar.is_empty());
        let avatar_source = match avatar.as_deref() {
            None => MigrationAvatarSource::None,
            Some(value) if value.to_ascii_lowercase().contains("gravatar") => {
                MigrationAvatarSource::Gravatar
            }
            Some(_) => MigrationAvatarSource::Upload,
        };
        Ok(Conversion::Ready(MigrationUser {
            source_id: user.id,
            username: source.username,
            email: user.email,
            display_name: user.nick,
            role,
            status,
            storage_used: user.storage,
            storage_quota: source
                .group
                .and_then(|group| group.max_storage)
                .unwrap_or(0),
            policy_group_source_id: user.group_users,
            config: user.settings,
            avatar_source,
            avatar_key: avatar,
            created_at: target_time(user.created_at),
            updated_at: target_time(user.updated_at),
        }))
    }
}
