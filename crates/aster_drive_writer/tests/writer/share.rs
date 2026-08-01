use super::*;

#[tokio::test]
async fn writes_share_with_exact_target_and_counters() -> Result<()> {
    let database = database().await?;
    let transaction = database.begin().await?;
    let (_, user_id, folder_id) = write_prerequisites(&transaction).await?;
    let target_id = AsterDriveWriter::new(&transaction)
        .write_share(ResolvedShare {
            owner_id: user_id,
            target: ResolvedShareTarget::Folder {
                target_id: folder_id,
            },
            ..share(30)
        })
        .await?;
    let duplicate_target_id = AsterDriveWriter::new(&transaction)
        .write_share(ResolvedShare {
            owner_id: user_id,
            target: ResolvedShareTarget::Folder {
                target_id: folder_id,
            },
            ..share(31)
        })
        .await?;
    transaction.commit().await?;

    let stored = aster_drive_schema::entities::share::Entity::find_by_id(target_id)
        .one(&database)
        .await?
        .expect("written share");
    assert_eq!(stored.token.len(), 32);
    assert!(
        stored
            .token
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    );
    assert_eq!(stored.user_id, user_id);
    assert_eq!(stored.team_id, None);
    assert_eq!(stored.file_id, None);
    assert_eq!(stored.folder_id, Some(folder_id));
    assert_eq!(stored.password.as_deref(), Some("$argon2id$test-hash"));
    assert_eq!(stored.max_downloads, 10);
    assert_eq!(stored.download_count, 7);
    assert_eq!(stored.view_count, 5);
    let duplicate = aster_drive_schema::entities::share::Entity::find_by_id(duplicate_target_id)
        .one(&database)
        .await?
        .expect("duplicate target share");
    assert_ne!(stored.token, duplicate.token);
    assert_eq!(duplicate.folder_id, Some(folder_id));
    assert_eq!(
        aster_drive_schema::entities::share::Entity::find()
            .count(&database)
            .await?,
        2
    );
    Ok(())
}

#[tokio::test]
async fn invalid_share_owner_rolls_back_without_partial_rows() -> Result<()> {
    let database = database().await?;
    let setup = database.begin().await?;
    let (_, _, folder_id) = write_prerequisites(&setup).await?;
    setup.commit().await?;

    let transaction = database.begin().await?;
    let result = AsterDriveWriter::new(&transaction)
        .write_share(ResolvedShare {
            owner_id: i64::MAX,
            target: ResolvedShareTarget::Folder {
                target_id: folder_id,
            },
            password_hash: None,
            ..share(31)
        })
        .await;
    assert!(result.is_err());
    transaction.rollback().await?;
    assert_eq!(
        aster_drive_schema::entities::share::Entity::find()
            .count(&database)
            .await?,
        0
    );
    Ok(())
}
