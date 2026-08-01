use super::*;

fn now() -> chrono::DateTime<chrono::FixedOffset> {
    chrono::Utc::now().fixed_offset()
}

fn policy(policy_type: &str, settings: Value) -> cloudreve_schema::storage_policies::Model {
    cloudreve_schema::storage_policies::Model {
        id: 7,
        created_at: now(),
        updated_at: now(),
        deleted_at: None,
        name: "Primary".to_string(),
        r#type: policy_type.to_string(),
        server: Some("https://storage.example.test".to_string()),
        bucket_name: Some("bucket".to_string()),
        is_private: Some(true),
        access_key: Some("access".to_string()),
        secret_key: Some("secret".to_string()),
        max_size: Some(1_024),
        dir_name_rule: None,
        file_name_rule: None,
        settings: Some(settings),
        node_id: None,
    }
}

fn group(permissions: Vec<u8>) -> cloudreve_schema::groups::Model {
    cloudreve_schema::groups::Model {
        id: 8,
        created_at: now(),
        updated_at: now(),
        deleted_at: None,
        name: "Members".to_string(),
        max_storage: Some(4_096),
        speed_limit: None,
        permissions,
        settings: None,
        storage_policy_id: Some(7),
    }
}

fn user() -> cloudreve_schema::users::Model {
    cloudreve_schema::users::Model {
        id: 9,
        created_at: now(),
        updated_at: now(),
        deleted_at: None,
        email: "user@example.test".to_string(),
        nick: "User".to_string(),
        password: Some("legacy".to_string()),
        status: "active".to_string(),
        storage: 512,
        two_factor_secret: None,
        avatar: Some("https://gravatar.example/avatar".to_string()),
        settings: Some(json!({"theme": "dark"})),
        group_users: 8,
    }
}

fn folder(file_type: i64) -> cloudreve_schema::files::Model {
    cloudreve_schema::files::Model {
        id: 10,
        created_at: now(),
        updated_at: now(),
        r#type: file_type,
        name: "Documents".to_string(),
        size: 0,
        primary_entity: None,
        is_symbolic: false,
        props: None,
        file_children: Some(2),
        storage_policy_files: Some(7),
        owner_id: 9,
    }
}

fn entity(id: i64, entity_type: i64, size: i64) -> cloudreve_schema::entities::Model {
    cloudreve_schema::entities::Model {
        id,
        created_at: now() + chrono::TimeDelta::seconds(id),
        updated_at: now(),
        deleted_at: None,
        r#type: entity_type,
        source: format!("objects/{id}"),
        size,
        reference_count: 1,
        upload_session_id: None,
        recycle_options: None,
        storage_policy_entities: 7,
        created_by: Some(9),
    }
}

fn share(
    id: i64,
    target_id: Option<i64>,
    owner_id: Option<i64>,
) -> cloudreve_schema::shares::Model {
    cloudreve_schema::shares::Model {
        id,
        created_at: now(),
        updated_at: now(),
        deleted_at: None,
        password: Some("secret".to_string()),
        views: 5,
        downloads: 7,
        expires: Some(now() + chrono::TimeDelta::days(1)),
        remain_downloads: Some(3),
        props: Some(json!({"show_readme": true})),
        file_shares: target_id,
        user_shares: owner_id,
    }
}

fn metadata(id: i64, name: &str, value: &str) -> cloudreve_schema::metadata::Model {
    cloudreve_schema::metadata::Model {
        id,
        created_at: now(),
        updated_at: now(),
        deleted_at: None,
        name: name.to_string(),
        value: value.to_string(),
        is_public: true,
        file_id: 10,
    }
}

fn direct_link(id: i64) -> cloudreve_schema::direct_links::Model {
    cloudreve_schema::direct_links::Model {
        id,
        created_at: now(),
        updated_at: now(),
        deleted_at: None,
        name: "legacy-name.txt".to_string(),
        downloads: 7,
        speed: 1_024,
        file_id: 10,
    }
}

fn ready<T>(conversion: Conversion<T>) -> T {
    conversion.into_ready().expect("ready conversion")
}

#[test]
fn converts_every_supported_storage_driver() -> Result<()> {
    for (source, expected) in [
        ("local", MigrationStorageDriver::Local),
        ("s3", MigrationStorageDriver::S3),
        ("ks3", MigrationStorageDriver::S3),
        ("cos", MigrationStorageDriver::TencentCos),
        ("onedrive", MigrationStorageDriver::OneDrive),
    ] {
        let mut source_policy = policy(
            source,
            json!({
                "chunk_size": 128,
                "file_type": ["jpg", "png"],
                "s3_path_style": false
            }),
        );
        if source == "cos" {
            source_policy.server = Some("https://bucket.cos.ap-guangzhou.myqcloud.com".to_string());
        } else if source == "onedrive" {
            source_policy.server = Some("https://graph.microsoft.com/v1.0".to_string());
        }
        let converted = ready(CloudreveConverter.convert(
            CloudreveStoragePolicyRecord {
                policy: source_policy,
                local_root: (source == "local").then(|| "/source".to_string()),
            },
            &ConversionContext,
        )?);
        assert_eq!(converted.driver, expected);
        assert_eq!(converted.chunk_size, 128);
        assert_eq!(converted.allowed_types, ["jpg", "png"]);
        assert!(!converted.s3_path_style);
        assert_eq!(converted.s3_region, None);
        if matches!(source, "local" | "onedrive") {
            assert_eq!(converted.object_storage_upload_strategy, None);
            assert_eq!(converted.object_storage_download_strategy, None);
        } else {
            assert_eq!(
                converted.object_storage_upload_strategy,
                Some(MigrationObjectStorageUploadStrategy::Presigned)
            );
            assert_eq!(
                converted.object_storage_download_strategy,
                Some(MigrationObjectStorageDownloadStrategy::Presigned)
            );
        }
        assert_eq!(converted.onedrive.is_some(), source == "onedrive");
        assert_eq!(converted.extensions["cloudreve_policy_type"], source);
        assert_eq!(
            converted.base_path,
            if source == "local" { "/source" } else { "" }
        );
    }
    Ok(())
}

#[test]
fn treats_missing_and_null_file_type_as_cloudreve_empty_default() -> Result<()> {
    for policy_settings in [json!({}), json!({"file_type": null})] {
        let converted = ready(CloudreveConverter.convert(
            CloudreveStoragePolicyRecord {
                policy: policy("local", policy_settings),
                local_root: Some("/source".to_string()),
            },
            &ConversionContext,
        )?);
        assert!(converted.allowed_types.is_empty());
    }
    Ok(())
}

#[test]
fn preserves_s3_and_ks3_regions_with_cloudreve_alias_precedence() -> Result<()> {
    for (policy_type, policy_settings, expected_region) in [
        (
            "s3",
            json!({"region": "  ap-southeast-1  ", "s3_region": "ignored"}),
            "ap-southeast-1",
        ),
        (
            "ks3",
            json!({"region": "", "s3_region": "cn-beijing"}),
            "cn-beijing",
        ),
    ] {
        let converted = ready(CloudreveConverter.convert(
            CloudreveStoragePolicyRecord {
                policy: policy(policy_type, policy_settings),
                local_root: None,
            },
            &ConversionContext,
        )?);
        assert_eq!(converted.driver, MigrationStorageDriver::S3);
        assert_eq!(converted.s3_region.as_deref(), Some(expected_region));
    }
    Ok(())
}

#[test]
fn validates_s3_region_full_and_boundary_values() -> Result<()> {
    for (settings, expected) in [
        (json!({}), None),
        (json!({"region": null}), None),
        (json!({"region": "   "}), None),
        (json!({"region": " auto "}), Some("auto".to_string())),
        (
            json!({"region": "us-east-1"}),
            Some("us-east-1".to_string()),
        ),
        (json!({"region": "r".repeat(128)}), Some("r".repeat(128))),
    ] {
        let region = storage_region_setting(&settings)
            .map_err(|message| color_eyre::eyre::eyre!(message))?;
        assert_eq!(region, expected);
    }

    for settings in [
        json!({"region": 1}),
        json!({"region": "us east 1"}),
        json!({"region": "us-east-1/invalid"}),
        json!({"region": "华东"}),
        json!({"region": "r".repeat(129)}),
        json!({"region": "us-\neast-1"}),
    ] {
        assert!(storage_region_setting(&settings).is_err());
    }
    Ok(())
}

#[test]
fn skips_s3_and_ks3_regions_rejected_by_asterdrive() -> Result<()> {
    for policy_type in ["s3", "ks3"] {
        let conversion = CloudreveConverter.convert(
            CloudreveStoragePolicyRecord {
                policy: policy(policy_type, json!({"region": "us-east-1/invalid"})),
                local_root: None,
            },
            &ConversionContext,
        )?;
        let Conversion::Skipped(reason) = conversion else {
            panic!("expected invalid region to skip policy");
        };
        assert_eq!(reason.code, "unsupported_storage_configuration");
        assert!(reason.message.contains("settings.region"));
        assert!(reason.message.contains(policy_type));
    }
    Ok(())
}

#[test]
fn converts_onedrive_cloud_locations_and_strips_legacy_credentials() -> Result<()> {
    for (endpoint, driver_resource, expected_cloud, expected_mode, drive, site, group) in [
        (
            "https://graph.microsoft.com/v1.0",
            "",
            MigrationMicrosoftGraphCloud::Global,
            MigrationOneDriveAccountMode::WorkOrSchool,
            None,
            None,
            None,
        ),
        (
            "https://microsoftgraph.chinacloudapi.cn/v1.0",
            "me/drive",
            MigrationMicrosoftGraphCloud::China,
            MigrationOneDriveAccountMode::WorkOrSchool,
            None,
            None,
            None,
        ),
        (
            "https://graph.microsoft.com/v1.0",
            "drives/drive-id",
            MigrationMicrosoftGraphCloud::Global,
            MigrationOneDriveAccountMode::WorkOrSchool,
            Some("drive-id"),
            None,
            None,
        ),
        (
            "https://graph.microsoft.com/v1.0",
            "sites/site-id/drive",
            MigrationMicrosoftGraphCloud::Global,
            MigrationOneDriveAccountMode::SharepointSite,
            None,
            Some("site-id"),
            None,
        ),
        (
            "https://graph.microsoft.com/v1.0",
            "sites/site-id/drives/drive-id",
            MigrationMicrosoftGraphCloud::Global,
            MigrationOneDriveAccountMode::SharepointSite,
            Some("drive-id"),
            Some("site-id"),
            None,
        ),
        (
            "https://graph.microsoft.com/v1.0",
            "groups/group-id/drive",
            MigrationMicrosoftGraphCloud::Global,
            MigrationOneDriveAccountMode::GroupDrive,
            None,
            None,
            Some("group-id"),
        ),
        (
            "https://graph.microsoft.com/v1.0",
            "/groups/group-id/drives/drive-id/",
            MigrationMicrosoftGraphCloud::Global,
            MigrationOneDriveAccountMode::GroupDrive,
            Some("drive-id"),
            None,
            Some("group-id"),
        ),
    ] {
        let mut source = policy(
            "onedrive",
            json!({
                "od_driver": driver_resource,
                "relay": true,
                "internal_proxy": false,
                "chunk_size": 0
            }),
        );
        source.server = Some(endpoint.to_string());
        source.bucket_name = Some("legacy-client-id".to_string());
        source.access_key = Some("legacy-refresh-token".to_string());
        source.secret_key = Some("legacy-client-secret".to_string());
        let converted = ready(CloudreveConverter.convert(
            CloudreveStoragePolicyRecord {
                policy: source,
                local_root: None,
            },
            &ConversionContext,
        )?);
        let onedrive = converted.onedrive.expect("OneDrive options");
        assert_eq!(onedrive.cloud, expected_cloud);
        assert_eq!(onedrive.account_mode, expected_mode);
        assert_eq!(onedrive.drive_id.as_deref(), drive);
        assert_eq!(onedrive.site_id.as_deref(), site);
        assert_eq!(onedrive.group_id.as_deref(), group);
        assert_eq!(onedrive.root_item_id.as_deref(), Some("root"));
        assert_eq!(onedrive.tenant, "common");
        assert_eq!(
            onedrive.upload_strategy,
            MigrationProviderResumableUploadStrategy::ServerRelay
        );
        assert_eq!(
            onedrive.download_strategy,
            MigrationProviderDownloadStrategy::FrontendDirect
        );
        assert_eq!(converted.chunk_size, 50 * 1024 * 1024);
        assert!(converted.endpoint.is_empty());
        assert!(converted.bucket.is_empty());
        assert!(converted.access_key.is_empty());
        assert!(converted.secret_key.is_empty());
    }
    Ok(())
}

#[test]
fn preserves_onedrive_relay_topology_for_every_boolean_combination() -> Result<()> {
    for (relay, internal_proxy, expected_upload, expected_download) in [
        (
            false,
            false,
            MigrationProviderResumableUploadStrategy::FrontendDirect,
            MigrationProviderDownloadStrategy::FrontendDirect,
        ),
        (
            true,
            false,
            MigrationProviderResumableUploadStrategy::ServerRelay,
            MigrationProviderDownloadStrategy::FrontendDirect,
        ),
        (
            false,
            true,
            MigrationProviderResumableUploadStrategy::FrontendDirect,
            MigrationProviderDownloadStrategy::ServerRelay,
        ),
        (
            true,
            true,
            MigrationProviderResumableUploadStrategy::ServerRelay,
            MigrationProviderDownloadStrategy::ServerRelay,
        ),
    ] {
        let mut source = policy(
            "onedrive",
            json!({"relay": relay, "internal_proxy": internal_proxy}),
        );
        source.server = Some("https://graph.microsoft.com/v1.0".to_string());
        let converted = ready(CloudreveConverter.convert(
            CloudreveStoragePolicyRecord {
                policy: source,
                local_root: None,
            },
            &ConversionContext,
        )?);
        let onedrive = converted.onedrive.expect("OneDrive options");
        assert_eq!(onedrive.upload_strategy, expected_upload);
        assert_eq!(onedrive.download_strategy, expected_download);
    }
    Ok(())
}

#[test]
fn skips_onedrive_endpoints_and_driver_resources_outside_ad_contract() -> Result<()> {
    for (endpoint, driver_resource, expected) in [
        ("", "me/drive", "endpoint is required"),
        ("not a URL", "me/drive", "invalid Microsoft Graph endpoint"),
        ("http://graph.microsoft.com/v1.0", "me/drive", "https"),
        (
            "https://graph.example.test/v1.0",
            "me/drive",
            "endpoint host",
        ),
        (
            "https://graph.microsoft.com/v1.0",
            "users/user-id/drive",
            "od_driver",
        ),
        (
            "https://graph.microsoft.com/v1.0",
            "drives//drive-id",
            "od_driver",
        ),
        (
            "https://graph.microsoft.com/v1.0",
            "sites/site-id//drive",
            "od_driver",
        ),
        (
            "https://graph.microsoft.com/v1.0",
            "groups//drives/drive-id",
            "od_driver",
        ),
    ] {
        let mut source = policy("onedrive", json!({"od_driver": driver_resource}));
        source.server = Some(endpoint.to_string());
        let conversion = CloudreveConverter.convert(
            CloudreveStoragePolicyRecord {
                policy: source,
                local_root: None,
            },
            &ConversionContext,
        )?;
        let Conversion::Skipped(reason) = conversion else {
            panic!("expected unsupported OneDrive policy");
        };
        assert_eq!(reason.code, "unsupported_storage_configuration");
        assert!(reason.message.contains(expected));
    }
    Ok(())
}

#[test]
fn skips_onedrive_non_string_driver_resource() -> Result<()> {
    let mut source = policy("onedrive", json!({"od_driver": 42}));
    source.server = Some("https://graph.microsoft.com/v1.0".to_string());
    let conversion = CloudreveConverter.convert(
        CloudreveStoragePolicyRecord {
            policy: source,
            local_root: None,
        },
        &ConversionContext,
    )?;
    let Conversion::Skipped(reason) = conversion else {
        panic!("expected unsupported OneDrive policy");
    };
    assert_eq!(reason.code, "unsupported_storage_configuration");
    assert!(reason.message.contains("must be a string"));
    Ok(())
}

#[test]
fn preserves_cloudreve_object_storage_upload_and_download_topology() -> Result<()> {
    for (relay, internal_proxy, expected_upload, expected_download) in [
        (
            false,
            false,
            MigrationObjectStorageUploadStrategy::Presigned,
            MigrationObjectStorageDownloadStrategy::Presigned,
        ),
        (
            true,
            false,
            MigrationObjectStorageUploadStrategy::RelayStream,
            MigrationObjectStorageDownloadStrategy::Presigned,
        ),
        (
            false,
            true,
            MigrationObjectStorageUploadStrategy::Presigned,
            MigrationObjectStorageDownloadStrategy::RelayStream,
        ),
        (
            true,
            true,
            MigrationObjectStorageUploadStrategy::RelayStream,
            MigrationObjectStorageDownloadStrategy::RelayStream,
        ),
    ] {
        for source in ["s3", "cos"] {
            let mut source_policy = policy(
                source,
                json!({
                    "relay": relay,
                    "internal_proxy": internal_proxy
                }),
            );
            if source == "cos" {
                source_policy.server =
                    Some("https://bucket.cos.ap-guangzhou.myqcloud.com".to_string());
            }
            let converted = ready(CloudreveConverter.convert(
                CloudreveStoragePolicyRecord {
                    policy: source_policy,
                    local_root: None,
                },
                &ConversionContext,
            )?);
            assert_eq!(
                converted.object_storage_upload_strategy,
                Some(expected_upload)
            );
            assert_eq!(
                converted.object_storage_download_strategy,
                Some(expected_download)
            );
        }
    }
    Ok(())
}

#[test]
fn skips_unsupported_or_encrypted_storage_policies() -> Result<()> {
    for source in ["oss", "obs", "qiniu", "upyun", "remote", "unknown"] {
        let converted = CloudreveConverter.convert(
            CloudreveStoragePolicyRecord {
                policy: policy(source, json!({})),
                local_root: None,
            },
            &ConversionContext,
        )?;
        let Conversion::Skipped(reason) = converted else {
            panic!("expected skipped conversion");
        };
        assert_eq!(reason.code, "unsupported_storage_driver");
        assert!(!reason.message.is_empty());
    }
    Ok(())
}

#[test]
fn storage_encryption_takes_precedence_for_every_policy_type() -> Result<()> {
    for source in ["local", "s3", "ks3", "cos", "onedrive", "oss", "obs"] {
        let mut encrypted = policy(source, json!({"encryption": true}));
        if source == "cos" {
            encrypted.server = Some("https://bucket.cos.ap-guangzhou.myqcloud.com".to_string());
        }
        let conversion = CloudreveConverter.convert(
            CloudreveStoragePolicyRecord {
                policy: encrypted,
                local_root: (source == "local").then(|| "/source".to_string()),
            },
            &ConversionContext,
        )?;
        let Conversion::Skipped(reason) = conversion else {
            panic!("expected encrypted policy to be skipped");
        };
        assert_eq!(reason.code, "cloudreve_storage_encryption");
        assert!(reason.message.contains(source));
    }
    Ok(())
}

#[test]
fn accepts_only_standard_tencent_cos_policy_endpoints() -> Result<()> {
    for (endpoint, bucket) in [
        ("https://bucket.cos.ap-guangzhou.myqcloud.com", "bucket"),
        ("http://bucket.cos.ap-shanghai.myqcloud.com", "bucket"),
        ("https://bucket.cos.ap-guangzhou.myqcloud.com/", "Bucket"),
    ] {
        let mut valid = policy("cos", json!({}));
        valid.server = Some(endpoint.to_string());
        valid.bucket_name = Some(bucket.to_string());
        assert!(
            CloudreveConverter
                .convert(
                    CloudreveStoragePolicyRecord {
                        policy: valid,
                        local_root: None,
                    },
                    &ConversionContext,
                )?
                .into_ready()
                .is_some()
        );
    }

    for (endpoint, bucket, expected_message) in [
        ("https://www.example.test", "bucket", "myqcloud.com host"),
        (
            "https://bucket.cos.ap-guangzhou.myqcloud.com.evil.test",
            "bucket",
            "myqcloud.com host",
        ),
        (
            "https://other.cos.ap-guangzhou.myqcloud.com",
            "bucket",
            "does not match bucket",
        ),
        ("not a URL", "bucket", "not a valid URL"),
        (
            "https://bucket.cos..myqcloud.com",
            "bucket",
            "invalid region",
        ),
        (
            "https://bucket.cos.ap.guangzhou.myqcloud.com",
            "bucket",
            "invalid region",
        ),
        (
            "ftp://bucket.cos.ap-guangzhou.myqcloud.com",
            "bucket",
            "http or https",
        ),
        (
            "https://bucket.cos.ap-guangzhou.myqcloud.com",
            "",
            "bucket is required",
        ),
    ] {
        let mut invalid = policy("cos", json!({}));
        invalid.server = Some(endpoint.to_string());
        invalid.bucket_name = Some(bucket.to_string());
        let conversion = CloudreveConverter.convert(
            CloudreveStoragePolicyRecord {
                policy: invalid,
                local_root: None,
            },
            &ConversionContext,
        )?;
        let Conversion::Skipped(reason) = conversion else {
            panic!("expected incompatible COS policy to be skipped");
        };
        assert_eq!(reason.code, "unsupported_storage_configuration");
        assert!(
            reason.message.contains(expected_message),
            "expected {expected_message:?} in {:?}",
            reason.message
        );
    }

    let mut missing_endpoint = policy("cos", json!({}));
    missing_endpoint.server = None;
    let conversion = CloudreveConverter.convert(
        CloudreveStoragePolicyRecord {
            policy: missing_endpoint,
            local_root: None,
        },
        &ConversionContext,
    )?;
    assert!(matches!(
        conversion,
        Conversion::Skipped(reason)
            if reason.code == "unsupported_storage_configuration"
                && reason.message.contains("endpoint is required")
    ));
    Ok(())
}

#[test]
fn skips_object_storage_policies_missing_runtime_credentials() -> Result<()> {
    for policy_type in ["s3", "ks3", "cos"] {
        for (field, expected_message) in [
            ("bucket", "bucket is required"),
            ("access_key", "access key is required"),
            ("secret_key", "secret key is required"),
        ] {
            let mut invalid = policy(policy_type, json!({}));
            if policy_type == "cos" {
                invalid.server = Some("https://bucket.cos.ap-guangzhou.myqcloud.com".to_string());
            }
            match field {
                "bucket" => invalid.bucket_name = Some(" ".to_string()),
                "access_key" => invalid.access_key = None,
                "secret_key" => invalid.secret_key = Some(String::new()),
                _ => panic!("unexpected test field"),
            }
            let conversion = CloudreveConverter.convert(
                CloudreveStoragePolicyRecord {
                    policy: invalid,
                    local_root: None,
                },
                &ConversionContext,
            )?;
            let Conversion::Skipped(reason) = conversion else {
                panic!("expected incomplete object-storage policy to be skipped");
            };
            assert_eq!(reason.code, "unsupported_storage_configuration");
            assert!(reason.message.contains(expected_message));
        }
    }
    Ok(())
}

#[test]
fn unsupported_policy_reasons_never_include_credentials() {
    for policy_type in ["oss", "obs"] {
        let mut unsupported = policy(policy_type, json!({}));
        unsupported.access_key = Some("sensitive-access".to_string());
        unsupported.secret_key = Some("sensitive-secret".to_string());
        let reason = storage_policy_skip_reason(&unsupported).expect("unsupported policy");
        assert!(!reason.message.contains("sensitive-access"));
        assert!(!reason.message.contains("sensitive-secret"));
    }
}

#[test]
fn rejects_invalid_policy_boundaries() {
    let missing_root = CloudreveConverter.convert(
        CloudreveStoragePolicyRecord {
            policy: policy("local", json!({})),
            local_root: None,
        },
        &ConversionContext,
    );
    assert!(format!("{:?}", missing_root.unwrap_err()).contains("no resolved target root"));

    let invalid_types = CloudreveConverter.convert(
        CloudreveStoragePolicyRecord {
            policy: policy("s3", json!({"file_type": "jpg"})),
            local_root: None,
        },
        &ConversionContext,
    );
    assert!(format!("{:?}", invalid_types.unwrap_err()).contains("must be an array"));

    let negative_chunk = CloudreveConverter.convert(
        CloudreveStoragePolicyRecord {
            policy: policy("s3", json!({"chunk_size": -1})),
            local_root: None,
        },
        &ConversionContext,
    );
    assert!(format!("{:?}", negative_chunk.unwrap_err()).contains("must not be negative"));
}

#[test]
fn converts_group_user_and_folder_records() -> Result<()> {
    let converted_group = ready(CloudreveConverter.convert(
        CloudrevePolicyGroupRecord {
            group: group(vec![1]),
        },
        &ConversionContext,
    )?);
    assert_eq!(converted_group.source_id, 8);
    assert_eq!(converted_group.policy_source_id, Some(7));

    let converted_user = ready(CloudreveConverter.convert(
        CloudreveUserRecord {
            user: user(),
            group: Some(group(vec![1])),
            username: "user".to_string(),
        },
        &ConversionContext,
    )?);
    assert_eq!(converted_user.role, MigrationUserRole::Admin);
    assert_eq!(converted_user.status, MigrationUserStatus::Active);
    assert_eq!(converted_user.storage_quota, 4_096);
    assert_eq!(
        converted_user.avatar_source,
        MigrationAvatarSource::Gravatar
    );
    assert_eq!(converted_user.config, Some(json!({"theme": "dark"})));

    let converted_folder = ready(CloudreveConverter.convert(
        CloudreveFolderRecord { folder: folder(1) },
        &ConversionContext,
    )?);
    assert_eq!(converted_folder.parent_source_id, Some(2));
    assert_eq!(converted_folder.owner_source_id, 9);
    assert_eq!(converted_folder.policy_source_id, Some(7));
    Ok(())
}

#[test]
fn handles_user_and_folder_boundaries() -> Result<()> {
    let mut disabled = user();
    disabled.status = "inactive".to_string();
    disabled.avatar = Some("/avatars/custom.png".to_string());
    let converted = ready(CloudreveConverter.convert(
        CloudreveUserRecord {
            user: disabled,
            group: None,
            username: "disabled".to_string(),
        },
        &ConversionContext,
    )?);
    assert_eq!(converted.role, MigrationUserRole::User);
    assert_eq!(converted.status, MigrationUserStatus::Disabled);
    assert_eq!(converted.storage_quota, 0);
    assert_eq!(converted.avatar_source, MigrationAvatarSource::Upload);

    let conversion = CloudreveConverter.convert(
        CloudreveFolderRecord { folder: folder(0) },
        &ConversionContext,
    )?;
    let Conversion::Skipped(reason) = conversion else {
        panic!("expected non-folder row to be skipped");
    };
    assert_eq!(reason.code, "not_a_folder");
    Ok(())
}

#[test]
fn converts_blob_and_orders_file_versions() -> Result<()> {
    let blob = ready(CloudreveConverter.convert(
        CloudreveBlobRecord {
            entity: entity(12, 0, 512),
            reference_count: 3,
            local_root: None,
        },
        &ConversionContext,
    )?);
    assert_eq!(blob.source_id, 12);
    assert_eq!(blob.opaque_key, "cloudreve-000000000000000c");
    assert_eq!(blob.storage_path, "objects/12");
    assert_eq!(blob.reference_count, 3);

    let mut file = folder(0);
    file.id = 20;
    file.name = "archive.tar.gz".to_string();
    file.size = 512;
    file.primary_entity = Some(12);
    let converted = ready(CloudreveConverter.convert(
        CloudreveFileRecord {
            file,
            entities: vec![entity(12, 0, 512), entity(11, 0, 256)],
        },
        &ConversionContext,
    )?);
    assert_eq!(converted.preferred_blob_source_id, Some(12));
    assert_eq!(
        converted
            .versions
            .iter()
            .map(|version| version.blob_source_id)
            .collect::<Vec<_>>(),
        [11, 12]
    );
    Ok(())
}

#[test]
fn normalizes_local_blob_paths_relative_to_the_configured_root() -> Result<()> {
    for (root, source, expected) in [
        ("/srv/cloudreve", "uploads/object.bin", "uploads/object.bin"),
        (
            "/srv/cloudreve",
            "/srv/cloudreve/data/uploads/object.bin",
            "data/uploads/object.bin",
        ),
        (
            "C:\\Cloudreve",
            "c:\\cloudreve\\data\\object.bin",
            "data/object.bin",
        ),
        (
            "\\\\server\\share\\cloudreve",
            "\\\\SERVER\\SHARE\\Cloudreve\\uploads\\object.bin",
            "uploads/object.bin",
        ),
    ] {
        let mut local_entity = entity(12, 0, 512);
        local_entity.source = source.to_string();
        let blob = ready(CloudreveConverter.convert(
            CloudreveBlobRecord {
                entity: local_entity,
                reference_count: 1,
                local_root: Some(root.to_string()),
            },
            &ConversionContext,
        )?);
        assert_eq!(blob.storage_path, expected);
    }
    Ok(())
}

#[test]
fn rejects_unsafe_or_unrelated_local_blob_paths() {
    for (root, source, expected_message) in [
        (
            "/srv/cloudreve",
            "/srv/other/object.bin",
            "outside the configured local root",
        ),
        ("C:/cloudreve", "D:/cloudreve/object.bin", "different root"),
        (
            "/srv/cloudreve",
            "uploads/../secret.bin",
            "parent-directory segments",
        ),
        (
            "/srv/cloudreve",
            "/srv/cloudreve",
            "resolves to the local storage root",
        ),
    ] {
        let mut local_entity = entity(12, 0, 512);
        local_entity.source = source.to_string();
        let error = CloudreveConverter
            .convert(
                CloudreveBlobRecord {
                    entity: local_entity,
                    reference_count: 1,
                    local_root: Some(root.to_string()),
                },
                &ConversionContext,
            )
            .unwrap_err();
        assert!(format!("{error:?}").contains(expected_message));
    }
}

#[test]
fn handles_blob_and_file_boundaries() -> Result<()> {
    let conversion = CloudreveConverter.convert(
        CloudreveBlobRecord {
            entity: entity(12, 1, 512),
            reference_count: 1,
            local_root: None,
        },
        &ConversionContext,
    )?;
    assert!(matches!(conversion, Conversion::Skipped(reason) if reason.code == "not_a_blob"));

    for recycle_options in [
        Some(json!({"unlink_only": true})),
        Some(json!({"encrypt_metadata": null})),
    ] {
        let mut plain_blob = entity(12, 0, 512);
        plain_blob.recycle_options = recycle_options;
        assert!(
            CloudreveConverter
                .convert(
                    CloudreveBlobRecord {
                        entity: plain_blob,
                        reference_count: 1,
                        local_root: None,
                    },
                    &ConversionContext,
                )?
                .into_ready()
                .is_some()
        );
    }

    let mut encrypted_blob = entity(12, 0, 512);
    encrypted_blob.recycle_options = Some(json!({
        "encrypt_metadata": {
            "algorithm": "aes256ctr",
            "key": "sensitive-key",
            "iv": "sensitive-iv"
        }
    }));
    let conversion = CloudreveConverter.convert(
        CloudreveBlobRecord {
            entity: encrypted_blob,
            reference_count: 1,
            local_root: None,
        },
        &ConversionContext,
    )?;
    let Conversion::Skipped(reason) = conversion else {
        panic!("expected encrypted entity to be skipped");
    };
    assert_eq!(reason.code, "cloudreve_encrypted_entity");
    assert_eq!(
        reason.message,
        "Cloudreve encrypted entity is not supported"
    );
    assert!(!reason.message.contains("sensitive"));

    let mut invalid_blob = entity(12, 0, 512);
    invalid_blob.source.clear();
    assert!(
        CloudreveConverter
            .convert(
                CloudreveBlobRecord {
                    entity: invalid_blob,
                    reference_count: 1,
                    local_root: None,
                },
                &ConversionContext,
            )
            .unwrap_err()
            .to_string()
            .contains("empty storage path")
    );

    let mut symbolic = folder(0);
    symbolic.is_symbolic = true;
    let conversion = CloudreveConverter.convert(
        CloudreveFileRecord {
            file: symbolic,
            entities: vec![],
        },
        &ConversionContext,
    )?;
    assert!(matches!(conversion, Conversion::Skipped(reason) if reason.code == "symbolic_file"));

    let conversion = CloudreveConverter.convert(
        CloudreveFileRecord {
            file: folder(0),
            entities: vec![],
        },
        &ConversionContext,
    )?;
    assert!(
        matches!(conversion, Conversion::Skipped(reason) if reason.code == "missing_primary_entity")
    );

    let mut negative_version_file = folder(0);
    negative_version_file.primary_entity = Some(13);
    assert!(
        CloudreveConverter
            .convert(
                CloudreveFileRecord {
                    file: negative_version_file,
                    entities: vec![entity(13, 0, -1)],
                },
                &ConversionContext,
            )
            .unwrap_err()
            .to_string()
            .contains("negative size")
    );
    Ok(())
}

#[test]
fn converts_file_and_folder_shares_with_download_semantics() -> Result<()> {
    for (target_type, expected_target) in [
        (0, MigrationShareTarget::File { source_id: 10 }),
        (1, MigrationShareTarget::Folder { source_id: 10 }),
    ] {
        let converted = ready(CloudreveConverter.convert(
            CloudreveShareRecord {
                share: share(30, Some(10), Some(9)),
                target: Some(folder(target_type)),
            },
            &ConversionContext,
        )?);
        assert_eq!(converted.source_id, 30);
        assert_eq!(converted.owner_source_id, 9);
        assert_eq!(converted.target, expected_target);
        assert_eq!(converted.plain_password.as_deref(), Some("secret"));
        assert_eq!(converted.max_downloads, 10);
        assert_eq!(converted.download_count, 7);
        assert_eq!(converted.view_count, 5);
    }

    let mut unlimited = share(31, Some(10), Some(9));
    unlimited.password = Some(String::new());
    unlimited.remain_downloads = None;
    let converted = ready(CloudreveConverter.convert(
        CloudreveShareRecord {
            share: unlimited,
            target: Some(folder(0)),
        },
        &ConversionContext,
    )?);
    assert_eq!(converted.plain_password, None);
    assert_eq!(converted.max_downloads, 0);
    assert_eq!(converted.download_count, 7);
    Ok(())
}

#[test]
fn handles_share_boundaries_without_reactivating_deleted_rows() -> Result<()> {
    let mut deleted = share(30, Some(10), Some(9));
    deleted.deleted_at = Some(now());
    let conversion = CloudreveConverter.convert(
        CloudreveShareRecord {
            share: deleted,
            target: Some(folder(0)),
        },
        &ConversionContext,
    )?;
    assert!(matches!(conversion, Conversion::Skipped(reason) if reason.code == "deleted_share"));

    for (source, target, expected_code) in [
        (
            share(30, Some(10), None),
            Some(folder(0)),
            "missing_share_owner",
        ),
        (share(30, None, Some(9)), None, "missing_share_target"),
        (share(30, Some(10), Some(9)), None, "missing_share_target"),
        (
            share(30, Some(10), Some(9)),
            Some(folder(2)),
            "unsupported_share_target",
        ),
    ] {
        let conversion = CloudreveConverter.convert(
            CloudreveShareRecord {
                share: source,
                target,
            },
            &ConversionContext,
        )?;
        assert!(matches!(conversion, Conversion::Skipped(reason) if reason.code == expected_code));
    }

    let mut negative = share(30, Some(10), Some(9));
    negative.remain_downloads = Some(-1);
    assert!(
        CloudreveConverter
            .convert(
                CloudreveShareRecord {
                    share: negative,
                    target: Some(folder(0)),
                },
                &ConversionContext,
            )
            .unwrap_err()
            .to_string()
            .contains("negative remaining downloads")
    );

    let mut overflow = share(30, Some(10), Some(9));
    overflow.downloads = i64::MAX;
    overflow.remain_downloads = Some(1);
    assert!(
        CloudreveConverter
            .convert(
                CloudreveShareRecord {
                    share: overflow,
                    target: Some(folder(0)),
                },
                &ConversionContext,
            )
            .unwrap_err()
            .to_string()
            .contains("download limit exceeds i64")
    );
    Ok(())
}

#[test]
fn converts_public_and_private_metadata_for_files_and_folders() -> Result<()> {
    for (is_public, target_type, namespace, kind) in [
        (true, 0, "cloudreve.public", MigrationEntityKind::File),
        (false, 1, "cloudreve.private", MigrationEntityKind::Folder),
    ] {
        let mut source = metadata(40, "author", "Cloudreve");
        source.is_public = is_public;
        let converted = ready(CloudreveConverter.convert(
            CloudreveMetadataRecord {
                metadata: source,
                target: Some(folder(target_type)),
            },
            &ConversionContext,
        )?);
        let MigrationMetadata::Property(property) = converted else {
            panic!("expected property conversion");
        };
        assert_eq!(property.source_metadata_id, 40);
        assert_eq!(property.target.kind, kind);
        assert_eq!(property.target.source_id, 10);
        assert_eq!(property.namespace, namespace);
        assert_eq!(property.name, "author");
        assert_eq!(property.value.as_deref(), Some("Cloudreve"));
    }
    Ok(())
}

#[test]
fn converts_tags_with_asterdrive_name_and_color_rules() -> Result<()> {
    for (source_name, source_color, expected_name, expected_color) in [
        ("tag:Important", "#AbC", "Important", "#aabbcc"),
        ("tag:  Project A  ", "#3B82F6", "Project A", "#3b82f6"),
        ("tag:Fallback", "invalid", "Fallback", DEFAULT_TAG_COLOR),
    ] {
        let converted = ready(CloudreveConverter.convert(
            CloudreveMetadataRecord {
                metadata: metadata(41, source_name, source_color),
                target: Some(folder(0)),
            },
            &ConversionContext,
        )?);
        let MigrationMetadata::TagAssignment(tag) = converted else {
            panic!("expected tag conversion");
        };
        assert_eq!(tag.source_metadata_id, 41);
        assert_eq!(tag.owner_source_id, 9);
        assert_eq!(tag.target.kind, MigrationEntityKind::File);
        assert_eq!(tag.name, expected_name);
        assert_eq!(tag.normalized_name, expected_name.to_lowercase());
        assert_eq!(tag.color, expected_color);
    }

    let long_name = format!("tag:{}", "x".repeat(65));
    let converted = ready(CloudreveConverter.convert(
        CloudreveMetadataRecord {
            metadata: metadata(42, &long_name, ""),
            target: Some(folder(1)),
        },
        &ConversionContext,
    )?);
    let MigrationMetadata::TagAssignment(tag) = converted else {
        panic!("expected tag conversion");
    };
    assert_eq!(tag.name.chars().count(), ASTER_DRIVE_TAG_NAME_MAX_CHARS);
    assert_eq!(tag.target.kind, MigrationEntityKind::Folder);

    let expanding_name = format!("tag:{}İ", "x".repeat(63));
    assert!(
        CloudreveConverter
            .convert(
                CloudreveMetadataRecord {
                    metadata: metadata(43, &expanding_name, "#abc"),
                    target: Some(folder(0)),
                },
                &ConversionContext,
            )
            .unwrap_err()
            .to_string()
            .contains("normalized tag name exceeds")
    );
    Ok(())
}

#[test]
fn skips_deleted_missing_and_unsupported_metadata() -> Result<()> {
    let mut deleted = metadata(40, "author", "Cloudreve");
    deleted.deleted_at = Some(now());
    let cases = [
        (
            CloudreveMetadataRecord {
                metadata: deleted,
                target: Some(folder(0)),
            },
            "deleted_metadata",
        ),
        (
            CloudreveMetadataRecord {
                metadata: metadata(41, "author", "Cloudreve"),
                target: None,
            },
            "missing_metadata_target",
        ),
        (
            CloudreveMetadataRecord {
                metadata: metadata(42, "author", "Cloudreve"),
                target: Some(folder(2)),
            },
            "unsupported_metadata_target",
        ),
        (
            CloudreveMetadataRecord {
                metadata: metadata(43, "tag:   ", "#abc"),
                target: Some(folder(0)),
            },
            "empty_tag_name",
        ),
    ];
    for (source, expected_code) in cases {
        let conversion = CloudreveConverter.convert(source, &ConversionContext)?;
        assert!(matches!(conversion, Conversion::Skipped(reason) if reason.code == expected_code));
    }
    Ok(())
}

#[test]
fn validates_metadata_target_identity_and_persistence_limits() -> Result<()> {
    let mut mismatched_target = folder(0);
    mismatched_target.id = 11;
    assert!(
        CloudreveConverter
            .convert(
                CloudreveMetadataRecord {
                    metadata: metadata(40, "author", "Cloudreve"),
                    target: Some(mismatched_target),
                },
                &ConversionContext,
            )
            .unwrap_err()
            .to_string()
            .contains("does not match target")
    );

    let unicode_boundary = "界".repeat(ASTER_DRIVE_PROPERTY_NAME_MAX_CHARS);
    let converted = ready(CloudreveConverter.convert(
        CloudreveMetadataRecord {
            metadata: metadata(41, &unicode_boundary, ""),
            target: Some(folder(0)),
        },
        &ConversionContext,
    )?);
    let MigrationMetadata::Property(property) = converted else {
        panic!("expected property conversion");
    };
    assert_eq!(
        property.name.chars().count(),
        ASTER_DRIVE_PROPERTY_NAME_MAX_CHARS
    );

    let too_long_name = "界".repeat(ASTER_DRIVE_PROPERTY_NAME_MAX_CHARS + 1);
    assert!(
        CloudreveConverter
            .convert(
                CloudreveMetadataRecord {
                    metadata: metadata(42, &too_long_name, ""),
                    target: Some(folder(0)),
                },
                &ConversionContext,
            )
            .unwrap_err()
            .to_string()
            .contains("name exceeds")
    );

    let value_boundary = "x".repeat(ASTER_DRIVE_PROPERTY_VALUE_MAX_BYTES);
    assert!(matches!(
        CloudreveConverter.convert(
            CloudreveMetadataRecord {
                metadata: metadata(43, "boundary", &value_boundary),
                target: Some(folder(0)),
            },
            &ConversionContext,
        )?,
        Conversion::Ready(MigrationMetadata::Property(_))
    ));
    let too_long_value = "x".repeat(ASTER_DRIVE_PROPERTY_VALUE_MAX_BYTES + 1);
    assert!(
        CloudreveConverter
            .convert(
                CloudreveMetadataRecord {
                    metadata: metadata(44, "too-long", &too_long_value),
                    target: Some(folder(0)),
                },
                &ConversionContext,
            )
            .unwrap_err()
            .to_string()
            .contains("value exceeds")
    );
    Ok(())
}

#[test]
fn converts_direct_links_as_file_scoped_legacy_records() -> Result<()> {
    let converted = ready(CloudreveConverter.convert(
        CloudreveDirectLinkRecord {
            direct_link: direct_link(50),
            target: Some(folder(0)),
        },
        &ConversionContext,
    )?);
    assert_eq!(converted.source_id, 50);
    assert_eq!(converted.file_source_id, 10);
    assert_eq!(converted.owner_source_id, 9);
    assert_eq!(converted.file_name, "Documents");
    assert_eq!(converted.source_name, "legacy-name.txt");
    assert_eq!(converted.source_downloads, 7);
    assert_eq!(converted.source_speed_limit, 1_024);
    Ok(())
}

#[test]
fn handles_direct_link_boundaries_without_reactivating_deleted_rows() -> Result<()> {
    let mut deleted = direct_link(50);
    deleted.deleted_at = Some(now());
    for (source, target, expected_code) in [
        (deleted, Some(folder(0)), "deleted_direct_link"),
        (direct_link(51), None, "missing_direct_link_target"),
        (
            direct_link(52),
            Some(folder(1)),
            "unsupported_direct_link_target",
        ),
    ] {
        let conversion = CloudreveConverter.convert(
            CloudreveDirectLinkRecord {
                direct_link: source,
                target,
            },
            &ConversionContext,
        )?;
        assert!(matches!(conversion, Conversion::Skipped(reason) if reason.code == expected_code));
    }

    let mut mismatched_target = folder(0);
    mismatched_target.id = 11;
    assert!(
        CloudreveConverter
            .convert(
                CloudreveDirectLinkRecord {
                    direct_link: direct_link(53),
                    target: Some(mismatched_target),
                },
                &ConversionContext,
            )
            .unwrap_err()
            .to_string()
            .contains("does not match target")
    );

    let mut negative_downloads = direct_link(54);
    negative_downloads.downloads = -1;
    assert!(
        CloudreveConverter
            .convert(
                CloudreveDirectLinkRecord {
                    direct_link: negative_downloads,
                    target: Some(folder(0)),
                },
                &ConversionContext,
            )
            .unwrap_err()
            .to_string()
            .contains("negative download count")
    );

    let mut negative_speed = direct_link(55);
    negative_speed.speed = -1;
    assert!(
        CloudreveConverter
            .convert(
                CloudreveDirectLinkRecord {
                    direct_link: negative_speed,
                    target: Some(folder(0)),
                },
                &ConversionContext,
            )
            .unwrap_err()
            .to_string()
            .contains("negative speed limit")
    );
    Ok(())
}
