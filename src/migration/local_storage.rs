use super::*;

pub(super) fn local_policy_root(options: &MigrationOptions, source_policy_id: i64) -> &str {
    options
        .local_policy_roots
        .get(&source_policy_id)
        .map(String::as_str)
        .unwrap_or(&options.local_base_path)
}

pub(super) fn local_storage_path(root: &str, storage_path: &str) -> std::path::PathBuf {
    let storage_path = std::path::Path::new(storage_path);
    if storage_path.is_absolute() {
        storage_path.to_path_buf()
    } else {
        std::path::Path::new(root).join(storage_path)
    }
}

pub(super) fn validate_local_policy_roots(
    source: &SourceData,
    options: &MigrationOptions,
) -> Result<()> {
    let policies = source
        .policies
        .iter()
        .map(|policy| (policy.id, policy))
        .collect::<HashMap<_, _>>();
    for source_policy_id in options.local_policy_roots.keys() {
        let Some(policy) = policies.get(source_policy_id) else {
            bail!(
                "local policy root references Cloudreve policy {source_policy_id}, which was not found"
            );
        };
        if policy.r#type != "local" {
            bail!(
                "local policy root references Cloudreve policy {source_policy_id}, which is not a local policy"
            );
        }
    }
    Ok(())
}

pub(super) fn verify_local_storage_roots(
    source: &SourceData,
    options: &MigrationOptions,
) -> Result<()> {
    for policy in source.policies.iter().filter(|policy| {
        policy.r#type == "local"
            && storage_policy_skip_reason(policy).is_none()
            && !source_settings(&policy.settings)
                .get("encryption")
                .and_then(Value::as_bool)
                .unwrap_or(false)
    }) {
        let root = local_policy_root(options, policy.id);
        let metadata = std::fs::metadata(root).wrap_err_with(|| {
            format!(
                "read local storage root for Cloudreve policy {}: {root}",
                policy.id
            )
        })?;
        if !metadata.is_dir() {
            bail!(
                "local storage root for Cloudreve policy {} is not a directory: {root}",
                policy.id
            );
        }
    }
    Ok(())
}

pub(super) async fn validate_all_local_source_objects(
    source_db: &DatabaseConnection,
    source: &SourceData,
    options: &MigrationOptions,
    verify_storage: bool,
) -> Result<()> {
    const VERIFY_BATCH_SIZE: u64 = 500;

    let policies = source
        .policies
        .iter()
        .map(|policy| (policy.id, policy))
        .collect::<HashMap<_, _>>();
    let mut cursor_value = 0;
    loop {
        let mut query = cloudreve_schema::entities::Entity::find()
            .filter(cloudreve_schema::entities::Column::Type.eq(0))
            .filter(cloudreve_schema::entities::Column::Id.gt(cursor_value))
            .order_by_asc(cloudreve_schema::entities::Column::Id)
            .limit(VERIFY_BATCH_SIZE);
        if !source.include_deleted {
            query = query.filter(cloudreve_schema::entities::Column::DeletedAt.is_null());
        }
        let entities = query.all(source_db).await?;
        let Some(last_entity) = entities.last() else {
            return Ok(());
        };
        for entity in &entities {
            if is_encrypted_entity(entity) {
                continue;
            }
            let Some(policy) = policies.get(&entity.storage_policy_entities) else {
                continue;
            };
            if policy.r#type == "local"
                && storage_policy_skip_reason(policy).is_none()
                && !source_settings(&policy.settings)
                    .get("encryption")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            {
                let root = local_policy_root(options, policy.id);
                cloudreve_adapter::normalize_local_storage_path(root, &entity.source)
                    .wrap_err_with(|| {
                        format!(
                            "validate Cloudreve local entity {} storage path against policy {} root",
                            entity.id, policy.id
                        )
                    })?;
                if verify_storage {
                    verify_local_entity(entity, policy, options)?;
                }
            }
        }
        cursor_value = last_entity.id;
    }
}

pub(super) fn verify_local_blob_batch(
    entities: &[cloudreve_schema::entities::Model],
    source: &SourceData,
    options: &MigrationOptions,
    context: &MigrationContext,
) -> Result<()> {
    let policies = source
        .policies
        .iter()
        .map(|policy| (policy.id, policy))
        .collect::<HashMap<_, _>>();
    for entity in entities {
        if is_encrypted_entity(entity) {
            continue;
        }
        if !context
            .policies
            .contains_key(&entity.storage_policy_entities)
        {
            continue;
        }
        let Some(policy) = policies.get(&entity.storage_policy_entities) else {
            continue;
        };
        if policy.r#type != "local" {
            continue;
        }
        verify_local_entity(entity, policy, options)?;
    }
    Ok(())
}

pub(super) async fn verify_all_remote_source_objects(
    source_db: &DatabaseConnection,
    source: &SourceData,
    _options: &MigrationOptions,
) -> Result<()> {
    const VERIFY_BATCH_SIZE: u64 = 500;

    let policies = source
        .policies
        .iter()
        .map(|policy| (policy.id, policy))
        .collect::<HashMap<_, _>>();
    let mut cursor_value = 0;
    loop {
        let mut query = cloudreve_schema::entities::Entity::find()
            .filter(cloudreve_schema::entities::Column::Type.eq(0))
            .filter(cloudreve_schema::entities::Column::Id.gt(cursor_value))
            .order_by_asc(cloudreve_schema::entities::Column::Id)
            .limit(VERIFY_BATCH_SIZE);
        if !source.include_deleted {
            query = query.filter(cloudreve_schema::entities::Column::DeletedAt.is_null());
        }
        let entities = query.all(source_db).await?;
        let Some(last_entity) = entities.last() else {
            return Ok(());
        };
        for entity in &entities {
            if is_encrypted_entity(entity) {
                continue;
            }
            let Some(policy) = policies.get(&entity.storage_policy_entities) else {
                continue;
            };
            if remote::supports_remote_validation(policy) {
                remote::verify_object(policy, &entity.source, entity.size, entity.id).await?;
            }
        }
        cursor_value = last_entity.id;
    }
}

pub(super) async fn verify_remote_blob_batch(
    entities: &[cloudreve_schema::entities::Model],
    source: &SourceData,
    _options: &MigrationOptions,
    context: &MigrationContext,
) -> Result<()> {
    let policies = source
        .policies
        .iter()
        .map(|policy| (policy.id, policy))
        .collect::<HashMap<_, _>>();
    for entity in entities {
        if is_encrypted_entity(entity) {
            continue;
        }
        if !context
            .policies
            .contains_key(&entity.storage_policy_entities)
        {
            continue;
        }
        let Some(policy) = policies.get(&entity.storage_policy_entities) else {
            continue;
        };
        if remote::supports_remote_validation(policy) {
            remote::verify_object(policy, &entity.source, entity.size, entity.id).await?;
        }
    }
    Ok(())
}

pub(super) fn verify_local_entity(
    entity: &cloudreve_schema::entities::Model,
    policy: &cloudreve_schema::storage_policies::Model,
    options: &MigrationOptions,
) -> Result<()> {
    let root = local_policy_root(options, policy.id);
    let path = local_storage_path(root, &entity.source);
    let metadata = std::fs::metadata(&path).wrap_err_with(|| {
        format!(
            "read Cloudreve local entity {} at {}",
            entity.id,
            path.display()
        )
    })?;
    if !metadata.is_file() {
        bail!(
            "Cloudreve local entity {} is not a regular file: {}",
            entity.id,
            path.display()
        );
    }
    std::fs::File::open(&path).wrap_err_with(|| {
        format!(
            "open Cloudreve local entity {} at {}",
            entity.id,
            path.display()
        )
    })?;
    let expected_size = u64::try_from(entity.size).map_err(|_| {
        color_eyre::eyre::eyre!(
            "Cloudreve local entity {} has negative size {}",
            entity.id,
            entity.size
        )
    })?;
    if metadata.len() != expected_size {
        bail!(
            "Cloudreve local entity {} size mismatch at {}: database={}, filesystem={}",
            entity.id,
            path.display(),
            expected_size,
            metadata.len()
        );
    }
    Ok(())
}
