use super::*;

pub async fn list_migration_runs(target_url: &str) -> Result<Vec<MigrationRunSummary>> {
    let target = connect(target_url, "AsterDrive").await?;
    checkpoint::ensure_table(&target).await?;
    checkpoint::list(&target)
        .await?
        .into_iter()
        .map(run_summary)
        .collect()
}

pub async fn migration_run_report(target_url: &str, run_id: &str) -> Result<MigrationReport> {
    let target = connect(target_url, "AsterDrive").await?;
    checkpoint::ensure_table(&target).await?;
    let run = checkpoint::load_any(&target, run_id).await?;
    serde_json::from_value(run.report_json).wrap_err("decode stored migration report")
}

pub async fn migration_run_status(target_url: &str, run_id: &str) -> Result<MigrationRunSummary> {
    let target = connect(target_url, "AsterDrive").await?;
    checkpoint::ensure_table(&target).await?;
    run_summary(checkpoint::load_any(&target, run_id).await?)
}

pub async fn abort_migration_run(target_url: &str, run_id: &str) -> Result<()> {
    let target = connect(target_url, "AsterDrive").await?;
    checkpoint::ensure_table(&target).await?;
    checkpoint::abort(&target, run_id).await
}

pub async fn cleanup_completed_migration_run(target_url: &str, run_id: &str) -> Result<()> {
    let target = connect(target_url, "AsterDrive").await?;
    checkpoint::ensure_table(&target).await?;
    checkpoint::delete_completed(&target, run_id).await
}

pub(super) fn run_summary(run: checkpoint::Model) -> Result<MigrationRunSummary> {
    Ok(MigrationRunSummary {
        run_id: run.id,
        status: run.status,
        last_completed_stage: run.last_completed_stage,
        created_at: run.created_at,
        updated_at: run.updated_at,
        last_error: run.last_error,
    })
}

pub async fn inspect(
    source_url: &str,
    target_url: &str,
    include_deleted: bool,
) -> Result<MigrationReport> {
    let source = connect(source_url, "Cloudreve").await?;
    let target = connect(target_url, "AsterDrive").await?;
    let source_data = SourceData::load(&source, include_deleted).await?;
    validate_target_schema(&target).await?;
    let mut report = source_data.report();
    report.dry_run = true;
    report.warnings.extend(source_data.compatibility_warnings());
    report.preflight = run_preflight(&source, &source_data).await?;
    Ok(report)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MigrationStage {
    Policies,
    PolicyGroups,
    Users,
    Folders,
    Blobs,
    Files,
    Metadata,
    Shares,
    DirectLinks,
}

impl MigrationStage {
    const ALL: [Self; 9] = [
        Self::Policies,
        Self::PolicyGroups,
        Self::Users,
        Self::Folders,
        Self::Blobs,
        Self::Files,
        Self::Metadata,
        Self::Shares,
        Self::DirectLinks,
    ];

    const fn as_str(self) -> &'static str {
        match self {
            Self::Policies => "policies",
            Self::PolicyGroups => "policy_groups",
            Self::Users => "users",
            Self::Folders => "folders",
            Self::Blobs => "blobs",
            Self::Files => "files",
            Self::Metadata => "metadata",
            Self::Shares => "shares",
            Self::DirectLinks => "direct_links",
        }
    }

    fn plan() -> Result<aster_drive_migration_core::StagePlan> {
        Ok(aster_drive_migration_core::StagePlan::new(
            Self::ALL
                .into_iter()
                .map(|stage| aster_drive_migration_core::StageId::borrowed(stage.as_str())),
        )?)
    }
}

pub async fn migrate(options: MigrationOptions) -> Result<MigrationReport> {
    validate_migration_options(&options)?;

    let source = connect(&options.source_url, "Cloudreve").await?;
    let target = connect(&options.target_url, "AsterDrive").await?;
    let source_data = SourceData::load(&source, options.include_deleted).await?;
    validate_target_schema(&target).await?;
    let preflight = run_preflight(&source, &source_data).await?;
    if !preflight.passed {
        bail!(
            "Cloudreve preflight failed ({} checks); run `check --report-path` for details and repair source data before migration",
            preflight
                .checks
                .iter()
                .filter(|check| !check.passed)
                .count()
        );
    }
    migrate_validated(options, source, target, source_data, preflight).await
}

fn validate_migration_options(options: &MigrationOptions) -> Result<()> {
    if options.default_password.chars().count() < 8 {
        bail!("--default-password must contain at least 8 characters");
    }
    if options
        .direct_link_secret
        .as_deref()
        .is_some_and(|secret| secret.chars().count() < 16)
    {
        bail!("--direct-link-secret must contain at least 16 characters");
    }
    if options.resume && options.run_id.is_none() {
        bail!("--resume requires --run-id");
    }
    if options.resume && options.dry_run {
        bail!("--resume cannot be combined with --dry-run");
    }
    if let Some(run_id) = options.run_id.as_deref() {
        validate_run_id(run_id)?;
    }
    if !(1..=10_000).contains(&options.blob_batch_size) {
        bail!("--blob-batch-size must be between 1 and 10000");
    }
    if !(1..=10_000).contains(&options.file_batch_size) {
        bail!("--file-batch-size must be between 1 and 10000");
    }
    Ok(())
}

async fn migrate_validated(
    options: MigrationOptions,
    source: DatabaseConnection,
    target: DatabaseConnection,
    source_data: SourceData,
    preflight: MigrationPreflight,
) -> Result<MigrationReport> {
    let unsupported = source_data.unsupported_policies();
    if !unsupported.is_empty() && !options.skip_unsupported_policies {
        bail!(
            "unsupported Cloudreve storage policies: {}; rerun with --skip-unsupported-policies to omit their files",
            unsupported.join(", ")
        );
    }
    validate_local_policy_roots(&source_data, &options)?;
    if options.verify_local_storage {
        verify_local_storage_roots(&source_data, &options)?;
    }
    validate_all_local_source_objects(
        &source,
        &source_data,
        &options,
        options.dry_run && options.verify_local_storage,
    )
    .await?;
    if options.dry_run && options.verify_remote_storage {
        verify_all_remote_source_objects(&source, &source_data, &options).await?;
    }

    let mut report = source_data.report();
    report.dry_run = options.dry_run;
    report.warnings.extend(source_data.compatibility_warnings());
    report.preflight = preflight;
    if options.dry_run {
        report.run_id = options.run_id.clone();
        return Ok(report);
    }

    checkpoint::ensure_table(&target).await?;
    let run_id = options
        .run_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let source_fingerprint = source_fingerprint(&options.source_url, &source_data);
    let target_fingerprint = hash_fingerprint(&options.target_url);
    let plan_fingerprint = plan_fingerprint(&options);

    let (mut context, target_before, mut last_completed_stage) = if options.resume {
        let loaded = checkpoint::load(
            &target,
            &run_id,
            &source_fingerprint,
            &target_fingerprint,
            &plan_fingerprint,
        )
        .await?;
        let status = aster_drive_migration_core::RunStatus::parse(&loaded.status)?;
        if !status.can_resume() {
            bail!(
                "migration run {run_id} has status {}; this run is terminal and cannot be resumed",
                loaded.status
            );
        }
        report = loaded.report;
        report.completed_stages.retain(|stage| stage != "tasks");
        report.resumed = true;
        report.run_id = Some(run_id.clone());
        let last_completed_stage = normalize_legacy_completed_stage(loaded.last_completed_stage);
        (loaded.context, loaded.baseline, last_completed_stage)
    } else {
        ensure_target_safe(&target, options.allow_non_empty_target).await?;
        let baseline = TargetCounts::load(&target).await?;
        report.run_id = Some(run_id.clone());
        let context = MigrationContext::default();
        checkpoint::create(
            &target,
            checkpoint::NewCheckpoint {
                run_id: &run_id,
                source_fingerprint: &source_fingerprint,
                target_fingerprint: &target_fingerprint,
                plan_fingerprint: &plan_fingerprint,
                context: &context,
                report: &report,
                baseline: &baseline,
            },
        )
        .await?;
        (context, baseline, None)
    };
    let mut blob_mappings = checkpoint::load_object_mappings(&target, &run_id, "blob").await?;
    if blob_mappings.is_empty() && !context.blobs.is_empty() {
        let legacy_mappings = context
            .blobs
            .iter()
            .map(|(source_id, target_id)| (*source_id, *target_id))
            .collect::<Vec<_>>();
        checkpoint::save_object_mappings(&target, &run_id, "blob", &legacy_mappings).await?;
        blob_mappings.extend(legacy_mappings);
        context.blobs.clear();
    }
    if report.migrated_blobs != blob_mappings.len() {
        bail!(
            "migration run {run_id} blob mapping count {} does not match migrated blob count {}; restore the checkpoint tables before resuming",
            blob_mappings.len(),
            report.migrated_blobs
        );
    }
    let mut file_mappings = checkpoint::load_object_mappings(&target, &run_id, "file").await?;
    if file_mappings.is_empty() && !context.files.is_empty() {
        let legacy_mappings = context
            .files
            .iter()
            .map(|(source_id, target_id)| (*source_id, *target_id))
            .collect::<Vec<_>>();
        checkpoint::save_object_mappings(&target, &run_id, "file", &legacy_mappings).await?;
        file_mappings.extend(legacy_mappings);
        context.files.clear();
    }
    if report.migrated_files != file_mappings.len() {
        bail!(
            "migration run {run_id} file mapping count {} does not match migrated file count {}; restore the checkpoint tables before resuming",
            file_mappings.len(),
            report.migrated_files
        );
    }

    let password_hash = hash_argon2_password(&options.default_password)?;

    let stage_plan = MigrationStage::plan()?;
    for (stage_index, stage) in MigrationStage::ALL.into_iter().enumerate() {
        if !stage_plan.should_run_after(
            &aster_drive_migration_core::StageId::borrowed(stage.as_str()),
            last_completed_stage.as_deref(),
        )? {
            continue;
        }
        eprintln!(
            "[progress] stage {}/{} {} started",
            stage_index + 1,
            MigrationStage::ALL.len(),
            stage.as_str()
        );
        if stage == MigrationStage::Blobs {
            let inputs = BlobBatchInputs {
                source: &source,
                target: &target,
                run_id: &run_id,
                source_data: &source_data,
                options: &options,
                context: &context,
                file_mappings: &file_mappings,
            };
            if let Err(error) =
                migrate_blobs_batched(&inputs, &mut blob_mappings, &mut report).await
            {
                let _ = checkpoint::mark_failed(&target, &run_id, &error.to_string()).await;
                return Err(error).wrap_err_with(|| {
                    format!(
                        "migration run {run_id} failed at stage {}; rerun with --resume --run-id {run_id}",
                        stage.as_str()
                    )
                });
            }
            last_completed_stage = Some(stage.as_str().to_string());
            eprintln!("[progress] stage {} completed", stage.as_str());
            continue;
        }
        if stage == MigrationStage::Files {
            let inputs = FileBatchInputs {
                source: &source,
                target: &target,
                run_id: &run_id,
                source_data: &source_data,
                options: &options,
                context: &context,
                blob_mappings: &blob_mappings,
            };
            if let Err(error) =
                migrate_files_batched(&inputs, &mut file_mappings, &mut report).await
            {
                let _ = checkpoint::mark_failed(&target, &run_id, &error.to_string()).await;
                return Err(error).wrap_err_with(|| {
                    format!(
                        "migration run {run_id} failed at stage {}; rerun with --resume --run-id {run_id}",
                        stage.as_str()
                    )
                });
            }
            last_completed_stage = Some(stage.as_str().to_string());
            eprintln!("[progress] stage {} completed", stage.as_str());
            continue;
        }
        let transaction = target
            .begin()
            .await
            .wrap_err_with(|| format!("begin migration stage {}", stage.as_str()))?;
        let stage_result: Result<()> = async {
            let inputs = StageInputs {
                source_db: &source,
                source_data: &source_data,
                options: &options,
                password_hash: &password_hash,
                file_mappings: &file_mappings,
            };
            execute_stage(stage, &transaction, &inputs, &mut context, &mut report).await?;
            report.set_mappings(&context, &blob_mappings, &file_mappings);
            if !report
                .completed_stages
                .iter()
                .any(|completed| completed == stage.as_str())
            {
                report.completed_stages.push(stage.as_str().to_string());
            }
            checkpoint::save_stage(&transaction, &run_id, stage.as_str(), &context, &report).await
        }
        .await;
        if let Err(error) = stage_result {
            drop(transaction);
            let _ = checkpoint::mark_failed(&target, &run_id, &error.to_string()).await;
            return Err(error).wrap_err_with(|| {
                format!(
                    "migration run {run_id} failed at stage {}; rerun with --resume --run-id {run_id}",
                    stage.as_str()
                )
            });
        }
        if let Err(error) = transaction.commit().await {
            let _ = checkpoint::mark_failed(&target, &run_id, &error.to_string()).await;
            return Err(error).wrap_err_with(|| {
                format!(
                    "commit migration run {run_id} stage {}; resume with the same run ID",
                    stage.as_str()
                )
            });
        }
        last_completed_stage = Some(stage.as_str().to_string());
        eprintln!("[progress] stage {} completed", stage.as_str());
    }

    report.set_mappings(&context, &blob_mappings, &file_mappings);
    let recalculation = target
        .begin()
        .await
        .wrap_err("begin final statistics recalculation")?;
    if let Err(error) = recalculate_statistics(&recalculation).await {
        let _ = recalculation.rollback().await;
        let _ = checkpoint::mark_failed(&target, &run_id, &error.to_string()).await;
        return Err(error)
            .wrap_err_with(|| format!("recalculate completed migration run {run_id}"));
    }
    recalculation
        .commit()
        .await
        .wrap_err("commit final statistics recalculation")?;
    report.validation =
        match validate_migration_result(&target, &target_before, &report, &options).await {
            Ok(validation) => validation,
            Err(error) => {
                let _ = checkpoint::mark_failed(&target, &run_id, &error.to_string()).await;
                return Err(error)
                    .wrap_err_with(|| format!("validate completed migration run {run_id}"));
            }
        };
    report.generated_at = chrono::Utc::now();
    checkpoint::finish(
        &target,
        &run_id,
        if report.validation.passed {
            "completed"
        } else {
            "validation_failed"
        },
        &context,
        &report,
    )
    .await?;
    Ok(report)
}

pub(super) struct StageInputs<'a> {
    source_db: &'a DatabaseConnection,
    source_data: &'a SourceData,
    options: &'a MigrationOptions,
    password_hash: &'a str,
    file_mappings: &'a HashMap<i64, i64>,
}

pub(super) async fn execute_stage(
    stage: MigrationStage,
    transaction: &sea_orm::DatabaseTransaction,
    inputs: &StageInputs<'_>,
    context: &mut MigrationContext,
    report: &mut MigrationReport,
) -> Result<()> {
    match stage {
        MigrationStage::Policies => {
            migrate_policies(
                transaction,
                inputs.source_data,
                inputs.options,
                context,
                report,
            )
            .await
        }
        MigrationStage::PolicyGroups => {
            migrate_policy_groups(transaction, inputs.source_data, context, report).await
        }
        MigrationStage::Users => {
            migrate_users(
                transaction,
                inputs.source_data,
                inputs.password_hash,
                context,
                report,
            )
            .await
        }
        MigrationStage::Folders => {
            migrate_folders(transaction, inputs.source_data, context, report).await
        }
        MigrationStage::Blobs => bail!("blobs stage must use the batched runner"),
        MigrationStage::Files => bail!("files stage must use the batched runner"),
        MigrationStage::Metadata => {
            migrate_metadata(
                transaction,
                inputs.source_db,
                inputs.source_data,
                inputs.file_mappings,
                context,
                report,
            )
            .await
        }
        MigrationStage::Shares => {
            migrate_shares(
                transaction,
                inputs.source_db,
                inputs.source_data,
                inputs.file_mappings,
                context,
                report,
            )
            .await
        }
        MigrationStage::DirectLinks => {
            migrate_direct_links(
                transaction,
                inputs.source_db,
                inputs.source_data,
                inputs.file_mappings,
                context,
                inputs.options.direct_link_secret.as_deref(),
                report,
            )
            .await
        }
    }
}

pub(super) struct BlobBatchInputs<'a> {
    source: &'a DatabaseConnection,
    target: &'a DatabaseConnection,
    run_id: &'a str,
    source_data: &'a SourceData,
    options: &'a MigrationOptions,
    context: &'a MigrationContext,
    file_mappings: &'a HashMap<i64, i64>,
}

pub(super) async fn migrate_blobs_batched(
    inputs: &BlobBatchInputs<'_>,
    blob_mappings: &mut HashMap<i64, i64>,
    report: &mut MigrationReport,
) -> Result<()> {
    let cursor =
        checkpoint::load_stage_cursor(inputs.target, inputs.run_id, MigrationStage::Blobs.as_str())
            .await?;
    let mut cursor_value = cursor.as_ref().map_or(0, |cursor| cursor.cursor_value);
    let mut processed_count = cursor.as_ref().map_or(0, |cursor| cursor.processed_count);
    let started_at = Instant::now();

    loop {
        let mut query = cloudreve_schema::entities::Entity::find()
            .filter(cloudreve_schema::entities::Column::Type.eq(0))
            .filter(cloudreve_schema::entities::Column::Id.gt(cursor_value))
            .order_by_asc(cloudreve_schema::entities::Column::Id)
            .limit(inputs.options.blob_batch_size as u64);
        if !inputs.source_data.include_deleted {
            query = query.filter(cloudreve_schema::entities::Column::DeletedAt.is_null());
        }
        let entities = query.all(inputs.source).await?;
        if entities.is_empty() {
            let transaction = inputs
                .target
                .begin()
                .await
                .wrap_err("begin blobs completion")?;
            report.set_mappings(inputs.context, blob_mappings, inputs.file_mappings);
            if !report
                .completed_stages
                .iter()
                .any(|completed| completed == MigrationStage::Blobs.as_str())
            {
                report
                    .completed_stages
                    .push(MigrationStage::Blobs.as_str().to_string());
            }
            checkpoint::save_stage(
                &transaction,
                inputs.run_id,
                MigrationStage::Blobs.as_str(),
                inputs.context,
                report,
            )
            .await?;
            transaction
                .commit()
                .await
                .wrap_err("commit blobs completion")?;
            return Ok(());
        }

        let entity_ids = entities.iter().map(|entity| entity.id).collect::<Vec<_>>();
        let association_info = load_blob_association_info(inputs.source, &entity_ids).await?;
        let referenced_entities = entities
            .iter()
            .filter(|entity| {
                entity.upload_session_id.is_none()
                    && association_info.reference_counts.contains_key(&entity.id)
            })
            .cloned()
            .collect::<Vec<_>>();

        if inputs.options.verify_local_storage {
            verify_local_blob_batch(
                &referenced_entities,
                inputs.source_data,
                inputs.options,
                inputs.context,
            )?;
        }
        if inputs.options.verify_remote_storage {
            verify_remote_blob_batch(
                &referenced_entities,
                inputs.source_data,
                inputs.options,
                inputs.context,
            )
            .await?;
        }

        let Some(last_entity_id) = entities.last().map(|entity| entity.id) else {
            bail!("blob batch query returned no rows after the empty-batch check");
        };
        let transaction = inputs.target.begin().await.wrap_err("begin blobs batch")?;
        let report_before_batch = report.clone();
        let batch_result: Result<Vec<(i64, i64)>> = async {
            let mappings = migrate_blob_batch(
                &transaction,
                &entities,
                &association_info.reference_counts,
                inputs.source_data,
                inputs.options,
                inputs.context,
                report,
            )
            .await?;
            checkpoint::save_object_mappings(&transaction, inputs.run_id, "blob", &mappings)
                .await?;
            processed_count += entities.len() as i64;
            checkpoint::save_stage_cursor(
                &transaction,
                inputs.run_id,
                MigrationStage::Blobs.as_str(),
                last_entity_id,
                processed_count,
            )
            .await?;
            checkpoint::save_progress(&transaction, inputs.run_id, inputs.context, report).await?;
            Ok(mappings)
        }
        .await;
        let mappings = match batch_result {
            Ok(mappings) => mappings,
            Err(error) => {
                let _ = transaction.rollback().await;
                *report = report_before_batch;
                return Err(error);
            }
        };
        if let Err(error) = transaction.commit().await {
            return Err(error).wrap_err("commit blobs batch");
        }
        let batch_bytes = entities.iter().try_fold(0_i64, |total, entity| {
            total
                .checked_add(entity.size)
                .ok_or_else(|| color_eyre::eyre::eyre!("blob batch byte count overflow"))
        })?;
        eprintln!(
            "[progress] blobs: source_rows={processed_count}/{}, batch_rows={}, batch_bytes={batch_bytes}, {}",
            inputs.source_data.source_blobs,
            entities.len(),
            progress_timing(processed_count, inputs.source_data.source_blobs, started_at)
        );
        blob_mappings.extend(mappings);
        cursor_value = last_entity_id;
    }
}

pub(super) struct FileBatchInputs<'a> {
    source: &'a DatabaseConnection,
    target: &'a DatabaseConnection,
    run_id: &'a str,
    source_data: &'a SourceData,
    options: &'a MigrationOptions,
    context: &'a MigrationContext,
    blob_mappings: &'a HashMap<i64, i64>,
}

pub(super) async fn migrate_files_batched(
    inputs: &FileBatchInputs<'_>,
    file_mappings: &mut HashMap<i64, i64>,
    report: &mut MigrationReport,
) -> Result<()> {
    let cursor =
        checkpoint::load_stage_cursor(inputs.target, inputs.run_id, MigrationStage::Files.as_str())
            .await?;
    let mut cursor_value = cursor.as_ref().map_or(0, |cursor| cursor.cursor_value);
    let mut processed_count = cursor.as_ref().map_or(0, |cursor| cursor.processed_count);
    let started_at = Instant::now();

    loop {
        let files = cloudreve_schema::files::Entity::find()
            .filter(cloudreve_schema::files::Column::Type.eq(0))
            .filter(cloudreve_schema::files::Column::Id.gt(cursor_value))
            .order_by_asc(cloudreve_schema::files::Column::Id)
            .limit(inputs.options.file_batch_size as u64)
            .all(inputs.source)
            .await?;
        if files.is_empty() {
            let transaction = inputs
                .target
                .begin()
                .await
                .wrap_err("begin files completion")?;
            report.set_mappings(inputs.context, inputs.blob_mappings, file_mappings);
            if !report
                .completed_stages
                .iter()
                .any(|completed| completed == MigrationStage::Files.as_str())
            {
                report
                    .completed_stages
                    .push(MigrationStage::Files.as_str().to_string());
            }
            checkpoint::save_stage(
                &transaction,
                inputs.run_id,
                MigrationStage::Files.as_str(),
                inputs.context,
                report,
            )
            .await?;
            transaction
                .commit()
                .await
                .wrap_err("commit files completion")?;
            return Ok(());
        }

        let (associations, entities) =
            load_file_batch_data(inputs.source, inputs.source_data.include_deleted, &files).await?;
        let Some(last_file_id) = files.last().map(|file| file.id) else {
            bail!("file batch query returned no rows after the empty-batch check");
        };
        let transaction = inputs.target.begin().await.wrap_err("begin files batch")?;
        let mappings = migrate_file_batch(
            &transaction,
            &files,
            &associations,
            &entities,
            inputs.blob_mappings,
            inputs.context,
            report,
        )
        .await?;
        checkpoint::save_object_mappings(&transaction, inputs.run_id, "file", &mappings).await?;
        processed_count += files.len() as i64;
        checkpoint::save_stage_cursor(
            &transaction,
            inputs.run_id,
            MigrationStage::Files.as_str(),
            last_file_id,
            processed_count,
        )
        .await?;
        checkpoint::save_progress(&transaction, inputs.run_id, inputs.context, report).await?;
        transaction.commit().await.wrap_err("commit files batch")?;
        let batch_bytes = files.iter().try_fold(0_i64, |total, file| {
            total
                .checked_add(file.size)
                .ok_or_else(|| color_eyre::eyre::eyre!("file batch byte count overflow"))
        })?;
        eprintln!(
            "[progress] files: source_rows={processed_count}/{}, batch_rows={}, batch_bytes={batch_bytes}, {}",
            inputs.source_data.source_files,
            files.len(),
            progress_timing(processed_count, inputs.source_data.source_files, started_at)
        );
        file_mappings.extend(mappings);
        cursor_value = last_file_id;
    }
}

pub(super) async fn load_file_batch_data(
    source: &DatabaseConnection,
    include_deleted: bool,
    files: &[cloudreve_schema::files::Model],
) -> Result<(
    HashMap<i64, Vec<i64>>,
    HashMap<i64, cloudreve_schema::entities::Model>,
)> {
    const QUERY_ID_BATCH_SIZE: usize = 500;

    let file_ids = files.iter().map(|file| file.id).collect::<Vec<_>>();
    let mut file_entities = Vec::new();
    for file_ids in file_ids.chunks(QUERY_ID_BATCH_SIZE) {
        file_entities.extend(
            cloudreve_schema::file_entities::Entity::find()
                .filter(
                    cloudreve_schema::file_entities::Column::FileId.is_in(file_ids.iter().copied()),
                )
                .all(source)
                .await?,
        );
    }
    let mut entity_ids = file_entities
        .iter()
        .map(|relation| relation.entity_id)
        .collect::<HashSet<_>>();
    entity_ids.extend(files.iter().filter_map(|file| file.primary_entity));
    let entity_ids = entity_ids.into_iter().collect::<Vec<_>>();
    let mut entities = HashMap::with_capacity(entity_ids.len());
    for entity_ids in entity_ids.chunks(QUERY_ID_BATCH_SIZE) {
        let mut query = cloudreve_schema::entities::Entity::find()
            .filter(cloudreve_schema::entities::Column::Id.is_in(entity_ids.iter().copied()))
            .filter(cloudreve_schema::entities::Column::Type.eq(0));
        if !include_deleted {
            query = query.filter(cloudreve_schema::entities::Column::DeletedAt.is_null());
        }
        for entity in query.all(source).await? {
            entities.insert(entity.id, entity);
        }
    }
    Ok((associations(files, &file_entities), entities))
}

pub(super) struct BlobAssociationInfo {
    reference_counts: HashMap<i64, i64>,
}

pub(super) async fn load_blob_association_info(
    source: &DatabaseConnection,
    blob_ids: &[i64],
) -> Result<BlobAssociationInfo> {
    if blob_ids.is_empty() {
        return Ok(BlobAssociationInfo {
            reference_counts: HashMap::new(),
        });
    }
    let relations = cloudreve_schema::file_entities::Entity::find()
        .filter(cloudreve_schema::file_entities::Column::EntityId.is_in(blob_ids.iter().copied()))
        .all(source)
        .await?;
    let primary_files = cloudreve_schema::files::Entity::find()
        .filter(cloudreve_schema::files::Column::PrimaryEntity.is_in(blob_ids.iter().copied()))
        .all(source)
        .await?;
    let file_ids = relations
        .iter()
        .map(|relation| relation.file_id)
        .chain(primary_files.iter().map(|file| file.id))
        .collect::<HashSet<_>>();
    if file_ids.is_empty() {
        return Ok(BlobAssociationInfo {
            reference_counts: HashMap::new(),
        });
    }
    let mut migratable_file_ids = HashSet::new();
    for file_ids in file_ids.iter().copied().collect::<Vec<_>>().chunks(500) {
        migratable_file_ids.extend(
            cloudreve_schema::files::Entity::find()
                .filter(cloudreve_schema::files::Column::Id.is_in(file_ids.iter().copied()))
                .filter(cloudreve_schema::files::Column::Type.eq(0))
                .filter(cloudreve_schema::files::Column::IsSymbolic.eq(false))
                .filter(cloudreve_schema::files::Column::PrimaryEntity.is_not_null())
                .all(source)
                .await?
                .into_iter()
                .map(|file| file.id),
        );
    }
    let blob_id_set = blob_ids.iter().copied().collect::<HashSet<_>>();
    let mut file_blob_pairs = HashSet::new();
    for (blob_id, file_id) in relations
        .into_iter()
        .map(|relation| (relation.entity_id, relation.file_id))
        .chain(
            primary_files
                .into_iter()
                .filter_map(|file| file.primary_entity.map(|entity_id| (entity_id, file.id))),
        )
    {
        if blob_id_set.contains(&blob_id) && migratable_file_ids.contains(&file_id) {
            file_blob_pairs.insert((file_id, blob_id));
        }
    }
    let mut reference_counts = HashMap::new();
    for (_, blob_id) in file_blob_pairs {
        *reference_counts.entry(blob_id).or_insert(0) += 1;
    }
    Ok(BlobAssociationInfo { reference_counts })
}

pub(super) fn validate_run_id(run_id: &str) -> Result<()> {
    let run_id = run_id.trim();
    if run_id.is_empty() || run_id.chars().count() > 128 || run_id.chars().any(char::is_control) {
        bail!("--run-id must contain 1-128 non-control characters");
    }
    Ok(())
}

fn normalize_legacy_completed_stage(stage: Option<String>) -> Option<String> {
    match stage.as_deref() {
        // Runs created before task migration was removed used `tasks` as the terminal stage.
        Some("tasks") => Some(MigrationStage::DirectLinks.as_str().to_string()),
        _ => stage,
    }
}

pub(super) fn hash_fingerprint(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub(super) fn source_fingerprint(source_url: &str, source: &SourceData) -> String {
    hash_fingerprint(&format!(
        "{source_url}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        source.users.len(),
        source.groups.len(),
        source.policies.len(),
        source.source_file_records,
        source.source_entities,
        source.source_file_entities,
        source.shares.len(),
        source.metadata.len(),
        source.direct_links.len(),
        source.source_tasks,
    ))
}

pub(super) fn progress_timing(processed: i64, total: u64, started_at: Instant) -> String {
    let elapsed_seconds = started_at.elapsed().as_secs_f64().max(0.001);
    let rows_per_second = processed as f64 / elapsed_seconds;
    let remaining_rows = total.saturating_sub(processed.max(0).cast_unsigned());
    let eta_seconds = if rows_per_second > 0.0 {
        remaining_rows as f64 / rows_per_second
    } else {
        0.0
    };
    format!(
        "elapsed_secs={elapsed_seconds:.1}, rows_per_sec={rows_per_second:.2}, eta_secs={eta_seconds:.1}"
    )
}

pub(super) fn plan_fingerprint(options: &MigrationOptions) -> String {
    hash_fingerprint(&format!(
        "{}|{}|{}|{}|{}|{}|{}|{}",
        options.local_base_path,
        options
            .local_policy_roots
            .iter()
            .map(|(policy_id, path)| format!("{policy_id}={path}"))
            .collect::<Vec<_>>()
            .join("|"),
        options.verify_local_storage,
        options.include_deleted,
        options.allow_non_empty_target,
        options.skip_unsupported_policies,
        hash_fingerprint(&options.default_password),
        options
            .direct_link_secret
            .as_deref()
            .map(hash_fingerprint)
            .unwrap_or_default(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectionTrait, DbBackend, Schema};
    use std::collections::BTreeMap;
    use std::time::Instant;

    fn migration_options() -> MigrationOptions {
        MigrationOptions {
            source_url: "sqlite::memory:".to_string(),
            target_url: "sqlite::memory:".to_string(),
            default_password: "12345678".to_string(),
            local_base_path: ".".to_string(),
            local_policy_roots: BTreeMap::new(),
            verify_local_storage: false,
            verify_remote_storage: false,
            direct_link_secret: Some("1234567890123456".to_string()),
            include_deleted: false,
            allow_non_empty_target: false,
            skip_unsupported_policies: false,
            dry_run: false,
            run_id: Some("run".to_string()),
            resume: false,
            blob_batch_size: 1,
            file_batch_size: 10_000,
        }
    }

    #[test]
    fn migration_option_validation_accepts_exact_boundaries() -> Result<()> {
        let options = migration_options();
        validate_migration_options(&options)?;
        let mut no_optional_secret = options;
        no_optional_secret.direct_link_secret = None;
        validate_migration_options(&no_optional_secret)
    }

    #[test]
    fn migration_option_validation_rejects_password_and_secret_below_minimum() {
        let mut options = migration_options();
        options.default_password = "1234567".to_string();
        assert!(
            validate_migration_options(&options)
                .expect_err("short password must fail")
                .to_string()
                .contains("at least 8")
        );

        let mut options = migration_options();
        options.direct_link_secret = Some("123456789012345".to_string());
        assert!(
            validate_migration_options(&options)
                .expect_err("short direct-link secret must fail")
                .to_string()
                .contains("at least 16")
        );
    }

    #[test]
    fn migration_option_validation_rejects_invalid_resume_and_run_ids() {
        let mut options = migration_options();
        options.resume = true;
        options.run_id = None;
        assert!(validate_migration_options(&options).is_err());

        let mut options = migration_options();
        options.resume = true;
        options.dry_run = true;
        assert!(validate_migration_options(&options).is_err());

        for run_id in [String::new(), "x".repeat(129), "run\n1".to_string()] {
            let mut options = migration_options();
            options.run_id = Some(run_id);
            assert!(validate_migration_options(&options).is_err());
        }
    }

    #[test]
    fn migration_option_validation_rejects_batch_sizes_outside_closed_range() {
        for (blob_batch_size, file_batch_size) in [(0, 1), (10_001, 1), (1, 0), (1, 10_001)] {
            let mut options = migration_options();
            options.blob_batch_size = blob_batch_size;
            options.file_batch_size = file_batch_size;
            assert!(validate_migration_options(&options).is_err());
        }
    }

    #[test]
    fn progress_timing_includes_rate_and_eta() {
        let timing = progress_timing(50, 100, Instant::now());
        assert!(timing.contains("rows_per_sec="));
        assert!(timing.contains("eta_secs="));
    }

    #[test]
    fn normalizes_removed_task_stage_for_legacy_checkpoints() {
        assert_eq!(
            normalize_legacy_completed_stage(Some("tasks".to_string())).as_deref(),
            Some("direct_links")
        );
        assert_eq!(
            normalize_legacy_completed_stage(Some("shares".to_string())).as_deref(),
            Some("shares")
        );
        assert_eq!(normalize_legacy_completed_stage(None), None);
    }

    #[tokio::test]
    async fn blob_reference_counts_match_distinct_file_blob_relations() -> Result<()> {
        let database = Database::connect("sqlite::memory:").await?;
        database
            .execute_unprepared("PRAGMA foreign_keys = OFF")
            .await?;
        let schema = Schema::new(DbBackend::Sqlite);
        for statement in [
            schema.create_table_from_entity(cloudreve_schema::files::Entity),
            schema.create_table_from_entity(cloudreve_schema::entities::Entity),
            schema.create_table_from_entity(cloudreve_schema::file_entities::Entity),
        ] {
            database.execute(&statement).await?;
        }

        let now = chrono::Utc::now().fixed_offset();
        for id in [10, 11, 12] {
            cloudreve_schema::entities::ActiveModel {
                id: Set(id),
                created_at: Set(now),
                updated_at: Set(now),
                deleted_at: Set(None),
                r#type: Set(0),
                source: Set(format!("objects/{id}")),
                size: Set(128),
                reference_count: Set(99),
                upload_session_id: Set(None),
                recycle_options: Set(None),
                storage_policy_entities: Set(1),
                created_by: Set(Some(1)),
            }
            .insert(&database)
            .await?;
        }
        for (id, primary_entity, is_symbolic) in [
            (20, Some(10), false),
            (21, Some(10), false),
            (22, Some(10), true),
            (23, None, false),
        ] {
            cloudreve_schema::files::ActiveModel {
                id: Set(id),
                created_at: Set(now),
                updated_at: Set(now),
                r#type: Set(0),
                name: Set(format!("file-{id}.bin")),
                size: Set(128),
                primary_entity: Set(primary_entity),
                is_symbolic: Set(is_symbolic),
                props: Set(None),
                file_children: Set(None),
                storage_policy_files: Set(Some(1)),
                owner_id: Set(1),
            }
            .insert(&database)
            .await?;
        }
        for (file_id, entity_id) in [(20, 10), (20, 11), (23, 12)] {
            cloudreve_schema::file_entities::ActiveModel {
                file_id: Set(file_id),
                entity_id: Set(entity_id),
            }
            .insert(&database)
            .await?;
        }

        let counts = load_blob_association_info(&database, &[10, 11, 12])
            .await?
            .reference_counts;
        assert_eq!(counts.get(&10), Some(&2));
        assert_eq!(counts.get(&11), Some(&1));
        assert_eq!(counts.get(&12), None);
        Ok(())
    }
}
