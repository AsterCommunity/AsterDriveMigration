use super::*;

impl SourceConverter<CloudreveFolderRecord> for CloudreveConverter {
    type Output = MigrationFolder;
    type Error = color_eyre::Report;

    fn convert(
        &self,
        source: CloudreveFolderRecord,
        _: &ConversionContext,
    ) -> Result<Conversion<Self::Output>> {
        let folder = source.folder;
        if folder.r#type != 1 {
            return Ok(Conversion::Skipped(SkipReason {
                code: "not_a_folder",
                message: format!("Cloudreve file {} is not a folder", folder.id),
            }));
        }
        Ok(Conversion::Ready(MigrationFolder {
            source_id: folder.id,
            name: folder.name,
            parent_source_id: folder.file_children,
            owner_source_id: folder.owner_id,
            policy_source_id: folder.storage_policy_files,
            created_at: target_time(folder.created_at),
            updated_at: target_time(folder.updated_at),
        }))
    }
}
