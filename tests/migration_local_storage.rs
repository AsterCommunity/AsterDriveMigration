mod support;

use aster_drive_migration::migration::*;
use aster_drive_model as aster_drive_schema;
use color_eyre::eyre::Result;
use sea_orm::{ActiveModelTrait, Database, EntityTrait, IntoActiveModel, PaginatorTrait, Set};
use support::{create_source_schema, create_target_schema, seed_source, sqlite_url};

#[tokio::test]
async fn reuses_and_verifies_local_storage_per_policy_root() -> Result<()> {
    let suffix = uuid::Uuid::new_v4();
    let source_path = std::env::temp_dir().join(format!("cloudreve-local-reuse-{suffix}.db"));
    let target_path = std::env::temp_dir().join(format!("asterdrive-local-reuse-{suffix}.db"));
    let storage_root = std::env::temp_dir().join(format!("cloudreve-local-storage-{suffix}"));
    let source_url = sqlite_url(&source_path);
    let target_url = sqlite_url(&target_path);

    let source = Database::connect(&source_url).await?;
    create_source_schema(&source).await?;
    seed_source(&source).await?;
    let source_policy_id = cloudreve_schema::storage_policies::Entity::find()
        .one(&source)
        .await?
        .expect("seeded storage policy")
        .id;
    std::fs::create_dir_all(storage_root.join("uploads"))?;
    let absolute_object_path = storage_root.join("uploads/object.bin");
    std::fs::write(&absolute_object_path, vec![0_u8; 128])?;
    let entity = cloudreve_schema::entities::Entity::find()
        .one(&source)
        .await?
        .expect("seeded entity");
    let mut entity = entity.into_active_model();
    entity.source = Set(absolute_object_path.to_string_lossy().to_string());
    entity.update(&source).await?;
    source.close().await?;

    let target = Database::connect(&target_url).await?;
    create_target_schema(&target).await?;
    target.close().await?;

    let report = migrate(MigrationOptions {
        source_url: source_url.clone(),
        target_url: target_url.clone(),
        default_password: "temporary-password".to_string(),
        local_base_path: "unused-local-root".to_string(),
        local_policy_roots: std::collections::BTreeMap::from([(
            source_policy_id,
            storage_root.to_string_lossy().to_string(),
        )]),
        verify_local_storage: true,
        verify_remote_storage: false,
        direct_link_secret: Some("test-direct-link-secret".to_string()),
        include_deleted: false,
        allow_non_empty_target: false,
        skip_unsupported_policies: false,
        dry_run: false,
        run_id: None,
        resume: false,
        blob_batch_size: 500,
        file_batch_size: 500,
    })
    .await?;
    assert!(report.validation.passed);

    let target = Database::connect(&target_url).await?;
    let policy = aster_drive_schema::entities::storage_policy::Entity::find()
        .one(&target)
        .await?
        .expect("migrated storage policy");
    assert_eq!(policy.base_path, storage_root.to_string_lossy());
    let blob = aster_drive_schema::entities::file_blob::Entity::find()
        .one(&target)
        .await?
        .expect("migrated blob");
    assert_eq!(blob.storage_path, "uploads/object.bin");
    target.close().await?;

    let _ = std::fs::remove_file(source_path);
    let _ = std::fs::remove_file(target_path);
    let _ = std::fs::remove_dir_all(storage_root);
    Ok(())
}

#[tokio::test]
async fn rejects_local_storage_size_mismatch() -> Result<()> {
    let suffix = uuid::Uuid::new_v4();
    let source_path = std::env::temp_dir().join(format!("cloudreve-local-mismatch-{suffix}.db"));
    let target_path = std::env::temp_dir().join(format!("asterdrive-local-mismatch-{suffix}.db"));
    let storage_root =
        std::env::temp_dir().join(format!("cloudreve-local-mismatch-storage-{suffix}"));
    let source_url = sqlite_url(&source_path);
    let target_url = sqlite_url(&target_path);

    let source = Database::connect(&source_url).await?;
    create_source_schema(&source).await?;
    seed_source(&source).await?;
    let source_policy_id = cloudreve_schema::storage_policies::Entity::find()
        .one(&source)
        .await?
        .expect("seeded storage policy")
        .id;
    source.close().await?;
    std::fs::create_dir_all(storage_root.join("uploads"))?;
    std::fs::write(storage_root.join("uploads/object.bin"), vec![0_u8; 127])?;

    let target = Database::connect(&target_url).await?;
    create_target_schema(&target).await?;
    target.close().await?;

    let error = migrate(MigrationOptions {
        source_url: source_url.clone(),
        target_url: target_url.clone(),
        default_password: "temporary-password".to_string(),
        local_base_path: "unused-local-root".to_string(),
        local_policy_roots: std::collections::BTreeMap::from([(
            source_policy_id,
            storage_root.to_string_lossy().to_string(),
        )]),
        verify_local_storage: true,
        verify_remote_storage: false,
        direct_link_secret: Some("test-direct-link-secret".to_string()),
        include_deleted: false,
        allow_non_empty_target: false,
        skip_unsupported_policies: false,
        dry_run: true,
        run_id: None,
        resume: false,
        blob_batch_size: 500,
        file_batch_size: 500,
    })
    .await
    .unwrap_err();
    assert!(format!("{error:?}").contains("size mismatch"));

    let target = Database::connect(&target_url).await?;
    assert_eq!(
        aster_drive_schema::entities::storage_policy::Entity::find()
            .count(&target)
            .await?,
        0
    );
    target.close().await?;

    let _ = std::fs::remove_file(source_path);
    let _ = std::fs::remove_file(target_path);
    let _ = std::fs::remove_dir_all(storage_root);
    Ok(())
}

#[tokio::test]
async fn rejects_local_absolute_paths_outside_the_policy_root_before_target_writes() -> Result<()> {
    let suffix = uuid::Uuid::new_v4();
    let source_path = std::env::temp_dir().join(format!("cloudreve-local-outside-{suffix}.db"));
    let target_path = std::env::temp_dir().join(format!("asterdrive-local-outside-{suffix}.db"));
    let storage_root = std::env::temp_dir().join(format!("cloudreve-local-root-{suffix}"));
    let outside_path = std::env::temp_dir().join(format!("cloudreve-local-outside-{suffix}.bin"));
    let source_url = sqlite_url(&source_path);
    let target_url = sqlite_url(&target_path);

    let source = Database::connect(&source_url).await?;
    create_source_schema(&source).await?;
    seed_source(&source).await?;
    let source_policy_id = cloudreve_schema::storage_policies::Entity::find()
        .one(&source)
        .await?
        .expect("seeded storage policy")
        .id;
    let entity = cloudreve_schema::entities::Entity::find()
        .one(&source)
        .await?
        .expect("seeded entity");
    let mut entity = entity.into_active_model();
    entity.source = Set(outside_path.to_string_lossy().to_string());
    entity.update(&source).await?;
    source.close().await?;

    let target = Database::connect(&target_url).await?;
    create_target_schema(&target).await?;
    target.close().await?;

    let error = migrate(MigrationOptions {
        source_url: source_url.clone(),
        target_url: target_url.clone(),
        default_password: "temporary-password".to_string(),
        local_base_path: "unused-local-root".to_string(),
        local_policy_roots: std::collections::BTreeMap::from([(
            source_policy_id,
            storage_root.to_string_lossy().to_string(),
        )]),
        verify_local_storage: false,
        verify_remote_storage: false,
        direct_link_secret: Some("test-direct-link-secret".to_string()),
        include_deleted: false,
        allow_non_empty_target: false,
        skip_unsupported_policies: false,
        dry_run: false,
        run_id: None,
        resume: false,
        blob_batch_size: 500,
        file_batch_size: 500,
    })
    .await
    .unwrap_err();
    assert!(format!("{error:?}").contains("outside the configured local root"));

    let target = Database::connect(&target_url).await?;
    assert_eq!(
        aster_drive_schema::entities::storage_policy::Entity::find()
            .count(&target)
            .await?,
        0
    );
    assert_eq!(
        aster_drive_schema::entities::user::Entity::find()
            .count(&target)
            .await?,
        0
    );
    assert_eq!(
        aster_drive_schema::entities::folder::Entity::find()
            .count(&target)
            .await?,
        0
    );
    target.close().await?;

    let _ = std::fs::remove_file(source_path);
    let _ = std::fs::remove_file(target_path);
    Ok(())
}

#[tokio::test]
async fn rejects_unknown_local_policy_root() -> Result<()> {
    let suffix = uuid::Uuid::new_v4();
    let source_path = std::env::temp_dir().join(format!("cloudreve-local-policy-{suffix}.db"));
    let target_path = std::env::temp_dir().join(format!("asterdrive-local-policy-{suffix}.db"));
    let source_url = sqlite_url(&source_path);
    let target_url = sqlite_url(&target_path);

    let source = Database::connect(&source_url).await?;
    create_source_schema(&source).await?;
    seed_source(&source).await?;
    source.close().await?;

    let target = Database::connect(&target_url).await?;
    create_target_schema(&target).await?;
    target.close().await?;

    let error = migrate(MigrationOptions {
        source_url: source_url.clone(),
        target_url: target_url.clone(),
        default_password: "temporary-password".to_string(),
        local_base_path: "unused-local-root".to_string(),
        local_policy_roots: std::collections::BTreeMap::from([(999, "C:/missing".to_string())]),
        verify_local_storage: false,
        verify_remote_storage: false,
        direct_link_secret: Some("test-direct-link-secret".to_string()),
        include_deleted: false,
        allow_non_empty_target: false,
        skip_unsupported_policies: false,
        dry_run: true,
        run_id: None,
        resume: false,
        blob_batch_size: 500,
        file_batch_size: 500,
    })
    .await
    .unwrap_err();
    assert!(error.to_string().contains("policy 999"));

    let _ = std::fs::remove_file(source_path);
    let _ = std::fs::remove_file(target_path);
    Ok(())
}
