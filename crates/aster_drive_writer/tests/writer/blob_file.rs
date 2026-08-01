use super::*;

#[tokio::test]
async fn writes_blob_file_and_versions_with_asterdrive_semantics() -> Result<()> {
    let database = database().await?;
    let transaction = database.begin().await?;
    let (policy_id, user_id, folder_id) = write_prerequisites(&transaction).await?;
    let writer = AsterDriveWriter::new(&transaction);
    let historical_blob_id = writer
        .write_blob(ResolvedBlob {
            blob: blob(10, "objects/history", 256, 1),
            policy_id,
        })
        .await?;
    let current_blob_id = writer
        .write_blob(ResolvedBlob {
            blob: blob(11, "objects/current", 512, 1),
            policy_id,
        })
        .await?;
    let historical_created_at = chrono::Utc::now() - chrono::Duration::days(1);
    let written = writer
        .write_file(ResolvedFile {
            file: file(20),
            folder_id: Some(folder_id),
            owner_id: user_id,
            owner_username: "owner".to_string(),
            primary_blob_id: current_blob_id,
            primary_blob_size: 512,
            historical_versions: vec![ResolvedFileVersion {
                blob_id: historical_blob_id,
                size: 256,
                created_at: historical_created_at,
            }],
        })
        .await?;
    transaction.commit().await?;

    assert_eq!(written.version_count, 1);
    let stored_file = aster_drive_schema::entities::file::Entity::find_by_id(written.target_id)
        .one(&database)
        .await?
        .expect("written file");
    assert_eq!(stored_file.blob_id, current_blob_id);
    assert_eq!(stored_file.size, 512);
    assert_eq!(stored_file.folder_id, Some(folder_id));
    assert_eq!(stored_file.mime_type, "application/gzip");
    assert_eq!(stored_file.extension, "gz");
    assert_eq!(stored_file.compound_extension.as_deref(), Some("tar.gz"));
    assert_eq!(stored_file.file_category.as_str(), "archive");

    let versions = aster_drive_schema::entities::file_version::Entity::find()
        .all(&database)
        .await?;
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].file_id, written.target_id);
    assert_eq!(versions[0].blob_id, historical_blob_id);
    assert_eq!(versions[0].version, 1);
    assert_eq!(versions[0].size, 256);
    assert_eq!(versions[0].created_at, historical_created_at);

    let current_blob = aster_drive_schema::entities::file_blob::Entity::find_by_id(current_blob_id)
        .one(&database)
        .await?
        .expect("current blob");
    assert_eq!(current_blob.hash, "cloudreve-000000000000000b");
    assert_eq!(current_blob.storage_path, "objects/current");
    assert_eq!(current_blob.size, 512);
    assert_eq!(current_blob.ref_count, 1);
    assert_eq!(current_blob.thumbnail_path, None);
    assert_eq!(current_blob.thumbnail_processor, None);
    assert_eq!(current_blob.thumbnail_version, None);
    Ok(())
}

#[tokio::test]
async fn file_and_versions_remain_atomic_when_a_version_blob_is_missing() -> Result<()> {
    let database = database().await?;
    let setup = database.begin().await?;
    let (policy_id, user_id, folder_id) = write_prerequisites(&setup).await?;
    let current_blob_id = AsterDriveWriter::new(&setup)
        .write_blob(ResolvedBlob {
            blob: blob(11, "objects/current", 512, 1),
            policy_id,
        })
        .await?;
    setup.commit().await?;

    let transaction = database.begin().await?;
    let result = AsterDriveWriter::new(&transaction)
        .write_file(ResolvedFile {
            file: file(21),
            folder_id: Some(folder_id),
            owner_id: user_id,
            owner_username: "owner".to_string(),
            primary_blob_id: current_blob_id,
            primary_blob_size: 512,
            historical_versions: vec![ResolvedFileVersion {
                blob_id: i64::MAX,
                size: 256,
                created_at: chrono::Utc::now(),
            }],
        })
        .await;
    assert!(result.is_err());
    transaction.rollback().await?;

    assert_eq!(
        aster_drive_schema::entities::file::Entity::find()
            .count(&database)
            .await?,
        0
    );
    assert_eq!(
        aster_drive_schema::entities::file_version::Entity::find()
            .count(&database)
            .await?,
        0
    );
    Ok(())
}
