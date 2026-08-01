use base64::Engine;
use color_eyre::eyre::{Result, WrapErr};
use hmac::{Hmac, Mac};
use sea_orm::{ActiveModelTrait, DatabaseTransaction, Set};
use serde_json::{Map, Value, json};
use sha2::Sha256;

use aster_drive_model as aster_drive_schema;
use aster_drive_model::types::{
    AvatarSource, DriverType, EntityType, StoredStoragePolicyAllowedTypes,
    StoredStoragePolicyOptions, StoredUserConfig, TagScopeType, UserRole, UserStatus,
};

use aster_drive_migration_core::{
    MigrationAvatarSource, MigrationBlob, MigrationDirectLink, MigrationFile, MigrationFolder,
    MigrationMicrosoftGraphCloud, MigrationObjectStorageDownloadStrategy,
    MigrationObjectStorageUploadStrategy, MigrationOneDriveAccountMode, MigrationPolicyGroup,
    MigrationProviderDownloadFilenameMode, MigrationProviderDownloadStrategy,
    MigrationProviderResumableUploadStrategy, MigrationStorageDriver, MigrationStoragePolicy,
    MigrationUser, MigrationUserRole, MigrationUserStatus,
};

pub struct ResolvedPolicyGroup {
    pub group: MigrationPolicyGroup,
    pub policy_id: Option<i64>,
}

pub struct ResolvedUser {
    pub user: MigrationUser,
    pub policy_group_id: Option<i64>,
}

pub struct ResolvedFolder {
    pub folder: MigrationFolder,
    pub parent_id: Option<i64>,
    pub owner_id: i64,
    pub owner_username: String,
    pub policy_id: Option<i64>,
}

pub struct ResolvedBlob {
    pub blob: MigrationBlob,
    pub policy_id: i64,
}

pub struct ResolvedFileVersion {
    pub blob_id: i64,
    pub size: i64,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub struct ResolvedFile {
    pub file: MigrationFile,
    pub folder_id: Option<i64>,
    pub owner_id: i64,
    pub owner_username: String,
    pub primary_blob_id: i64,
    pub primary_blob_size: i64,
    pub historical_versions: Vec<ResolvedFileVersion>,
}

pub enum ResolvedShareTarget {
    File { target_id: i64 },
    Folder { target_id: i64 },
}

pub struct ResolvedShare {
    pub source_id: i64,
    pub owner_id: i64,
    pub target: ResolvedShareTarget,
    pub password_hash: Option<String>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub max_downloads: i64,
    pub download_count: i64,
    pub view_count: i64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResolvedEntityTarget {
    File { target_id: i64 },
    Folder { target_id: i64 },
}

pub struct ResolvedProperty {
    pub source_metadata_id: i64,
    pub target: ResolvedEntityTarget,
    pub namespace: String,
    pub name: String,
    pub value: Option<String>,
}

pub struct ResolvedTag {
    pub source_metadata_id: i64,
    pub owner_id: i64,
    pub name: String,
    pub normalized_name: String,
    pub color: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

pub struct ResolvedTagAssignment {
    pub source_metadata_id: i64,
    pub target: ResolvedEntityTarget,
    pub tag_id: i64,
}

pub struct ResolvedDirectLink {
    pub direct_link: MigrationDirectLink,
    pub target_file_id: i64,
    pub target_owner_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrittenDirectLink {
    pub property_id: i64,
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WrittenFile {
    pub target_id: i64,
    pub version_count: usize,
}

pub struct AsterDriveWriter<'a> {
    transaction: &'a DatabaseTransaction,
}

impl<'a> AsterDriveWriter<'a> {
    pub const fn new(transaction: &'a DatabaseTransaction) -> Self {
        Self { transaction }
    }
}

mod blob;
mod direct_link;
mod file;
mod folder;
mod metadata;
mod policy;
mod share;
mod user;
