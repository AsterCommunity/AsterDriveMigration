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
    options: &MigrationOptions,
) -> Result<Vec<String>> {
    const VERIFY_BATCH_SIZE: u64 = 500;

    let policies = source
        .policies
        .iter()
        .map(|policy| (policy.id, policy))
        .collect::<HashMap<_, _>>();
    let mut cursor_value = 0;
    let mut warnings = Vec::new();
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
            return Ok(warnings);
        };
        let entity_ids = entities.iter().map(|entity| entity.id).collect::<Vec<_>>();
        let association_info =
            super::engine::load_blob_association_info(source_db, &entity_ids).await?;
        for entity in &entities {
            if is_encrypted_entity(entity)
                || entity.upload_session_id.is_some()
                || !association_info.reference_counts.contains_key(&entity.id)
            {
                continue;
            }
            let Some(policy) = policies.get(&entity.storage_policy_entities) else {
                continue;
            };
            if remote::supports_remote_validation(policy)
                && let Some(warning) = verify_remote_entity(policy, entity, options, None).await?
            {
                eprintln!("[audit warning] {warning}");
                warnings.push(warning);
            }
        }
        cursor_value = last_entity.id;
    }
}

pub(super) async fn verify_remote_blob_batch(
    entities: &[cloudreve_schema::entities::Model],
    source: &SourceData,
    options: &MigrationOptions,
    context: &MigrationContext,
    run_id: &str,
) -> Result<Vec<String>> {
    let policies = source
        .policies
        .iter()
        .map(|policy| (policy.id, policy))
        .collect::<HashMap<_, _>>();
    let mut warnings = Vec::new();
    for entity in entities {
        if is_encrypted_entity(entity) || entity.upload_session_id.is_some() {
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
        if remote::supports_remote_validation(policy)
            && let Some(warning) =
                verify_remote_entity(policy, entity, options, Some(run_id)).await?
        {
            eprintln!("[audit warning] {warning}");
            warnings.push(warning);
        }
    }
    Ok(warnings)
}

async fn verify_remote_entity(
    policy: &cloudreve_schema::storage_policies::Model,
    entity: &cloudreve_schema::entities::Model,
    options: &MigrationOptions,
    run_id: Option<&str>,
) -> Result<Option<String>> {
    handle_remote_audit_result(
        options,
        run_id,
        entity.id,
        remote::verify_object(policy, &entity.source, entity.size, entity.id).await,
    )
}

fn handle_remote_audit_result(
    options: &MigrationOptions,
    run_id: Option<&str>,
    entity_id: i64,
    result: Result<()>,
) -> Result<Option<String>> {
    match result {
        Ok(()) => Ok(None),
        Err(error) if options.ignored_remote_audit_entities.contains(&entity_id) => Ok(Some(
            format!(
                "ignored remote audit error for Cloudreve entity {}: {error:#}",
                entity_id
            ),
        )),
        Err(error) => Err(error).wrap_err_with(|| {
            format!(
                "remote audit failed for Cloudreve entity {}; to ignore this result and continue, run:\n{}",
                entity_id,
                remote_audit_command(options, run_id, entity_id)
            )
        }),
    }
}

fn remote_audit_command(
    options: &MigrationOptions,
    run_id: Option<&str>,
    entity_id: i64,
) -> String {
    let subcommand = if run_id.is_some() {
        "resume"
    } else {
        "migrate"
    };
    let mut args = vec![
        "aster_drive_migration".to_string(),
        subcommand.to_string(),
        "--source-url \"$CLOUDREVE_DATABASE_URL\"".to_string(),
        "--target-url \"$ASTERDRIVE_DATABASE_URL\"".to_string(),
        "--default-password \"$ASTER_MIGRATION_DEFAULT_PASSWORD\"".to_string(),
        format!(
            "--local-base-path {}",
            shell_quote(&options.local_base_path)
        ),
        "--verify-remote-storage".to_string(),
        format!("--blob-batch-size {}", options.blob_batch_size),
        format!("--file-batch-size {}", options.file_batch_size),
    ];
    let mut ignored_entities = options.ignored_remote_audit_entities.clone();
    ignored_entities.insert(entity_id);
    for ignored_entity_id in ignored_entities {
        args.push(format!("--ignore-remote-audit-entity {ignored_entity_id}"));
    }
    if options.direct_link_secret.is_some() {
        args.push("--direct-link-secret \"$ASTER_DIRECT_LINK_SECRET\"".to_string());
    }
    if let Some(run_id) = run_id.or(options.run_id.as_deref()) {
        args.push(format!("--run-id {}", shell_quote(run_id)));
    }
    if options.verify_local_storage {
        args.push("--verify-local-storage".to_string());
    }
    if options.include_deleted {
        args.push("--include-deleted".to_string());
    }
    if options.allow_non_empty_target {
        args.push("--allow-non-empty-target".to_string());
    }
    if options.skip_unsupported_policies {
        args.push("--skip-unsupported-policies".to_string());
    }
    if options.dry_run {
        args.push("--dry-run".to_string());
    }
    for (policy_id, path) in &options.local_policy_roots {
        args.push(format!(
            "--local-policy-root {}",
            shell_quote(&format!("{policy_id}={path}"))
        ));
    }
    args.join(" ")
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> MigrationOptions {
        MigrationOptions {
            source_url: "SOURCE_URL_SENTINEL".to_string(),
            target_url: "sqlite://target.db".to_string(),
            default_password: "PASSWORD_SENTINEL".to_string(),
            local_base_path: "/srv/cloudreve's data".to_string(),
            local_policy_roots: BTreeMap::from([(7, "/srv/cos-cache".to_string())]),
            verify_local_storage: true,
            verify_remote_storage: true,
            ignored_remote_audit_entities: BTreeSet::new(),
            direct_link_secret: Some("DIRECT_LINK_SECRET_SENTINEL".to_string()),
            include_deleted: true,
            allow_non_empty_target: true,
            skip_unsupported_policies: true,
            dry_run: false,
            run_id: Some("audit-run".to_string()),
            resume: false,
            blob_batch_size: 250,
            file_batch_size: 750,
        }
    }

    #[test]
    fn remote_audit_failure_prints_secret_free_resume_command() {
        let mut options = options();
        options.ignored_remote_audit_entities.insert(948);
        let error = handle_remote_audit_result(
            &options,
            Some("audit-run"),
            949,
            Err(color_eyre::eyre::eyre!("NotFound")),
        )
        .expect_err("unignored remote audit error must fail");
        let message = format!("{error:#}");
        assert!(message.contains("aster_drive_migration resume"));
        assert!(message.contains("--ignore-remote-audit-entity 948"));
        assert!(message.contains("--ignore-remote-audit-entity 949"));
        assert!(message.contains("--run-id 'audit-run'"));
        assert!(message.contains("--skip-unsupported-policies"));
        assert!(message.contains("$ASTER_MIGRATION_DEFAULT_PASSWORD"));
        assert!(message.contains("$ASTER_DIRECT_LINK_SECRET"));
        for secret in [
            "SOURCE_URL_SENTINEL",
            "PASSWORD_SENTINEL",
            "DIRECT_LINK_SECRET_SENTINEL",
        ] {
            assert!(!message.contains(secret));
        }
    }

    #[test]
    fn ignored_remote_audit_failure_becomes_report_warning() -> Result<()> {
        let mut options = options();
        options.ignored_remote_audit_entities.insert(949);
        let warning = handle_remote_audit_result(
            &options,
            Some("audit-run"),
            949,
            Err(color_eyre::eyre::eyre!("NotFound")),
        )?
        .expect("ignored failure must produce a warning");
        assert!(warning.contains("Cloudreve entity 949"));
        assert!(warning.contains("NotFound"));
        assert!(handle_remote_audit_result(&options, None, 950, Ok(()))?.is_none());
        Ok(())
    }
}
