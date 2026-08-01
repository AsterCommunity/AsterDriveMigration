//! Source-neutral objects exchanged between source adapters and target writers.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationStorageDriver {
    Local,
    S3,
    TencentCos,
    OneDrive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationObjectStorageUploadStrategy {
    RelayStream,
    Presigned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationObjectStorageDownloadStrategy {
    RelayStream,
    Presigned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationMicrosoftGraphCloud {
    Global,
    China,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationOneDriveAccountMode {
    WorkOrSchool,
    SharepointSite,
    GroupDrive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationProviderResumableUploadStrategy {
    ServerRelay,
    FrontendDirect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationProviderDownloadStrategy {
    ServerRelay,
    FrontendDirect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationProviderDownloadFilenameMode {
    ProviderNative,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationOneDriveOptions {
    pub cloud: MigrationMicrosoftGraphCloud,
    pub account_mode: MigrationOneDriveAccountMode,
    pub tenant: String,
    pub drive_id: Option<String>,
    pub root_item_id: Option<String>,
    pub site_id: Option<String>,
    pub group_id: Option<String>,
    pub upload_strategy: MigrationProviderResumableUploadStrategy,
    pub download_strategy: MigrationProviderDownloadStrategy,
    pub download_filename_mode: MigrationProviderDownloadFilenameMode,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MigrationStoragePolicy {
    pub source_id: i64,
    pub name: String,
    pub driver: MigrationStorageDriver,
    pub endpoint: String,
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
    pub base_path: String,
    pub max_file_size: i64,
    pub allowed_types: Vec<String>,
    pub s3_path_style: bool,
    pub s3_region: Option<String>,
    pub object_storage_upload_strategy: Option<MigrationObjectStorageUploadStrategy>,
    pub object_storage_download_strategy: Option<MigrationObjectStorageDownloadStrategy>,
    pub onedrive: Option<MigrationOneDriveOptions>,
    pub extensions: BTreeMap<String, Value>,
    pub chunk_size: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationPolicyGroup {
    pub source_id: i64,
    pub name: String,
    pub description: String,
    pub policy_source_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationUserRole {
    User,
    Admin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationUserStatus {
    Active,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationAvatarSource {
    None,
    Gravatar,
    Upload,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MigrationUser {
    pub source_id: i64,
    pub username: String,
    pub email: String,
    pub display_name: String,
    pub role: MigrationUserRole,
    pub status: MigrationUserStatus,
    pub storage_used: i64,
    pub storage_quota: i64,
    pub policy_group_source_id: i64,
    pub config: Option<Value>,
    pub avatar_source: MigrationAvatarSource,
    pub avatar_key: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationFolder {
    pub source_id: i64,
    pub name: String,
    pub parent_source_id: Option<i64>,
    pub owner_source_id: i64,
    pub policy_source_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationBlob {
    pub source_id: i64,
    pub policy_source_id: i64,
    pub opaque_key: String,
    pub storage_path: String,
    pub size: i64,
    pub reference_count: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationFileVersion {
    pub blob_source_id: i64,
    pub size: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationFile {
    pub source_id: i64,
    pub name: String,
    pub owner_source_id: i64,
    pub folder_source_id: Option<i64>,
    pub preferred_blob_source_id: Option<i64>,
    pub versions: Vec<MigrationFileVersion>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationShareTarget {
    File { source_id: i64 },
    Folder { source_id: i64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationShare {
    pub source_id: i64,
    pub owner_source_id: i64,
    pub target: MigrationShareTarget,
    pub plain_password: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub max_downloads: i64,
    pub download_count: i64,
    pub view_count: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MigrationEntityKind {
    File,
    Folder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MigrationEntityRef {
    pub kind: MigrationEntityKind,
    pub source_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationProperty {
    pub source_metadata_id: i64,
    pub target: MigrationEntityRef,
    pub namespace: String,
    pub name: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationTagAssignment {
    pub source_metadata_id: i64,
    pub owner_source_id: i64,
    pub target: MigrationEntityRef,
    pub name: String,
    pub normalized_name: String,
    pub color: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationMetadata {
    Property(MigrationProperty),
    TagAssignment(MigrationTagAssignment),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationDirectLink {
    pub source_id: i64,
    pub file_source_id: i64,
    pub owner_source_id: i64,
    pub file_name: String,
    pub source_name: String,
    pub source_downloads: i64,
    pub source_speed_limit: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_driver_enum_covers_all_target_drivers() {
        assert_eq!(
            [
                MigrationStorageDriver::Local,
                MigrationStorageDriver::S3,
                MigrationStorageDriver::TencentCos,
                MigrationStorageDriver::OneDrive,
            ]
            .len(),
            4
        );
        assert_ne!(
            MigrationStorageDriver::S3,
            MigrationStorageDriver::TencentCos
        );
        assert_ne!(
            MigrationObjectStorageUploadStrategy::RelayStream,
            MigrationObjectStorageUploadStrategy::Presigned
        );
        assert_ne!(
            MigrationObjectStorageDownloadStrategy::RelayStream,
            MigrationObjectStorageDownloadStrategy::Presigned
        );
    }

    #[test]
    fn entity_refs_and_share_targets_are_structural() {
        let file = MigrationEntityRef {
            kind: MigrationEntityKind::File,
            source_id: 0,
        };
        let folder = MigrationEntityRef {
            kind: MigrationEntityKind::Folder,
            source_id: -1,
        };
        assert_ne!(file, folder);
        assert_eq!(
            MigrationShareTarget::File { source_id: 7 },
            MigrationShareTarget::File { source_id: 7 }
        );
        assert_ne!(
            MigrationShareTarget::File { source_id: 7 },
            MigrationShareTarget::Folder { source_id: 7 }
        );
    }

    #[test]
    fn domain_values_allow_source_statistics_without_implicit_validation() {
        let now = DateTime::<Utc>::UNIX_EPOCH;
        let blob = MigrationBlob {
            source_id: 1,
            policy_source_id: 2,
            opaque_key: String::new(),
            storage_path: String::new(),
            size: -1,
            reference_count: -1,
            created_at: now,
            updated_at: now,
        };
        assert_eq!(blob.size, -1);
        assert_eq!(blob.reference_count, -1);
    }
}
