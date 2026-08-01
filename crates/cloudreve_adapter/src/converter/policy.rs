use super::*;

impl SourceConverter<CloudreveStoragePolicyRecord> for CloudreveConverter {
    type Output = MigrationStoragePolicy;
    type Error = color_eyre::Report;

    fn convert(
        &self,
        source: CloudreveStoragePolicyRecord,
        _: &ConversionContext,
    ) -> Result<Conversion<Self::Output>> {
        let policy = source.policy;
        if let Some(reason) = storage_policy_skip_reason(&policy) {
            return Ok(Conversion::Skipped(reason));
        }
        let policy_settings = settings(&policy.settings);
        let driver = match policy.r#type.as_str() {
            "local" => MigrationStorageDriver::Local,
            "s3" | "ks3" => MigrationStorageDriver::S3,
            "cos" => MigrationStorageDriver::TencentCos,
            "onedrive" => MigrationStorageDriver::OneDrive,
            unsupported => {
                return Ok(Conversion::Skipped(SkipReason {
                    code: "unsupported_storage_driver",
                    message: unsupported.to_string(),
                }));
            }
        };
        let base_path = match driver {
            MigrationStorageDriver::Local => source.local_root.ok_or_else(|| {
                color_eyre::eyre::eyre!(
                    "local Cloudreve policy {} has no resolved target root",
                    policy.id
                )
            })?,
            MigrationStorageDriver::S3
            | MigrationStorageDriver::TencentCos
            | MigrationStorageDriver::OneDrive => String::new(),
        };
        let allowed_types = match policy_settings.get("file_type") {
            None | Some(Value::Null) => &[][..],
            Some(Value::Array(values)) => values.as_slice(),
            Some(_) => {
                bail!(
                    "Cloudreve policy {} file_type setting must be an array",
                    policy.id
                );
            }
        };
        let allowed_types = allowed_types
            .iter()
            .map(|value| {
                value.as_str().map(str::to_string).ok_or_else(|| {
                    color_eyre::eyre::eyre!(
                        "Cloudreve policy {} file_type entries must be strings",
                        policy.id
                    )
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let mut chunk_size = policy_settings
            .get("chunk_size")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if chunk_size < 0 {
            bail!(
                "Cloudreve policy {} chunk_size must not be negative",
                policy.id
            );
        }
        let path_style = policy_settings
            .get("s3_path_style")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let s3_region = if driver == MigrationStorageDriver::S3 {
            storage_region_setting(&policy_settings).map_err(|message| {
                color_eyre::eyre::eyre!("Cloudreve policy {}: {message}", policy.id)
            })?
        } else {
            None
        };
        let (object_storage_upload_strategy, object_storage_download_strategy) = match driver {
            MigrationStorageDriver::Local | MigrationStorageDriver::OneDrive => (None, None),
            MigrationStorageDriver::S3 | MigrationStorageDriver::TencentCos => (
                Some(
                    if policy_settings
                        .get("relay")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        MigrationObjectStorageUploadStrategy::RelayStream
                    } else {
                        MigrationObjectStorageUploadStrategy::Presigned
                    },
                ),
                Some(
                    if policy_settings
                        .get("internal_proxy")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        MigrationObjectStorageDownloadStrategy::RelayStream
                    } else {
                        MigrationObjectStorageDownloadStrategy::Presigned
                    },
                ),
            ),
        };
        let onedrive = if driver == MigrationStorageDriver::OneDrive {
            if chunk_size == 0 {
                chunk_size = 50 * 1024 * 1024;
            }
            Some(one_drive_options(&policy).map_err(|message| {
                color_eyre::eyre::eyre!("Cloudreve OneDrive policy {}: {message}", policy.id)
            })?)
        } else {
            None
        };
        let extensions = BTreeMap::from([
            ("cloudreve_source".to_string(), policy_settings),
            ("cloudreve_policy_type".to_string(), json!(policy.r#type)),
        ]);
        let (endpoint, bucket, access_key, secret_key) =
            if driver == MigrationStorageDriver::OneDrive {
                (String::new(), String::new(), String::new(), String::new())
            } else {
                (
                    policy.server.unwrap_or_default(),
                    policy.bucket_name.unwrap_or_default(),
                    policy.access_key.unwrap_or_default(),
                    policy.secret_key.unwrap_or_default(),
                )
            };
        Ok(Conversion::Ready(MigrationStoragePolicy {
            source_id: policy.id,
            name: policy.name,
            driver,
            endpoint,
            bucket,
            access_key,
            secret_key,
            base_path,
            max_file_size: policy.max_size.unwrap_or(0),
            allowed_types,
            s3_path_style: path_style,
            s3_region,
            object_storage_upload_strategy,
            object_storage_download_strategy,
            onedrive,
            extensions,
            chunk_size,
            created_at: target_time(policy.created_at),
            updated_at: target_time(policy.updated_at),
        }))
    }
}

impl SourceConverter<CloudrevePolicyGroupRecord> for CloudreveConverter {
    type Output = MigrationPolicyGroup;
    type Error = color_eyre::Report;

    fn convert(
        &self,
        source: CloudrevePolicyGroupRecord,
        _: &ConversionContext,
    ) -> Result<Conversion<Self::Output>> {
        let group = source.group;
        Ok(Conversion::Ready(MigrationPolicyGroup {
            source_id: group.id,
            name: group.name,
            description: format!("Migrated from Cloudreve group {}", group.id),
            policy_source_id: group.storage_policy_id,
            created_at: target_time(group.created_at),
            updated_at: target_time(group.updated_at),
        }))
    }
}
