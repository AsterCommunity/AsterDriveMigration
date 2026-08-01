use super::*;
use sea_orm::DatabaseTransaction;

use aster_drive_migration_core::{
    Conversion, ConversionContext, MigrationEntityKind, MigrationEntityRef, MigrationMetadata,
    MigrationShareTarget, SourceConverter,
};
use aster_drive_writer::{
    AsterDriveWriter, ResolvedBlob, ResolvedDirectLink, ResolvedEntityTarget, ResolvedFile,
    ResolvedFileVersion, ResolvedFolder, ResolvedPolicyGroup, ResolvedProperty, ResolvedShare,
    ResolvedShareTarget, ResolvedTag, ResolvedTagAssignment, ResolvedUser,
};
use cloudreve_adapter::{
    CloudreveBlobRecord, CloudreveConverter, CloudreveDirectLinkRecord, CloudreveFileRecord,
    CloudreveFolderRecord, CloudreveMetadataRecord, CloudrevePolicyGroupRecord,
    CloudreveShareRecord, CloudreveStoragePolicyRecord, CloudreveUserRecord,
};

pub(super) async fn migrate_policies(
    transaction: &DatabaseTransaction,
    source: &SourceData,
    options: &MigrationOptions,
    context: &mut MigrationContext,
    report: &mut MigrationReport,
) -> Result<()> {
    let converter = CloudreveConverter;
    let conversion_context = ConversionContext;
    let writer = AsterDriveWriter::new(transaction);
    let mut default_assigned = false;
    for policy in &source.policies {
        let local_root = if policy.r#type == "local" {
            Some(local_policy_root(options, policy.id).to_string())
        } else {
            None
        };
        match converter.convert(
            CloudreveStoragePolicyRecord {
                policy: policy.clone(),
                local_root,
            },
            &conversion_context,
        )? {
            Conversion::Ready(converted) => {
                let source_id = converted.source_id;
                let target_id = writer.write_policy(converted, !default_assigned).await?;
                default_assigned = true;
                context.policies.insert(source_id, target_id);
                report.migrated_policies += 1;
            }
            Conversion::Skipped(reason) => {
                report.record_skip("storage_policy", Some(policy.id), reason.message);
            }
        }
    }
    Ok(())
}

pub(super) async fn migrate_policy_groups(
    transaction: &DatabaseTransaction,
    source: &SourceData,
    context: &mut MigrationContext,
    report: &mut MigrationReport,
) -> Result<()> {
    let converter = CloudreveConverter;
    let conversion_context = ConversionContext;
    let writer = AsterDriveWriter::new(transaction);
    for group in &source.groups {
        let converted = match converter.convert(
            CloudrevePolicyGroupRecord {
                group: group.clone(),
            },
            &conversion_context,
        )? {
            Conversion::Ready(converted) => converted,
            Conversion::Skipped(reason) => bail!(
                "Cloudreve policy group {} was unexpectedly skipped: {}",
                group.id,
                reason.message
            ),
        };
        let source_id = converted.source_id;
        let policy_id = converted
            .policy_source_id
            .and_then(|source_id| context.policies.get(&source_id).copied());
        let target_id = writer
            .write_policy_group(ResolvedPolicyGroup {
                group: converted,
                policy_id,
            })
            .await?;
        context.policy_groups.insert(source_id, target_id);
        report.migrated_policy_groups += 1;
    }
    Ok(())
}

pub(super) async fn migrate_users(
    transaction: &DatabaseTransaction,
    source: &SourceData,
    password_hash: &str,
    context: &mut MigrationContext,
    report: &mut MigrationReport,
) -> Result<()> {
    let groups: HashMap<i64, &cloudreve_schema::groups::Model> = source
        .groups
        .iter()
        .map(|group| (group.id, group))
        .collect();
    let converter = CloudreveConverter;
    let conversion_context = ConversionContext;
    let writer = AsterDriveWriter::new(transaction);
    let mut used_usernames = HashSet::new();
    for user in &source.users {
        let username = unique_username(&user.nick, user.id, &mut used_usernames);
        let converted = match converter.convert(
            CloudreveUserRecord {
                user: user.clone(),
                group: groups.get(&user.group_users).map(|group| (*group).clone()),
                username: username.clone(),
            },
            &conversion_context,
        )? {
            Conversion::Ready(converted) => converted,
            Conversion::Skipped(reason) => bail!(
                "Cloudreve user {} was unexpectedly skipped: {}",
                user.id,
                reason.message
            ),
        };
        let source_id = converted.source_id;
        let policy_group_id = context
            .policy_groups
            .get(&converted.policy_group_source_id)
            .copied();
        let target_id = writer
            .write_user(
                ResolvedUser {
                    user: converted,
                    policy_group_id,
                },
                password_hash,
            )
            .await?;
        context.users.insert(source_id, target_id);
        context.usernames.insert(source_id, username);
        report.migrated_users += 1;
    }
    Ok(())
}

pub(super) async fn migrate_folders(
    transaction: &DatabaseTransaction,
    source: &SourceData,
    context: &mut MigrationContext,
    report: &mut MigrationReport,
) -> Result<()> {
    let converter = CloudreveConverter;
    let conversion_context = ConversionContext;
    let writer = AsterDriveWriter::new(transaction);
    let mut pending = source
        .folders
        .iter()
        .map(|folder| {
            converter
                .convert(
                    CloudreveFolderRecord {
                        folder: folder.clone(),
                    },
                    &conversion_context,
                )
                .and_then(|conversion| {
                    conversion.into_ready().ok_or_else(|| {
                        color_eyre::eyre::eyre!("source folder query returned a non-folder row")
                    })
                })
        })
        .collect::<Result<Vec<_>>>()?;
    pending.sort_by_key(|folder| folder.source_id);

    while !pending.is_empty() {
        let mut progress = false;
        let mut next = Vec::new();
        for folder in pending {
            if folder.parent_source_id.is_some_and(|parent_id| {
                source
                    .folders
                    .iter()
                    .any(|candidate| candidate.id == parent_id && candidate.r#type == 1)
                    && !context.folders.contains_key(&parent_id)
            }) {
                next.push(folder);
                continue;
            }
            let source_id = folder.source_id;
            let owner_source_id = folder.owner_source_id;
            if !context.users.contains_key(&owner_source_id) {
                report.record_skip(
                    "folder",
                    Some(source_id),
                    format!("owner user {owner_source_id} was not migrated"),
                );
                continue;
            }
            let owner_id = context.users[&owner_source_id];
            let resolved = ResolvedFolder {
                parent_id: folder
                    .parent_source_id
                    .and_then(|id| context.folders.get(&id).copied()),
                owner_id,
                owner_username: context
                    .usernames
                    .get(&owner_source_id)
                    .cloned()
                    .unwrap_or_default(),
                policy_id: folder
                    .policy_source_id
                    .and_then(|id| context.policies.get(&id).copied()),
                folder,
            };
            let target_id = writer.write_folder(resolved).await?;
            context.folders.insert(source_id, target_id);
            report.migrated_folders += 1;
            progress = true;
        }
        if !progress && !next.is_empty() {
            bail!("Cloudreve folder hierarchy contains a cycle or a missing parent");
        }
        pending = next;
    }
    Ok(())
}

pub(super) async fn migrate_blob_batch(
    transaction: &DatabaseTransaction,
    entities: &[cloudreve_schema::entities::Model],
    reference_counts: &HashMap<i64, i64>,
    source: &SourceData,
    options: &MigrationOptions,
    context: &MigrationContext,
    report: &mut MigrationReport,
) -> Result<Vec<(i64, i64)>> {
    let mut mappings = Vec::with_capacity(entities.len());
    let converter = CloudreveConverter;
    let conversion_context = ConversionContext;
    let writer = AsterDriveWriter::new(transaction);
    for entity in entities {
        if entity.upload_session_id.is_some() {
            report.record_skip(
                "blob",
                Some(entity.id),
                "incomplete Cloudreve upload has no committed file version",
            );
            continue;
        }
        let Some(reference_count) = reference_counts.get(&entity.id).copied() else {
            report.record_skip(
                "blob",
                Some(entity.id),
                "Cloudreve entity is not referenced by a migratable file",
            );
            continue;
        };
        let local_root = context
            .policies
            .contains_key(&entity.storage_policy_entities)
            .then(|| {
                source
                    .policies
                    .iter()
                    .find(|policy| {
                        policy.id == entity.storage_policy_entities && policy.r#type == "local"
                    })
                    .map(|policy| local_policy_root(options, policy.id).to_string())
            })
            .flatten();
        let conversion = converter.convert(
            CloudreveBlobRecord {
                entity: entity.clone(),
                reference_count,
                local_root,
            },
            &conversion_context,
        )?;
        let blob = match conversion {
            Conversion::Ready(blob) => blob,
            Conversion::Skipped(reason) => {
                report.record_skip("blob", Some(entity.id), reason.message);
                continue;
            }
        };
        let Some(policy_id) = context.policies.get(&blob.policy_source_id).copied() else {
            report.record_skip(
                "blob",
                Some(blob.source_id),
                format!("storage policy {} was not migrated", blob.policy_source_id),
            );
            continue;
        };
        let source_id = blob.source_id;
        let target_id = writer.write_blob(ResolvedBlob { blob, policy_id }).await?;
        mappings.push((source_id, target_id));
        report.migrated_blobs += 1;
    }
    Ok(mappings)
}

pub(super) async fn migrate_file_batch(
    transaction: &DatabaseTransaction,
    files: &[cloudreve_schema::files::Model],
    associations: &HashMap<i64, Vec<i64>>,
    entities: &HashMap<i64, cloudreve_schema::entities::Model>,
    blob_mappings: &HashMap<i64, i64>,
    context: &MigrationContext,
    report: &mut MigrationReport,
) -> Result<Vec<(i64, i64)>> {
    let mut mappings = Vec::with_capacity(files.len());
    let converter = CloudreveConverter;
    let conversion_context = ConversionContext;
    let writer = AsterDriveWriter::new(transaction);
    for file in files {
        let mut source_entity_ids = associations.get(&file.id).cloned().unwrap_or_default();
        if let Some(primary_entity) = file.primary_entity
            && !source_entity_ids.contains(&primary_entity)
        {
            source_entity_ids.push(primary_entity);
        }
        let conversion = converter.convert(
            CloudreveFileRecord {
                file: file.clone(),
                entities: source_entity_ids
                    .into_iter()
                    .filter_map(|entity_id| entities.get(&entity_id).cloned())
                    .collect(),
            },
            &conversion_context,
        )?;
        let migrated_file = match conversion {
            Conversion::Ready(file) => file,
            Conversion::Skipped(reason) => {
                report.record_skip("file", Some(file.id), reason.message);
                continue;
            }
        };
        let Some(owner_id) = context.users.get(&migrated_file.owner_source_id).copied() else {
            report.record_skip(
                "file",
                Some(migrated_file.source_id),
                format!(
                    "owner user {} was not migrated",
                    migrated_file.owner_source_id
                ),
            );
            continue;
        };
        let Some(primary_entity_id) = migrated_file.preferred_blob_source_id else {
            bail!(
                "converted Cloudreve file {} has no primary entity",
                migrated_file.source_id
            );
        };
        if entities
            .get(&primary_entity_id)
            .is_some_and(is_encrypted_entity)
        {
            report.record_skip(
                "file",
                Some(migrated_file.source_id),
                format!("current entity {primary_entity_id} is encrypted by Cloudreve"),
            );
            continue;
        }
        if !blob_mappings.contains_key(&primary_entity_id) {
            report.record_skip(
                "file",
                Some(migrated_file.source_id),
                format!("current entity {primary_entity_id} was not migrated"),
            );
            report.warnings.push(format!(
                "file {} current entity {primary_entity_id} was not migrated",
                migrated_file.source_id,
            ));
            continue;
        }
        let historical_versions = migrated_file
            .versions
            .iter()
            .filter(|version| version.blob_source_id != primary_entity_id)
            .filter_map(|version| {
                blob_mappings
                    .get(&version.blob_source_id)
                    .copied()
                    .map(|blob_id| ResolvedFileVersion {
                        blob_id,
                        size: version.size,
                        created_at: version.created_at,
                    })
            })
            .collect();
        let source_id = migrated_file.source_id;
        let Some(primary_blob_size) = migrated_file
            .versions
            .iter()
            .find(|version| version.blob_source_id == primary_entity_id)
            .map(|version| version.size)
        else {
            bail!(
                "converted Cloudreve file {} is missing current entity {primary_entity_id} in its version set",
                migrated_file.source_id
            );
        };
        let resolved = ResolvedFile {
            folder_id: migrated_file
                .folder_source_id
                .and_then(|folder_id| context.folders.get(&folder_id).copied()),
            owner_id,
            owner_username: context
                .usernames
                .get(&migrated_file.owner_source_id)
                .cloned()
                .unwrap_or_default(),
            primary_blob_id: blob_mappings[&primary_entity_id],
            primary_blob_size,
            historical_versions,
            file: migrated_file,
        };
        let written = writer.write_file(resolved).await?;
        mappings.push((source_id, written.target_id));
        report.migrated_files += 1;
        report.migrated_versions += written.version_count;
    }
    Ok(mappings)
}

pub(super) async fn migrate_metadata(
    transaction: &DatabaseTransaction,
    source_db: &DatabaseConnection,
    source: &SourceData,
    file_mappings: &HashMap<i64, i64>,
    context: &MigrationContext,
    report: &mut MigrationReport,
) -> Result<()> {
    #[derive(Debug)]
    struct WrittenTag {
        id: i64,
        name: String,
        color: String,
        source_metadata_id: i64,
        conflict_reported: bool,
    }

    let source_file_ids = source
        .metadata
        .iter()
        .map(|metadata| metadata.file_id)
        .collect::<Vec<_>>();
    let source_files = load_source_files(source_db, &source_file_ids).await?;
    let converter = CloudreveConverter;
    let conversion_context = ConversionContext;
    let writer = AsterDriveWriter::new(transaction);
    let mut tags: HashMap<(i64, String), WrittenTag> = HashMap::new();
    let mut assignments = HashSet::new();
    let mut metadata_rows = source.metadata.iter().collect::<Vec<_>>();
    metadata_rows.sort_by_key(|metadata| (metadata.created_at, metadata.id));

    for metadata in metadata_rows {
        let conversion = converter.convert(
            CloudreveMetadataRecord {
                metadata: metadata.clone(),
                target: source_files.get(&metadata.file_id).cloned(),
            },
            &conversion_context,
        )?;
        let converted = match conversion {
            Conversion::Ready(converted) => converted,
            Conversion::Skipped(reason) => {
                report.record_skip("metadata", Some(metadata.id), reason.message);
                continue;
            }
        };

        match converted {
            MigrationMetadata::Property(property) => {
                let Some((target, _, _)) =
                    resolve_metadata_target(property.target, file_mappings, &context.folders)
                else {
                    report.record_skip(
                        "metadata",
                        Some(property.source_metadata_id),
                        format!(
                            "source entity {} was not migrated",
                            property.target.source_id
                        ),
                    );
                    continue;
                };
                writer
                    .write_property(ResolvedProperty {
                        source_metadata_id: property.source_metadata_id,
                        target,
                        namespace: property.namespace,
                        name: property.name,
                        value: property.value,
                    })
                    .await?;
                report.migrated_properties += 1;
            }
            MigrationMetadata::TagAssignment(tag) => {
                let Some(owner_user_id) = context.users.get(&tag.owner_source_id).copied() else {
                    report.record_skip(
                        "tag_assignment",
                        Some(tag.source_metadata_id),
                        format!("owner user {} was not migrated", tag.owner_source_id),
                    );
                    continue;
                };
                let Some((target, entity_type, entity_id)) =
                    resolve_metadata_target(tag.target, file_mappings, &context.folders)
                else {
                    report.record_skip(
                        "tag_assignment",
                        Some(tag.source_metadata_id),
                        format!("source entity {} was not migrated", tag.target.source_id),
                    );
                    continue;
                };

                let tag_key = (owner_user_id, tag.normalized_name.clone());
                let tag_id = if let Some(written) = tags.get_mut(&tag_key) {
                    if (written.name != tag.name || written.color != tag.color)
                        && !written.conflict_reported
                    {
                        report.warnings.push(format!(
                            "Cloudreve metadata {} and {} define personal tag '{}' with conflicting display names or colors; the earliest definition '{}' ({}) is used",
                            written.source_metadata_id,
                            tag.source_metadata_id,
                            tag.normalized_name,
                            written.name,
                            written.color
                        ));
                        written.conflict_reported = true;
                    }
                    written.id
                } else {
                    let tag_id = writer
                        .write_tag(ResolvedTag {
                            source_metadata_id: tag.source_metadata_id,
                            owner_id: owner_user_id,
                            name: tag.name.clone(),
                            normalized_name: tag.normalized_name.clone(),
                            color: tag.color.clone(),
                            created_at: tag.created_at,
                            updated_at: tag.updated_at,
                        })
                        .await?;
                    tags.insert(
                        tag_key,
                        WrittenTag {
                            id: tag_id,
                            name: tag.name.clone(),
                            color: tag.color.clone(),
                            source_metadata_id: tag.source_metadata_id,
                            conflict_reported: false,
                        },
                    );
                    report.migrated_tags += 1;
                    tag_id
                };

                if !assignments.insert((target, tag_id)) {
                    report.record_skip(
                        "tag_assignment",
                        Some(tag.source_metadata_id),
                        format!(
                            "source entity {} already has the normalized tag '{}'",
                            tag.target.source_id, tag.normalized_name
                        ),
                    );
                    continue;
                }
                writer
                    .write_tag_assignment(ResolvedTagAssignment {
                        source_metadata_id: tag.source_metadata_id,
                        target,
                        tag_id,
                    })
                    .await?;
                report.migrated_properties += 1;
                report.migrated_tag_assignments += 1;
                report.tag_assignments.push(TagAssignmentReport {
                    source_metadata_id: tag.source_metadata_id,
                    source_entity_id: tag.target.source_id,
                    target_entity_type: entity_type.to_string(),
                    target_entity_id: entity_id,
                    target_tag_id: tag_id,
                    tag_name: tag.name,
                });
            }
        }
    }
    Ok(())
}

fn resolve_metadata_target(
    target: MigrationEntityRef,
    file_mappings: &HashMap<i64, i64>,
    folder_mappings: &HashMap<i64, i64>,
) -> Option<(ResolvedEntityTarget, &'static str, i64)> {
    match target.kind {
        MigrationEntityKind::File => {
            let target_id = file_mappings.get(&target.source_id).copied()?;
            Some((ResolvedEntityTarget::File { target_id }, "file", target_id))
        }
        MigrationEntityKind::Folder => {
            let target_id = folder_mappings.get(&target.source_id).copied()?;
            Some((
                ResolvedEntityTarget::Folder { target_id },
                "folder",
                target_id,
            ))
        }
    }
}

pub(super) async fn migrate_direct_links(
    transaction: &DatabaseTransaction,
    source_db: &DatabaseConnection,
    source: &SourceData,
    file_mappings: &HashMap<i64, i64>,
    context: &MigrationContext,
    direct_link_secret: Option<&str>,
    report: &mut MigrationReport,
) -> Result<()> {
    if source.direct_links.is_empty() {
        return Ok(());
    }
    let source_file_ids = source
        .direct_links
        .iter()
        .map(|link| link.file_id)
        .collect::<Vec<_>>();
    let source_files = load_source_files(source_db, &source_file_ids).await?;
    let converter = CloudreveConverter;
    let conversion_context = ConversionContext;
    let writer = AsterDriveWriter::new(transaction);
    let mut missing_secret_count = 0_usize;
    for link in &source.direct_links {
        let conversion = converter.convert(
            CloudreveDirectLinkRecord {
                direct_link: link.clone(),
                target: source_files.get(&link.file_id).cloned(),
            },
            &conversion_context,
        )?;
        let direct_link = match conversion {
            Conversion::Ready(direct_link) => direct_link,
            Conversion::Skipped(reason) => {
                report.record_skip("direct_link", Some(link.id), reason.message);
                continue;
            }
        };
        let Some(file_id) = file_mappings.get(&direct_link.file_source_id).copied() else {
            report.record_skip(
                "direct_link",
                Some(direct_link.source_id),
                format!(
                    "source file {} was not migrated",
                    direct_link.file_source_id
                ),
            );
            continue;
        };
        let Some(owner_user_id) = context.users.get(&direct_link.owner_source_id).copied() else {
            report.record_skip(
                "direct_link",
                Some(direct_link.source_id),
                format!(
                    "owner user {} was not migrated",
                    direct_link.owner_source_id
                ),
            );
            continue;
        };
        let Some(secret) = direct_link_secret else {
            missing_secret_count += 1;
            report.record_skip(
                "direct_link",
                Some(direct_link.source_id),
                "AD direct_link_secret was not supplied",
            );
            continue;
        };
        let source_file_id = direct_link.file_source_id;
        let source_name = direct_link.source_name.clone();
        let source_downloads = direct_link.source_downloads;
        let source_speed_limit = direct_link.source_speed_limit;
        let source_direct_link_id = direct_link.source_id;
        let written = writer
            .write_direct_link(
                ResolvedDirectLink {
                    direct_link,
                    target_file_id: file_id,
                    target_owner_id: owner_user_id,
                },
                secret,
            )
            .await?;
        report.migrated_properties += 1;
        report.migrated_direct_links += 1;
        report.direct_links.push(DirectLinkReport {
            source_direct_link_id,
            source_file_id,
            target_file_id: file_id,
            source_name,
            source_downloads,
            source_speed_limit,
            url: written.url,
        });
    }
    if missing_secret_count > 0 {
        report.warnings.push(format!(
            "{missing_secret_count} Cloudreve direct links were not regenerated because --direct-link-secret was not supplied"
        ));
    }
    Ok(())
}

pub(super) async fn migrate_shares(
    transaction: &DatabaseTransaction,
    source_db: &DatabaseConnection,
    source: &SourceData,
    file_mappings: &HashMap<i64, i64>,
    context: &mut MigrationContext,
    report: &mut MigrationReport,
) -> Result<()> {
    let source_file_ids = source
        .shares
        .iter()
        .filter_map(|share| share.file_shares)
        .collect::<Vec<_>>();
    let source_files = load_source_files(source_db, &source_file_ids).await?;
    let converter = CloudreveConverter;
    let conversion_context = ConversionContext;
    let writer = AsterDriveWriter::new(transaction);
    for share in &source.shares {
        let conversion = converter.convert(
            CloudreveShareRecord {
                share: share.clone(),
                target: share
                    .file_shares
                    .and_then(|source_id| source_files.get(&source_id).cloned()),
            },
            &conversion_context,
        )?;
        let migrated_share = match conversion {
            Conversion::Ready(share) => share,
            Conversion::Skipped(reason) => {
                report.record_skip("share", Some(share.id), reason.message);
                continue;
            }
        };
        let Some(user_id) = context.users.get(&migrated_share.owner_source_id).copied() else {
            report.record_skip(
                "share",
                Some(share.id),
                format!(
                    "owner user {} was not migrated",
                    migrated_share.owner_source_id
                ),
            );
            continue;
        };
        let resolved_target = match migrated_share.target {
            MigrationShareTarget::File { source_id } => file_mappings
                .get(&source_id)
                .copied()
                .map(|target_id| ResolvedShareTarget::File { target_id }),
            MigrationShareTarget::Folder { source_id } => context
                .folders
                .get(&source_id)
                .copied()
                .map(|target_id| ResolvedShareTarget::Folder { target_id }),
        };
        let Some(resolved_target) = resolved_target else {
            let source_target_id = match migrated_share.target {
                MigrationShareTarget::File { source_id }
                | MigrationShareTarget::Folder { source_id } => source_id,
            };
            report.record_skip(
                "share",
                Some(share.id),
                format!("source target {source_target_id} was not migrated"),
            );
            continue;
        };
        let password_hash = match migrated_share.plain_password.as_deref() {
            Some(password) => Some(hash_argon2_password(password)?),
            None => None,
        };
        let source_id = migrated_share.source_id;
        let target_id = writer
            .write_share(ResolvedShare {
                source_id,
                owner_id: user_id,
                target: resolved_target,
                password_hash,
                expires_at: migrated_share.expires_at,
                max_downloads: migrated_share.max_downloads,
                download_count: migrated_share.download_count,
                view_count: migrated_share.view_count,
                created_at: migrated_share.created_at,
                updated_at: migrated_share.updated_at,
            })
            .await?;
        context.shares.insert(source_id, target_id);
        report.migrated_shares += 1;
    }
    Ok(())
}

pub(super) async fn load_source_files(
    source_db: &DatabaseConnection,
    source_ids: &[i64],
) -> Result<HashMap<i64, cloudreve_schema::files::Model>> {
    const QUERY_ID_BATCH_SIZE: usize = 500;

    let source_ids = source_ids.iter().copied().collect::<HashSet<_>>();
    let mut files = HashMap::with_capacity(source_ids.len());
    let source_ids = source_ids.into_iter().collect::<Vec<_>>();
    for source_ids in source_ids.chunks(QUERY_ID_BATCH_SIZE) {
        for file in cloudreve_schema::files::Entity::find()
            .filter(cloudreve_schema::files::Column::Id.is_in(source_ids.iter().copied()))
            .all(source_db)
            .await?
        {
            files.insert(file.id, file);
        }
    }
    Ok(files)
}

pub(super) fn associations(
    files: &[cloudreve_schema::files::Model],
    file_entities: &[cloudreve_schema::file_entities::Model],
) -> HashMap<i64, Vec<i64>> {
    let mut result: HashMap<i64, Vec<i64>> = HashMap::new();
    for relation in file_entities {
        result
            .entry(relation.file_id)
            .or_default()
            .push(relation.entity_id);
    }
    for file in files {
        if let Some(primary_entity) = file.primary_entity {
            let values = result.entry(file.id).or_default();
            if !values.contains(&primary_entity) {
                values.push(primary_entity);
            }
        }
    }
    result
}
