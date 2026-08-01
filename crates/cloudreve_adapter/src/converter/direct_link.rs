use super::*;

impl SourceConverter<CloudreveDirectLinkRecord> for CloudreveConverter {
    type Output = MigrationDirectLink;
    type Error = color_eyre::Report;

    fn convert(
        &self,
        source: CloudreveDirectLinkRecord,
        _: &ConversionContext,
    ) -> Result<Conversion<Self::Output>> {
        let link = source.direct_link;
        if link.deleted_at.is_some() {
            return Ok(Conversion::Skipped(SkipReason {
                code: "deleted_direct_link",
                message: format!("Cloudreve direct link {} is deleted", link.id),
            }));
        }
        let Some(target) = source.target else {
            return Ok(Conversion::Skipped(SkipReason {
                code: "missing_direct_link_target",
                message: format!(
                    "Cloudreve direct link {} target file {} does not exist",
                    link.id, link.file_id
                ),
            }));
        };
        if target.id != link.file_id {
            bail!(
                "Cloudreve direct link {} target record {} does not match target {}",
                link.id,
                target.id,
                link.file_id
            );
        }
        if target.r#type != 0 {
            return Ok(Conversion::Skipped(SkipReason {
                code: "unsupported_direct_link_target",
                message: format!(
                    "Cloudreve direct link {} target {} is not a file",
                    link.id, target.id
                ),
            }));
        }
        if link.downloads < 0 {
            bail!(
                "Cloudreve direct link {} has negative download count {}",
                link.id,
                link.downloads
            );
        }
        if link.speed < 0 {
            bail!(
                "Cloudreve direct link {} has negative speed limit {}",
                link.id,
                link.speed
            );
        }
        Ok(Conversion::Ready(MigrationDirectLink {
            source_id: link.id,
            file_source_id: target.id,
            owner_source_id: target.owner_id,
            file_name: target.name,
            source_name: link.name,
            source_downloads: link.downloads,
            source_speed_limit: link.speed,
        }))
    }
}
