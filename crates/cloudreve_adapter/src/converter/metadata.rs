use super::*;

impl SourceConverter<CloudreveMetadataRecord> for CloudreveConverter {
    type Output = MigrationMetadata;
    type Error = color_eyre::Report;

    fn convert(
        &self,
        source: CloudreveMetadataRecord,
        _: &ConversionContext,
    ) -> Result<Conversion<Self::Output>> {
        let metadata = source.metadata;
        if metadata.deleted_at.is_some() {
            return Ok(Conversion::Skipped(SkipReason {
                code: "deleted_metadata",
                message: format!("Cloudreve metadata {} is deleted", metadata.id),
            }));
        }
        let Some(target) = source.target else {
            return Ok(Conversion::Skipped(SkipReason {
                code: "missing_metadata_target",
                message: format!(
                    "Cloudreve metadata {} target {} does not exist",
                    metadata.id, metadata.file_id
                ),
            }));
        };
        if target.id != metadata.file_id {
            bail!(
                "Cloudreve metadata {} target record {} does not match target {}",
                metadata.id,
                target.id,
                metadata.file_id
            );
        }
        let Some(target_ref) = metadata_target(&target) else {
            return Ok(Conversion::Skipped(SkipReason {
                code: "unsupported_metadata_target",
                message: format!(
                    "Cloudreve metadata {} target {} has unsupported type {}",
                    metadata.id, target.id, target.r#type
                ),
            }));
        };

        if let Some(source_tag_name) = tag_name(&metadata.name) {
            let name = target_tag_name(source_tag_name);
            if name.is_empty() {
                return Ok(Conversion::Skipped(SkipReason {
                    code: "empty_tag_name",
                    message: format!(
                        "Cloudreve metadata {} tag name is empty after trimming",
                        metadata.id
                    ),
                }));
            }
            let normalized_name = name.to_lowercase();
            if normalized_name.chars().count() > ASTER_DRIVE_TAG_NAME_MAX_CHARS {
                bail!(
                    "Cloudreve metadata {} normalized tag name exceeds AsterDrive's {} character limit",
                    metadata.id,
                    ASTER_DRIVE_TAG_NAME_MAX_CHARS
                );
            }
            return Ok(Conversion::Ready(MigrationMetadata::TagAssignment(
                MigrationTagAssignment {
                    source_metadata_id: metadata.id,
                    owner_source_id: target.owner_id,
                    target: target_ref,
                    normalized_name,
                    name,
                    color: target_tag_color(&metadata.value),
                    created_at: target_time(metadata.created_at),
                    updated_at: target_time(metadata.updated_at),
                },
            )));
        }

        if metadata.name.chars().count() > ASTER_DRIVE_PROPERTY_NAME_MAX_CHARS {
            bail!(
                "Cloudreve metadata {} name exceeds AsterDrive's {} character limit",
                metadata.id,
                ASTER_DRIVE_PROPERTY_NAME_MAX_CHARS
            );
        }
        if metadata.value.len() > ASTER_DRIVE_PROPERTY_VALUE_MAX_BYTES {
            bail!(
                "Cloudreve metadata {} value exceeds AsterDrive's {} byte API limit",
                metadata.id,
                ASTER_DRIVE_PROPERTY_VALUE_MAX_BYTES
            );
        }
        Ok(Conversion::Ready(MigrationMetadata::Property(
            MigrationProperty {
                source_metadata_id: metadata.id,
                target: target_ref,
                namespace: if metadata.is_public {
                    "cloudreve.public".to_string()
                } else {
                    "cloudreve.private".to_string()
                },
                name: metadata.name,
                value: Some(metadata.value),
            },
        )))
    }
}
