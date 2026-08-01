use std::collections::BTreeMap;

use aster_drive_migration_core::{
    MigrationAvatarSource, MigrationBlob, MigrationDirectLink, MigrationFile, MigrationFileVersion,
    MigrationFolder, MigrationMicrosoftGraphCloud, MigrationObjectStorageDownloadStrategy,
    MigrationObjectStorageUploadStrategy, MigrationOneDriveAccountMode, MigrationOneDriveOptions,
    MigrationPolicyGroup, MigrationProviderDownloadFilenameMode, MigrationProviderDownloadStrategy,
    MigrationProviderResumableUploadStrategy, MigrationStorageDriver, MigrationStoragePolicy,
    MigrationUser, MigrationUserRole, MigrationUserStatus,
};
use aster_drive_model as aster_drive_schema;
use aster_drive_model::types::{EntityType, TagScopeType};
use aster_drive_writer::*;
use color_eyre::eyre::{Result, WrapErr};
use sea_orm::{
    ColumnTrait, ConnectOptions, ConnectionTrait, Database, DatabaseConnection,
    DatabaseTransaction, EntityTrait, PaginatorTrait, QueryFilter, TransactionTrait,
};
use serde_json::Value;

async fn database() -> Result<DatabaseConnection> {
    let mut options = ConnectOptions::new("sqlite::memory:");
    options.max_connections(1);
    let database = Database::connect(options)
        .await
        .wrap_err("connect writer test database")?;
    database
        .execute_unprepared("PRAGMA foreign_keys = ON")
        .await
        .wrap_err("enable SQLite foreign keys")?;
    aster_drive_schema_migration::Migrator::up(&database, None)
        .await
        .wrap_err("apply AsterDrive schema migrations")?;
    Ok(database)
}

async fn write_prerequisites(transaction: &DatabaseTransaction) -> Result<(i64, i64, i64)> {
    let now = chrono::Utc::now();
    let writer = AsterDriveWriter::new(transaction);
    let policy_id = writer
        .write_policy(
            MigrationStoragePolicy {
                source_id: 1,
                name: "Local".to_string(),
                driver: MigrationStorageDriver::Local,
                endpoint: String::new(),
                bucket: String::new(),
                access_key: String::new(),
                secret_key: String::new(),
                base_path: "/storage".to_string(),
                max_file_size: 0,
                allowed_types: Vec::new(),
                s3_path_style: true,
                s3_region: None,
                object_storage_upload_strategy: None,
                object_storage_download_strategy: None,
                onedrive: None,
                extensions: BTreeMap::new(),
                chunk_size: 0,
                created_at: now,
                updated_at: now,
            },
            true,
        )
        .await?;
    let user_id = writer
        .write_user(
            ResolvedUser {
                user: MigrationUser {
                    source_id: 2,
                    username: "owner".to_string(),
                    email: "owner@example.test".to_string(),
                    display_name: "Owner".to_string(),
                    role: MigrationUserRole::User,
                    status: MigrationUserStatus::Active,
                    storage_used: 0,
                    storage_quota: 0,
                    policy_group_source_id: 0,
                    config: None,
                    avatar_source: MigrationAvatarSource::None,
                    avatar_key: None,
                    created_at: now,
                    updated_at: now,
                },
                policy_group_id: None,
            },
            "migration-test-password-hash",
        )
        .await?;
    let folder_id = writer
        .write_folder(ResolvedFolder {
            folder: MigrationFolder {
                source_id: 3,
                name: "Documents".to_string(),
                parent_source_id: None,
                owner_source_id: 2,
                policy_source_id: Some(1),
                created_at: now,
                updated_at: now,
            },
            parent_id: None,
            owner_id: user_id,
            owner_username: "owner".to_string(),
            policy_id: Some(policy_id),
        })
        .await?;
    Ok((policy_id, user_id, folder_id))
}

fn blob(source_id: i64, storage_path: &str, size: i64, reference_count: i32) -> MigrationBlob {
    let now = chrono::Utc::now();
    MigrationBlob {
        source_id,
        policy_source_id: 1,
        opaque_key: format!("cloudreve-{source_id:016x}"),
        storage_path: storage_path.to_string(),
        size,
        reference_count,
        created_at: now,
        updated_at: now,
    }
}

fn file(source_id: i64) -> MigrationFile {
    let now = chrono::Utc::now();
    MigrationFile {
        source_id,
        name: "archive.tar.gz".to_string(),
        owner_source_id: 2,
        folder_source_id: Some(3),
        preferred_blob_source_id: Some(11),
        versions: vec![MigrationFileVersion {
            blob_source_id: 11,
            size: 512,
            created_at: now,
        }],
        created_at: now,
        updated_at: now,
    }
}

fn share(source_id: i64) -> ResolvedShare {
    let now = chrono::Utc::now();
    ResolvedShare {
        source_id,
        owner_id: 0,
        target: ResolvedShareTarget::Folder { target_id: 0 },
        password_hash: Some("$argon2id$test-hash".to_string()),
        expires_at: Some(now + chrono::Duration::days(1)),
        max_downloads: 10,
        download_count: 7,
        view_count: 5,
        created_at: now,
        updated_at: now,
    }
}

fn direct_link(source_id: i64) -> MigrationDirectLink {
    MigrationDirectLink {
        source_id,
        file_source_id: 71,
        owner_source_id: 2,
        file_name: "report 2026.txt".to_string(),
        source_name: "legacy-name.txt".to_string(),
        source_downloads: 7,
        source_speed_limit: 1_024,
    }
}

async fn write_file_target(
    transaction: &DatabaseTransaction,
    policy_id: i64,
    user_id: i64,
    folder_id: i64,
) -> Result<i64> {
    let writer = AsterDriveWriter::new(transaction);
    let blob_id = writer
        .write_blob(ResolvedBlob {
            blob: blob(70, "objects/property-target", 64, 1),
            policy_id,
        })
        .await?;
    Ok(writer
        .write_file(ResolvedFile {
            file: file(71),
            folder_id: Some(folder_id),
            owner_id: user_id,
            owner_username: "owner".to_string(),
            primary_blob_id: blob_id,
            primary_blob_size: 64,
            historical_versions: Vec::new(),
        })
        .await?
        .target_id)
}

#[tokio::test]
async fn writes_policy_driver_options_allowed_types_and_extensions() -> Result<()> {
    let database = database().await?;
    let transaction = database.begin().await?;
    let now = chrono::Utc::now();
    let mut extensions = BTreeMap::new();
    extensions.insert("custom_flag".to_string(), Value::Bool(true));
    let target_id = AsterDriveWriter::new(&transaction)
        .write_policy(
            MigrationStoragePolicy {
                source_id: 10,
                name: "S3 policy".to_string(),
                driver: MigrationStorageDriver::S3,
                endpoint: "https://bucket.s3.example.test".to_string(),
                bucket: "bucket".to_string(),
                access_key: "access".to_string(),
                secret_key: "secret".to_string(),
                base_path: "objects".to_string(),
                max_file_size: 123,
                allowed_types: vec!["jpg".to_string(), "png".to_string()],
                s3_path_style: false,
                s3_region: Some("ap-southeast-1".to_string()),
                object_storage_upload_strategy: Some(
                    MigrationObjectStorageUploadStrategy::Presigned,
                ),
                object_storage_download_strategy: Some(
                    MigrationObjectStorageDownloadStrategy::RelayStream,
                ),
                onedrive: None,
                extensions,
                chunk_size: 456,
                created_at: now,
                updated_at: now,
            },
            false,
        )
        .await?;
    let cos_target_id = AsterDriveWriter::new(&transaction)
        .write_policy(
            MigrationStoragePolicy {
                source_id: 11,
                name: "COS policy".to_string(),
                driver: MigrationStorageDriver::TencentCos,
                endpoint: "https://bucket.cos.ap-guangzhou.myqcloud.com".to_string(),
                bucket: "bucket".to_string(),
                access_key: "access".to_string(),
                secret_key: "secret".to_string(),
                base_path: String::new(),
                max_file_size: 0,
                allowed_types: Vec::new(),
                s3_path_style: false,
                s3_region: None,
                object_storage_upload_strategy: Some(
                    MigrationObjectStorageUploadStrategy::RelayStream,
                ),
                object_storage_download_strategy: Some(
                    MigrationObjectStorageDownloadStrategy::Presigned,
                ),
                onedrive: None,
                extensions: BTreeMap::new(),
                chunk_size: 0,
                created_at: now,
                updated_at: now,
            },
            false,
        )
        .await?;
    let onedrive_target_id = AsterDriveWriter::new(&transaction)
        .write_policy(
            MigrationStoragePolicy {
                source_id: 12,
                name: "OneDrive policy".to_string(),
                driver: MigrationStorageDriver::OneDrive,
                endpoint: String::new(),
                bucket: String::new(),
                access_key: String::new(),
                secret_key: String::new(),
                base_path: String::new(),
                max_file_size: 0,
                allowed_types: Vec::new(),
                s3_path_style: false,
                s3_region: None,
                object_storage_upload_strategy: None,
                object_storage_download_strategy: None,
                onedrive: Some(MigrationOneDriveOptions {
                    cloud: MigrationMicrosoftGraphCloud::China,
                    account_mode: MigrationOneDriveAccountMode::SharepointSite,
                    tenant: "common".to_string(),
                    drive_id: Some("drive-id".to_string()),
                    root_item_id: Some("root".to_string()),
                    site_id: Some("site-id".to_string()),
                    group_id: None,
                    upload_strategy: MigrationProviderResumableUploadStrategy::ServerRelay,
                    download_strategy: MigrationProviderDownloadStrategy::FrontendDirect,
                    download_filename_mode: MigrationProviderDownloadFilenameMode::ProviderNative,
                }),
                extensions: BTreeMap::new(),
                chunk_size: 50 * 1024 * 1024,
                created_at: now,
                updated_at: now,
            },
            false,
        )
        .await?;
    transaction.commit().await?;

    let policy = aster_drive_schema::entities::storage_policy::Entity::find_by_id(target_id)
        .one(&database)
        .await?
        .expect("written policy");
    assert_eq!(
        policy.driver_type,
        aster_drive_schema::types::DriverType::S3
    );
    assert!(!policy.is_default);
    assert_eq!(policy.max_file_size, 123);
    assert_eq!(policy.chunk_size, 456);
    assert_eq!(policy.allowed_types.as_ref(), r#"["jpg","png"]"#);
    let options: Value = serde_json::from_str(policy.options.as_ref())?;
    assert_eq!(options["s3_path_style"], false);
    assert_eq!(options["s3_region"], "ap-southeast-1");
    assert_eq!(options["object_storage_upload_strategy"], "presigned");
    assert_eq!(options["object_storage_download_strategy"], "relay_stream");
    assert_eq!(options["custom_flag"], true);

    let cos_policy =
        aster_drive_schema::entities::storage_policy::Entity::find_by_id(cos_target_id)
            .one(&database)
            .await?
            .expect("written COS policy");
    assert_eq!(
        cos_policy.driver_type,
        aster_drive_schema::types::DriverType::TencentCos
    );
    let cos_options: Value = serde_json::from_str(cos_policy.options.as_ref())?;
    assert_eq!(
        cos_options["object_storage_upload_strategy"],
        "relay_stream"
    );
    assert_eq!(cos_options["object_storage_download_strategy"], "presigned");
    assert!(cos_options.get("s3_region").is_none());

    let onedrive_policy =
        aster_drive_schema::entities::storage_policy::Entity::find_by_id(onedrive_target_id)
            .one(&database)
            .await?
            .expect("written OneDrive policy");
    assert_eq!(
        onedrive_policy.driver_type,
        aster_drive_schema::types::DriverType::OneDrive
    );
    assert!(onedrive_policy.endpoint.is_empty());
    assert!(onedrive_policy.bucket.is_empty());
    assert!(onedrive_policy.access_key.is_empty());
    assert!(onedrive_policy.secret_key.is_empty());
    assert_eq!(onedrive_policy.chunk_size, 50 * 1024 * 1024);
    let onedrive_options: Value = serde_json::from_str(onedrive_policy.options.as_ref())?;
    assert_eq!(onedrive_options["onedrive_cloud"], "china");
    assert_eq!(onedrive_options["onedrive_account_mode"], "sharepoint_site");
    assert_eq!(onedrive_options["onedrive_drive_id"], "drive-id");
    assert_eq!(onedrive_options["onedrive_root_item_id"], "root");
    assert_eq!(onedrive_options["onedrive_site_id"], "site-id");
    assert_eq!(
        onedrive_options["provider_resumable_upload_strategy"],
        "server_relay"
    );
    assert_eq!(
        onedrive_options["provider_download_strategy"],
        "frontend_direct"
    );
    assert_eq!(
        onedrive_options["provider_download_filename_mode"],
        "provider_native"
    );
    assert!(onedrive_options.get("s3_path_style").is_none());
    Ok(())
}

#[tokio::test]
async fn writes_policy_groups_with_and_without_policy_items() -> Result<()> {
    let database = database().await?;
    let transaction = database.begin().await?;
    let now = chrono::Utc::now();
    let writer = AsterDriveWriter::new(&transaction);
    let policy_id = writer
        .write_policy(
            MigrationStoragePolicy {
                source_id: 1,
                name: "Local".to_string(),
                driver: MigrationStorageDriver::Local,
                endpoint: String::new(),
                bucket: String::new(),
                access_key: String::new(),
                secret_key: String::new(),
                base_path: "/storage".to_string(),
                max_file_size: 0,
                allowed_types: Vec::new(),
                s3_path_style: true,
                s3_region: None,
                object_storage_upload_strategy: None,
                object_storage_download_strategy: None,
                onedrive: None,
                extensions: BTreeMap::new(),
                chunk_size: 0,
                created_at: now,
                updated_at: now,
            },
            true,
        )
        .await?;
    let with_policy = writer
        .write_policy_group(ResolvedPolicyGroup {
            group: MigrationPolicyGroup {
                source_id: 2,
                name: "with policy".to_string(),
                description: "description".to_string(),
                policy_source_id: Some(1),
                created_at: now,
                updated_at: now,
            },
            policy_id: Some(policy_id),
        })
        .await?;
    let without_policy = writer
        .write_policy_group(ResolvedPolicyGroup {
            group: MigrationPolicyGroup {
                source_id: 3,
                name: "without policy".to_string(),
                description: String::new(),
                policy_source_id: None,
                created_at: now,
                updated_at: now,
            },
            policy_id: None,
        })
        .await?;
    transaction.commit().await?;

    let items = aster_drive_schema::entities::storage_policy_group_item::Entity::find()
        .all(&database)
        .await?;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].group_id, with_policy);
    assert_eq!(items[0].policy_id, policy_id);
    assert_ne!(with_policy, without_policy);
    Ok(())
}

#[tokio::test]
async fn writes_disabled_user_profile_without_verification_timestamp() -> Result<()> {
    let database = database().await?;
    let transaction = database.begin().await?;
    let now = chrono::Utc::now();
    let user_id = AsterDriveWriter::new(&transaction)
        .write_user(
            ResolvedUser {
                user: MigrationUser {
                    source_id: 20,
                    username: "disabled".to_string(),
                    email: "disabled@example.test".to_string(),
                    display_name: "Disabled User".to_string(),
                    role: MigrationUserRole::Admin,
                    status: MigrationUserStatus::Disabled,
                    storage_used: 11,
                    storage_quota: 22,
                    policy_group_source_id: 0,
                    config: Some(serde_json::json!({"theme": "dark"})),
                    avatar_source: MigrationAvatarSource::Upload,
                    avatar_key: Some("avatar/key".to_string()),
                    created_at: now,
                    updated_at: now,
                },
                policy_group_id: None,
            },
            "hash",
        )
        .await?;
    transaction.commit().await?;

    let user = aster_drive_schema::entities::user::Entity::find_by_id(user_id)
        .one(&database)
        .await?
        .expect("written user");
    assert_eq!(user.role, aster_drive_schema::types::UserRole::Admin);
    assert_eq!(user.status, aster_drive_schema::types::UserStatus::Disabled);
    assert_eq!(user.email_verified_at, None);
    assert_eq!(user.storage_used, 11);
    let profile = aster_drive_schema::entities::user_profile::Entity::find_by_id(user_id)
        .one(&database)
        .await?
        .expect("written profile");
    assert_eq!(profile.display_name.as_deref(), Some("Disabled User"));
    assert_eq!(profile.avatar_key.as_deref(), Some("avatar/key"));
    Ok(())
}
#[path = "writer/blob_file.rs"]
mod blob_file;
#[path = "writer/direct_link.rs"]
mod direct_link;
#[path = "writer/metadata.rs"]
mod metadata;
#[path = "writer/share.rs"]
mod share;
