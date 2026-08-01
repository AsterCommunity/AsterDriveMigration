use super::*;

#[tokio::test]
async fn writes_v2_direct_link_and_archives_cloudreve_counters() -> Result<()> {
    let database = database().await?;
    let transaction = database.begin().await?;
    let (policy_id, user_id, folder_id) = write_prerequisites(&transaction).await?;
    let file_id = write_file_target(&transaction, policy_id, user_id, folder_id).await?;
    let writer = AsterDriveWriter::new(&transaction);
    let written = writer
        .write_direct_link(
            ResolvedDirectLink {
                direct_link: direct_link(100),
                target_file_id: file_id,
                target_owner_id: user_id,
            },
            "test-direct-link-secret",
        )
        .await?;
    let other_owner = writer
        .write_direct_link(
            ResolvedDirectLink {
                direct_link: direct_link(101),
                target_file_id: file_id,
                target_owner_id: user_id + 1,
            },
            "test-direct-link-secret",
        )
        .await?;
    let other_secret = writer
        .write_direct_link(
            ResolvedDirectLink {
                direct_link: direct_link(102),
                target_file_id: file_id,
                target_owner_id: user_id,
            },
            "different-secret",
        )
        .await?;
    transaction.commit().await?;

    assert!(written.url.starts_with("/d/v2."));
    assert!(written.url.ends_with("/report%202026.txt"));
    let property =
        aster_drive_schema::entities::entity_property::Entity::find_by_id(written.property_id)
            .one(&database)
            .await?
            .expect("archived direct link property");
    assert_eq!(property.entity_type, EntityType::File);
    assert_eq!(property.entity_id, file_id);
    assert_eq!(property.namespace, "cloudreve.direct_links");
    assert_eq!(property.name, "100");
    let value: Value = serde_json::from_str(property.value.as_deref().expect("property value"))?;
    assert_eq!(value["url"], written.url);
    assert_eq!(value["source_direct_link_id"], 100);
    assert_eq!(value["source_file_id"], 71);
    assert_eq!(value["source_name"], "legacy-name.txt");
    assert_eq!(value["source_downloads"], 7);
    assert_eq!(value["source_speed_limit"], 1_024);

    assert_ne!(written.url, other_owner.url);
    assert_ne!(written.url, other_secret.url);
    Ok(())
}

#[tokio::test]
async fn invalid_direct_link_target_id_leaves_no_archive_property() -> Result<()> {
    let database = database().await?;
    let transaction = database.begin().await?;
    let result = AsterDriveWriter::new(&transaction)
        .write_direct_link(
            ResolvedDirectLink {
                direct_link: direct_link(101),
                target_file_id: -1,
                target_owner_id: 2,
            },
            "test-direct-link-secret",
        )
        .await;
    assert!(result.is_err());
    transaction.rollback().await?;
    assert_eq!(
        aster_drive_schema::entities::entity_property::Entity::find()
            .filter(
                aster_drive_schema::entities::entity_property::Column::Namespace
                    .eq("cloudreve.direct_links"),
            )
            .count(&database)
            .await?,
        0
    );
    Ok(())
}
