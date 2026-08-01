use super::*;

impl SourceConverter<CloudreveFileRecord> for CloudreveConverter {
    type Output = MigrationFile;
    type Error = color_eyre::Report;

    fn convert(
        &self,
        source: CloudreveFileRecord,
        _: &ConversionContext,
    ) -> Result<Conversion<Self::Output>> {
        let file = source.file;
        if file.r#type != 0 {
            return Ok(Conversion::Skipped(SkipReason {
                code: "not_a_file",
                message: format!("Cloudreve file {} is not a regular file", file.id),
            }));
        }
        if file.is_symbolic {
            return Ok(Conversion::Skipped(SkipReason {
                code: "symbolic_file",
                message: "symbolic/placeholder files are not representable in AD".to_string(),
            }));
        }
        if file.size < 0 {
            bail!("Cloudreve file {} has negative size {}", file.id, file.size);
        }
        if file.primary_entity.is_none() {
            return Ok(Conversion::Skipped(SkipReason {
                code: "missing_primary_entity",
                message: format!("Cloudreve file {} has no current entity", file.id),
            }));
        }

        let mut entities = source
            .entities
            .into_iter()
            .filter(|entity| entity.r#type == 0)
            .collect::<Vec<_>>();
        entities.sort_by_key(|entity| (entity.created_at, entity.id));
        let versions = entities
            .into_iter()
            .map(|entity| {
                if entity.size < 0 {
                    bail!(
                        "Cloudreve entity {} for file {} has negative size {}",
                        entity.id,
                        file.id,
                        entity.size
                    );
                }
                Ok(MigrationFileVersion {
                    blob_source_id: entity.id,
                    size: entity.size,
                    created_at: target_time(entity.created_at),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Conversion::Ready(MigrationFile {
            source_id: file.id,
            name: file.name,
            owner_source_id: file.owner_id,
            folder_source_id: file.file_children,
            preferred_blob_source_id: file.primary_entity,
            versions,
            created_at: target_time(file.created_at),
            updated_at: target_time(file.updated_at),
        }))
    }
}
