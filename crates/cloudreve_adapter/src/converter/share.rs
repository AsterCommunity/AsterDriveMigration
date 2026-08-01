use super::*;

impl SourceConverter<CloudreveShareRecord> for CloudreveConverter {
    type Output = MigrationShare;
    type Error = color_eyre::Report;

    fn convert(
        &self,
        source: CloudreveShareRecord,
        _: &ConversionContext,
    ) -> Result<Conversion<Self::Output>> {
        let share = source.share;
        if share.deleted_at.is_some() {
            return Ok(Conversion::Skipped(SkipReason {
                code: "deleted_share",
                message: format!("Cloudreve share {} is deleted", share.id),
            }));
        }
        let Some(owner_source_id) = share.user_shares else {
            return Ok(Conversion::Skipped(SkipReason {
                code: "missing_share_owner",
                message: format!("Cloudreve share {} has no owner user", share.id),
            }));
        };
        let Some(target_source_id) = share.file_shares else {
            return Ok(Conversion::Skipped(SkipReason {
                code: "missing_share_target",
                message: format!("Cloudreve share {} has no file/folder target", share.id),
            }));
        };
        let Some(target) = source.target else {
            return Ok(Conversion::Skipped(SkipReason {
                code: "missing_share_target",
                message: format!(
                    "Cloudreve share {} target {} does not exist",
                    share.id, target_source_id
                ),
            }));
        };
        if target.id != target_source_id {
            bail!(
                "Cloudreve share {} target record {} does not match target {}",
                share.id,
                target.id,
                target_source_id
            );
        }
        let target = match target.r#type {
            0 => MigrationShareTarget::File {
                source_id: target_source_id,
            },
            1 => MigrationShareTarget::Folder {
                source_id: target_source_id,
            },
            target_type => {
                return Ok(Conversion::Skipped(SkipReason {
                    code: "unsupported_share_target",
                    message: format!(
                        "Cloudreve share {} target {} has unsupported type {}",
                        share.id, target_source_id, target_type
                    ),
                }));
            }
        };
        if share.downloads < 0 || share.views < 0 {
            bail!(
                "Cloudreve share {} has negative view or download counters",
                share.id
            );
        }
        let max_downloads = match share.remain_downloads {
            None => 0,
            Some(remaining) if remaining < 0 => {
                bail!(
                    "Cloudreve share {} has negative remaining downloads {}",
                    share.id,
                    remaining
                );
            }
            Some(remaining) => share.downloads.checked_add(remaining).ok_or_else(|| {
                color_eyre::eyre::eyre!("Cloudreve share {} download limit exceeds i64", share.id)
            })?,
        };
        let plain_password = share.password.filter(|password| !password.is_empty());
        Ok(Conversion::Ready(MigrationShare {
            source_id: share.id,
            owner_source_id,
            target,
            plain_password,
            expires_at: share.expires.map(target_time),
            max_downloads,
            download_count: share.downloads,
            view_count: share.views,
            created_at: target_time(share.created_at),
            updated_at: target_time(share.updated_at),
        }))
    }
}
