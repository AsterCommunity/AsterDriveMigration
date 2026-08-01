//! Cloudreve schema records and their conversion into migration-domain values.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::unreachable,
        clippy::expect_used,
        clippy::panic,
        clippy::unimplemented,
        clippy::todo
    )
)]

mod converter;
mod local_path;
mod record;

pub use converter::CloudreveConverter;
pub use converter::storage_policy_skip_reason;
pub use converter::storage_region_setting;
pub use local_path::normalize_local_storage_path;
pub use record::{
    CloudreveBlobRecord, CloudreveDirectLinkRecord, CloudreveFileRecord, CloudreveFolderRecord,
    CloudreveMetadataRecord, CloudrevePolicyGroupRecord, CloudreveShareRecord,
    CloudreveStoragePolicyRecord, CloudreveUserRecord,
};

pub fn is_encrypted_entity(entity: &cloudreve_schema::entities::Model) -> bool {
    entity
        .recycle_options
        .as_ref()
        .and_then(|options| options.get("encrypt_metadata"))
        .is_some_and(|metadata| !metadata.is_null())
}
