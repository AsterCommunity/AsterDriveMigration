use super::*;

impl AsterDriveWriter<'_> {
    pub async fn write_policy(
        &self,
        policy: MigrationStoragePolicy,
        is_default: bool,
    ) -> Result<i64> {
        let source_id = policy.source_id;
        let mut options = Map::new();
        if matches!(
            policy.driver,
            MigrationStorageDriver::S3 | MigrationStorageDriver::TencentCos
        ) {
            options.insert("s3_path_style".to_string(), json!(policy.s3_path_style));
        }
        if policy.driver == MigrationStorageDriver::S3
            && let Some(region) = policy.s3_region
        {
            options.insert("s3_region".to_string(), json!(region));
        }
        if let Some(strategy) = policy.object_storage_upload_strategy {
            options.insert(
                "object_storage_upload_strategy".to_string(),
                json!(match strategy {
                    MigrationObjectStorageUploadStrategy::RelayStream => "relay_stream",
                    MigrationObjectStorageUploadStrategy::Presigned => "presigned",
                }),
            );
        }
        if let Some(strategy) = policy.object_storage_download_strategy {
            options.insert(
                "object_storage_download_strategy".to_string(),
                json!(match strategy {
                    MigrationObjectStorageDownloadStrategy::RelayStream => "relay_stream",
                    MigrationObjectStorageDownloadStrategy::Presigned => "presigned",
                }),
            );
        }
        if let Some(onedrive) = policy.onedrive {
            options.insert(
                "onedrive_cloud".to_string(),
                json!(match onedrive.cloud {
                    MigrationMicrosoftGraphCloud::Global => "global",
                    MigrationMicrosoftGraphCloud::China => "china",
                }),
            );
            options.insert(
                "onedrive_account_mode".to_string(),
                json!(match onedrive.account_mode {
                    MigrationOneDriveAccountMode::WorkOrSchool => "work_or_school",
                    MigrationOneDriveAccountMode::SharepointSite => "sharepoint_site",
                    MigrationOneDriveAccountMode::GroupDrive => "group_drive",
                }),
            );
            options.insert("onedrive_tenant".to_string(), json!(onedrive.tenant));
            if let Some(value) = onedrive.drive_id {
                options.insert("onedrive_drive_id".to_string(), json!(value));
            }
            if let Some(value) = onedrive.root_item_id {
                options.insert("onedrive_root_item_id".to_string(), json!(value));
            }
            if let Some(value) = onedrive.site_id {
                options.insert("onedrive_site_id".to_string(), json!(value));
            }
            if let Some(value) = onedrive.group_id {
                options.insert("onedrive_group_id".to_string(), json!(value));
            }
            options.insert(
                "provider_resumable_upload_strategy".to_string(),
                json!(match onedrive.upload_strategy {
                    MigrationProviderResumableUploadStrategy::ServerRelay => "server_relay",
                    MigrationProviderResumableUploadStrategy::FrontendDirect => "frontend_direct",
                }),
            );
            options.insert(
                "provider_download_strategy".to_string(),
                json!(match onedrive.download_strategy {
                    MigrationProviderDownloadStrategy::ServerRelay => "server_relay",
                    MigrationProviderDownloadStrategy::FrontendDirect => "frontend_direct",
                }),
            );
            options.insert(
                "provider_download_filename_mode".to_string(),
                json!(match onedrive.download_filename_mode {
                    MigrationProviderDownloadFilenameMode::ProviderNative => "provider_native",
                }),
            );
        }
        options.extend(policy.extensions);
        let target = aster_drive_schema::entities::storage_policy::ActiveModel {
            name: Set(policy.name),
            driver_type: Set(match policy.driver {
                MigrationStorageDriver::Local => DriverType::Local,
                MigrationStorageDriver::S3 => DriverType::S3,
                MigrationStorageDriver::TencentCos => DriverType::TencentCos,
                MigrationStorageDriver::OneDrive => DriverType::OneDrive,
            }),
            endpoint: Set(policy.endpoint),
            bucket: Set(policy.bucket),
            access_key: Set(policy.access_key),
            secret_key: Set(policy.secret_key),
            base_path: Set(policy.base_path),
            remote_node_id: Set(None),
            remote_storage_target_key: Set(None),
            max_file_size: Set(policy.max_file_size),
            allowed_types: Set(StoredStoragePolicyAllowedTypes::from(
                serde_json::to_string(&policy.allowed_types)
                    .wrap_err("serialize storage policy allowed types")?,
            )),
            options: Set(StoredStoragePolicyOptions::from(
                Value::Object(options).to_string(),
            )),
            is_default: Set(is_default),
            chunk_size: Set(policy.chunk_size),
            created_at: Set(policy.created_at),
            updated_at: Set(policy.updated_at),
            ..Default::default()
        }
        .insert(self.transaction)
        .await
        .wrap_err_with(|| format!("migrate storage policy {source_id}"))?;
        Ok(target.id)
    }

    pub async fn write_policy_group(&self, resolved: ResolvedPolicyGroup) -> Result<i64> {
        let ResolvedPolicyGroup { group, policy_id } = resolved;
        let source_id = group.source_id;
        let target = aster_drive_schema::entities::storage_policy_group::ActiveModel {
            name: Set(group.name),
            description: Set(group.description),
            is_enabled: Set(true),
            is_default: Set(false),
            created_at: Set(group.created_at),
            updated_at: Set(group.updated_at),
            ..Default::default()
        }
        .insert(self.transaction)
        .await
        .wrap_err_with(|| format!("migrate Cloudreve group {source_id}"))?;

        if let Some(target_policy_id) = policy_id {
            aster_drive_schema::entities::storage_policy_group_item::ActiveModel {
                group_id: Set(target.id),
                policy_id: Set(target_policy_id),
                priority: Set(0),
                min_file_size: Set(0),
                max_file_size: Set(0),
                created_at: Set(group.created_at),
                ..Default::default()
            }
            .insert(self.transaction)
            .await
            .wrap_err_with(|| format!("link Cloudreve group {source_id} storage policy"))?;
        }
        Ok(target.id)
    }
}
