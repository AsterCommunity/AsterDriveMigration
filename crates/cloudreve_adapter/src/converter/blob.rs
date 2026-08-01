use super::*;

impl SourceConverter<CloudreveBlobRecord> for CloudreveConverter {
    type Output = MigrationBlob;
    type Error = color_eyre::Report;

    fn convert(
        &self,
        source: CloudreveBlobRecord,
        _: &ConversionContext,
    ) -> Result<Conversion<Self::Output>> {
        let entity = source.entity;
        if entity.r#type != 0 {
            return Ok(Conversion::Skipped(SkipReason {
                code: "not_a_blob",
                message: format!("Cloudreve entity {} is not an original object", entity.id),
            }));
        }
        if crate::is_encrypted_entity(&entity) {
            return Ok(Conversion::Skipped(SkipReason {
                code: "cloudreve_encrypted_entity",
                message: "Cloudreve encrypted entity is not supported".to_string(),
            }));
        }
        if entity.size < 0 {
            bail!(
                "Cloudreve entity {} has negative size {}",
                entity.id,
                entity.size
            );
        }
        if entity.source.is_empty() {
            bail!("Cloudreve entity {} has an empty storage path", entity.id);
        }
        let reference_count = i32::try_from(source.reference_count)
            .map_err(|_| color_eyre::eyre::eyre!("blob reference count exceeds i32"))?;
        if reference_count < 0 {
            bail!(
                "Cloudreve entity {} has a negative reference count",
                entity.id
            );
        }
        let storage_path = match source.local_root {
            Some(root) => {
                crate::normalize_local_storage_path(&root, &entity.source).map_err(|error| {
                    color_eyre::eyre::eyre!(
                        "Cloudreve local entity {} has an invalid storage path: {error}",
                        entity.id
                    )
                })?
            }
            None => entity.source,
        };
        Ok(Conversion::Ready(MigrationBlob {
            source_id: entity.id,
            policy_source_id: entity.storage_policy_entities,
            opaque_key: format!("cloudreve-{:016x}", entity.id),
            storage_path,
            size: entity.size,
            reference_count,
            created_at: target_time(entity.created_at),
            updated_at: target_time(entity.updated_at),
        }))
    }
}
