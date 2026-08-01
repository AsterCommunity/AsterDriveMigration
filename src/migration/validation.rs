use super::*;

pub(super) async fn connect(url: &str, label: &str) -> Result<DatabaseConnection> {
    Database::connect(url)
        .await
        .wrap_err_with(|| format!("connect to {label} database"))
}

pub(super) async fn validate_target_schema(db: &DatabaseConnection) -> Result<()> {
    let history = inspect_migration_history(db)
        .await
        .wrap_err("inspect AsterDrive database migration history")?;
    if history.track != MigrationTrack::Current || !history.effective_pending().is_empty() {
        let pending = history.effective_pending();
        let status = if history.track == MigrationTrack::Unknown {
            "contains an unknown or incompatible migration history".to_string()
        } else {
            format!("is missing current migrations: {}", pending.join(", "))
        };
        bail!(
            "AsterDrive target database {status}; apply the migrations from the matching aster_drive_migration dependency before importing data"
        );
    }
    aster_drive_schema::entities::user::Entity::find()
        .count(db)
        .await
        .wrap_err("AsterDrive schema is unavailable; run AD database migrations first")?;
    aster_drive_schema::entities::storage_policy::Entity::find()
        .count(db)
        .await
        .wrap_err("AsterDrive storage_policies table is unavailable")?;
    aster_drive_schema::entities::file::Entity::find()
        .count(db)
        .await
        .wrap_err("AsterDrive files table is unavailable")?;
    aster_drive_schema::entities::entity_property::Entity::find()
        .count(db)
        .await
        .wrap_err("AsterDrive entity_properties table is unavailable")?;
    aster_drive_schema::entities::tag::Entity::find()
        .count(db)
        .await
        .wrap_err("AsterDrive tags table is unavailable")?;
    Ok(())
}

pub(super) async fn ensure_target_safe(
    db: &DatabaseConnection,
    allow_non_empty: bool,
) -> Result<()> {
    if allow_non_empty {
        return Ok(());
    }
    let counts = [
        (
            "storage_policies",
            aster_drive_schema::entities::storage_policy::Entity::find()
                .count(db)
                .await?,
        ),
        (
            "storage_policy_groups",
            aster_drive_schema::entities::storage_policy_group::Entity::find()
                .count(db)
                .await?,
        ),
        (
            "users",
            aster_drive_schema::entities::user::Entity::find()
                .count(db)
                .await?,
        ),
        (
            "user_profiles",
            aster_drive_schema::entities::user_profile::Entity::find()
                .count(db)
                .await?,
        ),
        (
            "folders",
            aster_drive_schema::entities::folder::Entity::find()
                .count(db)
                .await?,
        ),
        (
            "files",
            aster_drive_schema::entities::file::Entity::find()
                .count(db)
                .await?,
        ),
        (
            "file_blobs",
            aster_drive_schema::entities::file_blob::Entity::find()
                .count(db)
                .await?,
        ),
        (
            "file_versions",
            aster_drive_schema::entities::file_version::Entity::find()
                .count(db)
                .await?,
        ),
        (
            "shares",
            aster_drive_schema::entities::share::Entity::find()
                .count(db)
                .await?,
        ),
        (
            "entity_properties",
            aster_drive_schema::entities::entity_property::Entity::find()
                .count(db)
                .await?,
        ),
        (
            "tags",
            aster_drive_schema::entities::tag::Entity::find()
                .count(db)
                .await?,
        ),
    ];
    let occupied: Vec<String> = counts
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .map(|(table, count)| format!("{table}={count}"))
        .collect();
    if !occupied.is_empty() {
        bail!(
            "target AD database is not empty ({}); use a freshly migrated database or pass --allow-non-empty-target",
            occupied.join(", ")
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct TargetCounts {
    policies: u64,
    policy_groups: u64,
    users: u64,
    user_profiles: u64,
    folders: u64,
    blobs: u64,
    files: u64,
    versions: u64,
    shares: u64,
    properties: u64,
    tags: u64,
}

impl TargetCounts {
    pub(super) async fn load(db: &DatabaseConnection) -> Result<Self> {
        Ok(Self {
            policies: aster_drive_schema::entities::storage_policy::Entity::find()
                .count(db)
                .await?,
            policy_groups: aster_drive_schema::entities::storage_policy_group::Entity::find()
                .count(db)
                .await?,
            users: aster_drive_schema::entities::user::Entity::find()
                .count(db)
                .await?,
            user_profiles: aster_drive_schema::entities::user_profile::Entity::find()
                .count(db)
                .await?,
            folders: aster_drive_schema::entities::folder::Entity::find()
                .count(db)
                .await?,
            blobs: aster_drive_schema::entities::file_blob::Entity::find()
                .count(db)
                .await?,
            files: aster_drive_schema::entities::file::Entity::find()
                .count(db)
                .await?,
            versions: aster_drive_schema::entities::file_version::Entity::find()
                .count(db)
                .await?,
            shares: aster_drive_schema::entities::share::Entity::find()
                .count(db)
                .await?,
            properties: aster_drive_schema::entities::entity_property::Entity::find()
                .count(db)
                .await?,
            tags: aster_drive_schema::entities::tag::Entity::find()
                .count(db)
                .await?,
        })
    }
}

const INTEGRITY_BATCH_SIZE: u64 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum StorageOwner {
    User(i64),
    Team(i64),
}

pub(super) fn add_storage_usage(
    totals: &mut HashMap<StorageOwner, i64>,
    owner: StorageOwner,
    size: i64,
) -> Result<()> {
    let value = totals.entry(owner).or_insert(0);
    *value = value.checked_add(size).ok_or_else(|| {
        color_eyre::eyre::eyre!("storage usage overflow while recalculating {owner:?}")
    })?;
    Ok(())
}

pub(super) fn file_storage_owner(
    file: &aster_drive_schema::entities::file::Model,
) -> Option<StorageOwner> {
    file.team_id
        .map(StorageOwner::Team)
        .or_else(|| file.owner_user_id.map(StorageOwner::User))
}

pub(super) async fn recalculate_statistics(transaction: &DatabaseTransaction) -> Result<()> {
    let mut file_owners = HashMap::new();
    let mut ref_counts = HashMap::<i64, i32>::new();
    let mut usage = HashMap::<StorageOwner, i64>::new();
    let mut last_file_id = 0;
    loop {
        let files = aster_drive_schema::entities::file::Entity::find()
            .filter(aster_drive_schema::entities::file::Column::Id.gt(last_file_id))
            .order_by_asc(aster_drive_schema::entities::file::Column::Id)
            .limit(INTEGRITY_BATCH_SIZE)
            .all(transaction)
            .await?;
        let Some(last_file) = files.last() else { break };
        last_file_id = last_file.id;
        for file in files {
            if let Some(owner) = file_storage_owner(&file) {
                add_storage_usage(&mut usage, owner, file.size)?;
                file_owners.insert(file.id, owner);
            }
            let count = ref_counts.entry(file.blob_id).or_insert(0);
            *count = (*count)
                .checked_add(1)
                .ok_or_else(|| color_eyre::eyre::eyre!("blob reference count exceeds i32"))?;
        }
    }

    let mut last_version_id = 0;
    loop {
        let versions = aster_drive_schema::entities::file_version::Entity::find()
            .filter(aster_drive_schema::entities::file_version::Column::Id.gt(last_version_id))
            .order_by_asc(aster_drive_schema::entities::file_version::Column::Id)
            .limit(INTEGRITY_BATCH_SIZE)
            .all(transaction)
            .await?;
        let Some(last_version) = versions.last() else {
            break;
        };
        last_version_id = last_version.id;
        for version in versions {
            if let Some(owner) = file_owners.get(&version.file_id).copied() {
                add_storage_usage(&mut usage, owner, version.size)?;
            }
            let count = ref_counts.entry(version.blob_id).or_insert(0);
            *count = (*count)
                .checked_add(1)
                .ok_or_else(|| color_eyre::eyre::eyre!("blob reference count exceeds i32"))?;
        }
    }

    let now = chrono::Utc::now();
    let mut last_blob_id = 0;
    loop {
        let blobs = aster_drive_schema::entities::file_blob::Entity::find()
            .filter(aster_drive_schema::entities::file_blob::Column::Id.gt(last_blob_id))
            .order_by_asc(aster_drive_schema::entities::file_blob::Column::Id)
            .limit(INTEGRITY_BATCH_SIZE)
            .all(transaction)
            .await?;
        let Some(last_blob) = blobs.last() else { break };
        last_blob_id = last_blob.id;
        for blob in blobs {
            let actual = ref_counts.get(&blob.id).copied().unwrap_or(0);
            if blob.ref_count != actual {
                let mut active = blob.into_active_model();
                active.ref_count = Set(actual);
                active.updated_at = Set(now);
                active.update(transaction).await?;
            }
        }
    }
    for user in aster_drive_schema::entities::user::Entity::find()
        .all(transaction)
        .await?
    {
        let actual = usage
            .get(&StorageOwner::User(user.id))
            .copied()
            .unwrap_or(0);
        if user.storage_used != actual {
            let mut active = user.into_active_model();
            active.storage_used = Set(actual);
            active.updated_at = Set(now);
            active.update(transaction).await?;
        }
    }
    for team in aster_drive_schema::entities::team::Entity::find()
        .all(transaction)
        .await?
    {
        let actual = usage
            .get(&StorageOwner::Team(team.id))
            .copied()
            .unwrap_or(0);
        if team.storage_used != actual {
            let mut active = team.into_active_model();
            active.storage_used = Set(actual);
            active.updated_at = Set(now);
            active.update(transaction).await?;
        }
    }
    Ok(())
}

pub(super) fn count_check(
    name: &str,
    before: u64,
    migrated: usize,
    actual: u64,
) -> ValidationCheck {
    let migrated = u64::try_from(migrated).unwrap_or(u64::MAX);
    let expected = before.saturating_add(migrated);
    ValidationCheck {
        name: name.to_string(),
        passed: actual == expected,
        expected: expected.to_string(),
        actual: actual.to_string(),
        message: (actual != expected)
            .then(|| format!("expected baseline {before} plus {migrated} migrated records")),
    }
}

pub(super) fn invariant_check(
    name: &str,
    expected: usize,
    actual: usize,
    message: &str,
) -> ValidationCheck {
    ValidationCheck {
        name: name.to_string(),
        passed: actual == expected,
        expected: expected.to_string(),
        actual: actual.to_string(),
        message: (actual != expected).then(|| message.to_string()),
    }
}

pub(super) async fn run_preflight(
    db: &DatabaseConnection,
    source: &SourceData,
) -> Result<MigrationPreflight> {
    let files = cloudreve_schema::files::Entity::find().all(db).await?;
    let entities = cloudreve_schema::entities::Entity::find().all(db).await?;
    let file_entities = cloudreve_schema::file_entities::Entity::find()
        .all(db)
        .await?;
    let user_ids = source
        .users
        .iter()
        .map(|user| user.id)
        .collect::<HashSet<_>>();
    let policy_ids = source
        .policies
        .iter()
        .map(|policy| policy.id)
        .collect::<HashSet<_>>();
    let file_ids = files.iter().map(|file| file.id).collect::<HashSet<_>>();
    let entity_ids = entities
        .iter()
        .map(|entity| entity.id)
        .collect::<HashSet<_>>();
    let folder_ids = source
        .folders
        .iter()
        .map(|folder| folder.id)
        .collect::<HashSet<_>>();
    let folders_by_id = source
        .folders
        .iter()
        .map(|folder| (folder.id, folder))
        .collect::<HashMap<_, _>>();

    let invalid_folders = source
        .folders
        .iter()
        .filter(|folder| {
            !user_ids.contains(&folder.owner_id)
                || folder
                    .file_children
                    .is_some_and(|id| !folder_ids.contains(&id))
                || folder
                    .storage_policy_files
                    .is_some_and(|id| !policy_ids.contains(&id))
        })
        .count();
    let folder_cycles = source
        .folders
        .iter()
        .filter(|folder| source_folder_has_cycle(folder.id, &folders_by_id))
        .count();
    let invalid_files = files
        .iter()
        .filter(|file| {
            !user_ids.contains(&file.owner_id)
                || file
                    .file_children
                    .is_some_and(|id| !folder_ids.contains(&id))
                || file
                    .storage_policy_files
                    .is_some_and(|id| !policy_ids.contains(&id))
                || file
                    .primary_entity
                    .is_some_and(|id| !entity_ids.contains(&id))
                || file.size < 0
        })
        .count();
    let invalid_entities = entities
        .iter()
        .filter(|entity| entity.size < 0 || !policy_ids.contains(&entity.storage_policy_entities))
        .count();
    let invalid_file_entities = file_entities
        .iter()
        .filter(|relation| {
            !file_ids.contains(&relation.file_id) || !entity_ids.contains(&relation.entity_id)
        })
        .count();
    let invalid_metadata = source
        .metadata
        .iter()
        .filter(|metadata| !file_ids.contains(&metadata.file_id))
        .count();
    let invalid_shares = source
        .shares
        .iter()
        .filter(|share| {
            share.user_shares.is_none_or(|id| !user_ids.contains(&id))
                || share.file_shares.is_none_or(|id| !file_ids.contains(&id))
                || share.views < 0
                || share.downloads < 0
                || share.remain_downloads.is_some_and(|value| value < 0)
                || share
                    .remain_downloads
                    .is_some_and(|remaining| share.downloads.checked_add(remaining).is_none())
        })
        .count();
    let invalid_direct_links = source
        .direct_links
        .iter()
        .filter(|link| !file_ids.contains(&link.file_id) || link.downloads < 0 || link.speed < 0)
        .count();
    let duplicate_emails = source
        .users
        .iter()
        .fold(HashMap::<&str, usize>::new(), |mut counts, user| {
            *counts.entry(user.email.as_str()).or_default() += 1;
            counts
        })
        .values()
        .filter(|count| **count > 1)
        .count();

    let checks = vec![
        invariant_check(
            "source_folder_relations",
            0,
            invalid_folders,
            "folders have an orphan owner, parent, or policy",
        ),
        invariant_check(
            "source_folder_cycles",
            0,
            folder_cycles,
            "folders contain parent cycles",
        ),
        invariant_check(
            "source_file_relations",
            0,
            invalid_files,
            "files have an orphan owner, parent, policy, primary entity, or negative size",
        ),
        invariant_check(
            "source_entity_relations",
            0,
            invalid_entities,
            "entities have an orphan policy or negative size",
        ),
        invariant_check(
            "source_file_entity_relations",
            0,
            invalid_file_entities,
            "file_entities contain an orphan file or entity",
        ),
        invariant_check(
            "source_metadata_relations",
            0,
            invalid_metadata,
            "metadata references a missing file",
        ),
        invariant_check(
            "source_share_relations",
            0,
            invalid_shares,
            "shares have a missing owner/target or invalid counters",
        ),
        invariant_check(
            "source_direct_link_relations",
            0,
            invalid_direct_links,
            "direct links have a missing file or invalid counters",
        ),
        invariant_check(
            "source_duplicate_emails",
            0,
            duplicate_emails,
            "active source users have duplicate email addresses",
        ),
    ];
    Ok(MigrationPreflight {
        performed: true,
        passed: checks.iter().all(|check| check.passed),
        checks,
    })
}

pub(super) fn source_folder_has_cycle(
    folder_id: i64,
    folders: &HashMap<i64, &cloudreve_schema::files::Model>,
) -> bool {
    let mut visited = HashSet::new();
    let mut current = Some(folder_id);
    while let Some(id) = current {
        if !visited.insert(id) {
            return true;
        }
        current = folders.get(&id).and_then(|folder| folder.file_children);
    }
    false
}

pub(super) async fn validate_migration_result(
    db: &DatabaseConnection,
    before: &TargetCounts,
    report: &MigrationReport,
    options: &MigrationOptions,
) -> Result<MigrationValidation> {
    let after = TargetCounts::load(db).await?;
    let mut checks = vec![
        count_check(
            "storage_policies_count",
            before.policies,
            report.migrated_policies,
            after.policies,
        ),
        count_check(
            "storage_policy_groups_count",
            before.policy_groups,
            report.migrated_policy_groups,
            after.policy_groups,
        ),
        count_check(
            "users_count",
            before.users,
            report.migrated_users,
            after.users,
        ),
        count_check(
            "user_profiles_count",
            before.user_profiles,
            report.migrated_users,
            after.user_profiles,
        ),
        count_check(
            "folders_count",
            before.folders,
            report.migrated_folders,
            after.folders,
        ),
        count_check(
            "file_blobs_count",
            before.blobs,
            report.migrated_blobs,
            after.blobs,
        ),
        count_check(
            "files_count",
            before.files,
            report.migrated_files,
            after.files,
        ),
        count_check(
            "file_versions_count",
            before.versions,
            report.migrated_versions,
            after.versions,
        ),
        count_check(
            "shares_count",
            before.shares,
            report.migrated_shares,
            after.shares,
        ),
        count_check(
            "entity_properties_count",
            before.properties,
            report.migrated_properties,
            after.properties,
        ),
        count_check("tags_count", before.tags, report.migrated_tags, after.tags),
        invariant_check(
            "policy_mappings_count",
            report.migrated_policies,
            report.mappings.policies.len(),
            "storage policy source-to-target mappings are incomplete",
        ),
        invariant_check(
            "policy_group_mappings_count",
            report.migrated_policy_groups,
            report.mappings.policy_groups.len(),
            "policy group source-to-target mappings are incomplete",
        ),
        invariant_check(
            "user_mappings_count",
            report.migrated_users,
            report.mappings.users.len(),
            "user source-to-target mappings are incomplete",
        ),
        invariant_check(
            "folder_mappings_count",
            report.migrated_folders,
            report.mappings.folders.len(),
            "folder source-to-target mappings are incomplete",
        ),
        invariant_check(
            "blob_mappings_count",
            report.migrated_blobs,
            report.mappings.blobs.len(),
            "blob source-to-target mappings are incomplete",
        ),
        invariant_check(
            "file_mappings_count",
            report.migrated_files,
            report.mappings.files.len(),
            "file source-to-target mappings are incomplete",
        ),
        invariant_check(
            "share_mappings_count",
            report.migrated_shares,
            report.mappings.shares.len(),
            "share source-to-target mappings are incomplete",
        ),
    ];

    let tag_properties = aster_drive_schema::entities::entity_property::Entity::find()
        .filter(aster_drive_schema::entities::entity_property::Column::Namespace.eq("system.tags"))
        .all(db)
        .await?;
    let tag_binding_keys = tag_properties
        .into_iter()
        .map(|property| (property.entity_type, property.entity_id, property.name))
        .collect::<HashSet<_>>();
    let valid_tag_assignments = report
        .tag_assignments
        .iter()
        .filter(|assignment| {
            tag_binding_keys
                .iter()
                .any(|(entity_type, entity_id, name)| {
                    entity_type.as_str() == assignment.target_entity_type
                        && *entity_id == assignment.target_entity_id
                        && *name == assignment.target_tag_id.to_string()
                })
        })
        .count();
    checks.push(invariant_check(
        "tag_assignments_exist",
        report.tag_assignments.len(),
        valid_tag_assignments,
        "one or more system.tags bindings are missing",
    ));

    let direct_link_properties = aster_drive_schema::entities::entity_property::Entity::find()
        .filter(
            aster_drive_schema::entities::entity_property::Column::Namespace
                .eq("cloudreve.direct_links"),
        )
        .all(db)
        .await?;
    let direct_link_values = direct_link_properties
        .into_iter()
        .map(|property| ((property.entity_id, property.name), property.value))
        .collect::<HashMap<_, _>>();
    let valid_direct_links = report
        .direct_links
        .iter()
        .filter(|link| {
            direct_link_values
                .get(&(link.target_file_id, link.source_direct_link_id.to_string()))
                .and_then(|value| value.as_deref())
                .and_then(|value| serde_json::from_str::<Value>(value).ok())
                .and_then(|value| value.get("url").and_then(Value::as_str).map(str::to_string))
                .is_some_and(|url| url == link.url)
        })
        .count();
    checks.push(invariant_check(
        "direct_link_mappings_exist",
        report.direct_links.len(),
        valid_direct_links,
        "one or more cloudreve.direct_links properties are missing or changed",
    ));
    checks.extend(validate_target_integrity(db, options).await?);

    Ok(MigrationValidation {
        performed: true,
        passed: checks.iter().all(|check| check.passed),
        checks,
    })
}

pub(super) async fn validate_target_integrity(
    db: &DatabaseConnection,
    options: &MigrationOptions,
) -> Result<Vec<ValidationCheck>> {
    let users = aster_drive_schema::entities::user::Entity::find()
        .all(db)
        .await?;
    let user_ids = users.iter().map(|user| user.id).collect::<HashSet<_>>();
    let teams = aster_drive_schema::entities::team::Entity::find()
        .all(db)
        .await?;
    let team_ids = teams.iter().map(|team| team.id).collect::<HashSet<_>>();
    let policies = aster_drive_schema::entities::storage_policy::Entity::find()
        .all(db)
        .await?;
    let policies_by_id = policies
        .iter()
        .map(|policy| (policy.id, policy))
        .collect::<HashMap<_, _>>();
    let policy_ids = policies_by_id.keys().copied().collect::<HashSet<_>>();
    let blobs = aster_drive_schema::entities::file_blob::Entity::find()
        .all(db)
        .await?;
    let blob_ids = blobs.iter().map(|blob| blob.id).collect::<HashSet<_>>();
    let folders = aster_drive_schema::entities::folder::Entity::find()
        .all(db)
        .await?;
    let folders_by_id = folders
        .iter()
        .map(|folder| (folder.id, folder))
        .collect::<HashMap<_, _>>();
    let folder_ids = folders_by_id.keys().copied().collect::<HashSet<_>>();
    let files = aster_drive_schema::entities::file::Entity::find()
        .all(db)
        .await?;
    let file_ids = files.iter().map(|file| file.id).collect::<HashSet<_>>();

    let invalid_folders = folders
        .iter()
        .filter(|folder| {
            folder
                .owner_user_id
                .is_some_and(|id| !user_ids.contains(&id))
                || folder
                    .created_by_user_id
                    .is_some_and(|id| !user_ids.contains(&id))
                || folder.team_id.is_some_and(|id| !team_ids.contains(&id))
                || folder.policy_id.is_some_and(|id| !policy_ids.contains(&id))
                || folder.parent_id.is_some_and(|id| !folder_ids.contains(&id))
        })
        .count();
    let mut checks = vec![invariant_check(
        "folder_relations_exist",
        0,
        invalid_folders,
        "folders contain an orphan owner, creator, team, policy, or parent",
    )];

    let folder_cycles = folders
        .iter()
        .filter(|folder| folder_has_cycle(folder.id, &folders_by_id))
        .count();
    checks.push(invariant_check(
        "folder_tree_has_no_cycles",
        0,
        folder_cycles,
        "folders contain one or more parent cycles",
    ));

    let invalid_blobs = blobs
        .iter()
        .filter(|blob| !policy_ids.contains(&blob.policy_id))
        .count();
    checks.push(invariant_check(
        "blob_policies_exist",
        0,
        invalid_blobs,
        "file_blobs contain an orphan storage policy",
    ));

    let invalid_files = files
        .iter()
        .filter(|file| {
            !blob_ids.contains(&file.blob_id)
                || file.folder_id.is_some_and(|id| !folder_ids.contains(&id))
                || file.owner_user_id.is_some_and(|id| !user_ids.contains(&id))
                || file
                    .created_by_user_id
                    .is_some_and(|id| !user_ids.contains(&id))
                || file.team_id.is_some_and(|id| !team_ids.contains(&id))
                || (file.team_id.is_none() && file.owner_user_id.is_none())
        })
        .count();
    checks.push(invariant_check(
        "file_relations_exist",
        0,
        invalid_files,
        "files contain an orphan relation or invalid personal/team scope",
    ));

    let versions = aster_drive_schema::entities::file_version::Entity::find()
        .all(db)
        .await?;
    let invalid_versions = versions
        .iter()
        .filter(|version| {
            !file_ids.contains(&version.file_id) || !blob_ids.contains(&version.blob_id)
        })
        .count();
    checks.push(invariant_check(
        "file_version_relations_exist",
        0,
        invalid_versions,
        "file_versions contain an orphan file or blob",
    ));

    let shares = aster_drive_schema::entities::share::Entity::find()
        .all(db)
        .await?;
    let invalid_shares = shares
        .iter()
        .filter(|share| {
            !user_ids.contains(&share.user_id)
                || share.team_id.is_some_and(|id| !team_ids.contains(&id))
                || (share.file_id.is_some() == share.folder_id.is_some())
                || share.file_id.is_some_and(|id| !file_ids.contains(&id))
                || share.folder_id.is_some_and(|id| !folder_ids.contains(&id))
        })
        .count();
    checks.push(invariant_check(
        "share_relations_exist",
        0,
        invalid_shares,
        "shares contain an orphan owner/target or do not select exactly one target",
    ));

    let (expected_ref_counts, expected_usage) = expected_statistics(&files, &versions)?;
    let ref_count_drifts = blobs
        .iter()
        .filter(|blob| blob.ref_count != expected_ref_counts.get(&blob.id).copied().unwrap_or(0))
        .count();
    checks.push(invariant_check(
        "blob_ref_counts_recalculated",
        0,
        ref_count_drifts,
        "file_blobs.ref_count differs from files plus file_versions references",
    ));
    let user_usage_drifts = users
        .iter()
        .filter(|user| {
            user.storage_used
                != expected_usage
                    .get(&StorageOwner::User(user.id))
                    .copied()
                    .unwrap_or(0)
        })
        .count();
    let team_usage_drifts = teams
        .iter()
        .filter(|team| {
            team.storage_used
                != expected_usage
                    .get(&StorageOwner::Team(team.id))
                    .copied()
                    .unwrap_or(0)
        })
        .count();
    checks.push(invariant_check(
        "storage_usage_recalculated",
        0,
        user_usage_drifts + team_usage_drifts,
        "users.storage_used or teams.storage_used differs from current files plus historical versions",
    ));
    if options.verify_local_storage {
        checks.push(verify_local_runtime_readability(&blobs, &policies_by_id));
    }
    Ok(checks)
}

pub(super) fn expected_statistics(
    files: &[aster_drive_schema::entities::file::Model],
    versions: &[aster_drive_schema::entities::file_version::Model],
) -> Result<(HashMap<i64, i32>, HashMap<StorageOwner, i64>)> {
    let mut refs = HashMap::<i64, i32>::new();
    let mut usage = HashMap::new();
    let mut owners = HashMap::new();
    for file in files {
        let count = refs.entry(file.blob_id).or_insert(0);
        *count = (*count)
            .checked_add(1)
            .ok_or_else(|| color_eyre::eyre::eyre!("blob reference count exceeds i32"))?;
        if let Some(owner) = file_storage_owner(file) {
            add_storage_usage(&mut usage, owner, file.size)?;
            owners.insert(file.id, owner);
        }
    }
    for version in versions {
        let count = refs.entry(version.blob_id).or_insert(0);
        *count = (*count)
            .checked_add(1)
            .ok_or_else(|| color_eyre::eyre::eyre!("blob reference count exceeds i32"))?;
        if let Some(owner) = owners.get(&version.file_id).copied() {
            add_storage_usage(&mut usage, owner, version.size)?;
        }
    }
    Ok((refs, usage))
}

pub(super) fn folder_has_cycle(
    folder_id: i64,
    folders: &HashMap<i64, &aster_drive_schema::entities::folder::Model>,
) -> bool {
    let mut visited = HashSet::new();
    let mut current = Some(folder_id);
    while let Some(id) = current {
        if !visited.insert(id) {
            return true;
        }
        current = folders.get(&id).and_then(|folder| folder.parent_id);
    }
    false
}

pub(super) fn verify_local_runtime_readability(
    blobs: &[aster_drive_schema::entities::file_blob::Model],
    policies: &HashMap<i64, &aster_drive_schema::entities::storage_policy::Model>,
) -> ValidationCheck {
    let mut checked = 0usize;
    let mut failed = 0usize;
    let mut failures = Vec::new();
    for blob in blobs {
        let Some(policy) = policies.get(&blob.policy_id) else {
            continue;
        };
        if policy.driver_type != DriverType::Local {
            continue;
        }
        checked += 1;
        let path = local_storage_path(&policy.base_path, &blob.storage_path);
        let result: Result<()> = (|| {
            let metadata = std::fs::metadata(&path)?;
            if !metadata.is_file() || metadata.len() != u64::try_from(blob.size)? {
                bail!("not a regular file with the expected size");
            }
            let mut file = std::fs::File::open(&path)?;
            if metadata.len() > 0 {
                let mut byte = [0_u8; 1];
                file.read_exact(&mut byte)?;
                file.seek(SeekFrom::End(-1))?;
                file.read_exact(&mut byte)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            failed += 1;
            if failures.len() < 3 {
                failures.push(format!("blob {} at {}: {error}", blob.id, path.display()));
            }
        }
    }
    ValidationCheck {
        name: "local_storage_runtime_readability".to_string(),
        passed: failed == 0,
        expected: checked.to_string(),
        actual: (checked - failed).to_string(),
        message: (failed > 0).then(|| failures.join("; ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(id: i64, parent_id: Option<i64>) -> aster_drive_schema::entities::folder::Model {
        let now = chrono::Utc::now();
        aster_drive_schema::entities::folder::Model {
            id,
            name: format!("folder-{id}"),
            parent_id,
            team_id: None,
            owner_user_id: Some(1),
            created_by_user_id: Some(1),
            created_by_username: "owner".to_string(),
            policy_id: None,
            created_at: now,
            updated_at: now,
            deleted_at: None,
            is_locked: false,
        }
    }

    #[test]
    fn folder_cycle_detection_handles_roots_missing_parents_self_and_multi_node_cycles() {
        let root = folder(1, None);
        let child = folder(2, Some(1));
        let missing_parent = folder(3, Some(99));
        let self_cycle = folder(4, Some(4));
        let cycle_a = folder(5, Some(6));
        let cycle_b = folder(6, Some(5));
        let folders = [
            &root,
            &child,
            &missing_parent,
            &self_cycle,
            &cycle_a,
            &cycle_b,
        ]
        .into_iter()
        .map(|folder| (folder.id, folder))
        .collect::<HashMap<_, _>>();

        assert!(!folder_has_cycle(1, &folders));
        assert!(!folder_has_cycle(2, &folders));
        assert!(!folder_has_cycle(3, &folders));
        assert!(folder_has_cycle(4, &folders));
        assert!(folder_has_cycle(5, &folders));
        assert!(folder_has_cycle(6, &folders));
    }

    #[test]
    fn storage_usage_accumulation_covers_zero_negative_and_overflow_values() -> Result<()> {
        let owner = StorageOwner::User(1);
        let mut totals = HashMap::new();
        add_storage_usage(&mut totals, owner, 0)?;
        add_storage_usage(&mut totals, owner, 5)?;
        add_storage_usage(&mut totals, owner, -2)?;
        assert_eq!(totals[&owner], 3);

        totals.insert(owner, i64::MAX);
        let error =
            add_storage_usage(&mut totals, owner, 1).expect_err("i64 overflow must be rejected");
        assert!(error.to_string().contains("storage usage overflow"));
        Ok(())
    }

    #[test]
    fn invariant_check_reports_exact_expected_and_actual_values() {
        let passed = invariant_check("check", 0, 0, "drift");
        assert!(passed.passed);
        assert_eq!(passed.expected, "0");
        assert_eq!(passed.actual, "0");
        assert_eq!(passed.message, None);

        let failed = invariant_check("check", 0, 2, "drift");
        assert!(!failed.passed);
        assert_eq!(failed.message.as_deref(), Some("drift"));
    }
}
