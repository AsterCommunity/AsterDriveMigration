use super::*;

#[tokio::test]
async fn writes_properties_personal_tags_and_file_folder_assignments() -> Result<()> {
    let database = database().await?;
    let transaction = database.begin().await?;
    let (policy_id, user_id, folder_id) = write_prerequisites(&transaction).await?;
    let file_id = write_file_target(&transaction, policy_id, user_id, folder_id).await?;
    let writer = AsterDriveWriter::new(&transaction);

    writer
        .write_property(ResolvedProperty {
            source_metadata_id: 80,
            target: ResolvedEntityTarget::File { target_id: file_id },
            namespace: "cloudreve.public".to_string(),
            name: "author".to_string(),
            value: Some("Cloudreve".to_string()),
        })
        .await?;
    let now = chrono::Utc::now();
    let tag_id = writer
        .write_tag(ResolvedTag {
            source_metadata_id: 81,
            owner_id: user_id,
            name: "Important".to_string(),
            normalized_name: "important".to_string(),
            color: "#aabbcc".to_string(),
            created_at: now,
            updated_at: now,
        })
        .await?;
    writer
        .write_tag_assignment(ResolvedTagAssignment {
            source_metadata_id: 81,
            target: ResolvedEntityTarget::File { target_id: file_id },
            tag_id,
        })
        .await?;
    writer
        .write_tag_assignment(ResolvedTagAssignment {
            source_metadata_id: 82,
            target: ResolvedEntityTarget::Folder {
                target_id: folder_id,
            },
            tag_id,
        })
        .await?;
    transaction.commit().await?;

    let stored_tag = aster_drive_schema::entities::tag::Entity::find_by_id(tag_id)
        .one(&database)
        .await?
        .expect("written tag");
    assert_eq!(stored_tag.scope_type, TagScopeType::Personal);
    assert_eq!(stored_tag.owner_user_id, Some(user_id));
    assert_eq!(stored_tag.team_id, None);
    assert_eq!(stored_tag.name, "Important");
    assert_eq!(stored_tag.normalized_name, "important");
    assert_eq!(stored_tag.color, "#aabbcc");

    let properties = aster_drive_schema::entities::entity_property::Entity::find()
        .all(&database)
        .await?;
    assert_eq!(properties.len(), 3);
    assert!(properties.iter().any(|property| {
        property.entity_type == EntityType::File
            && property.entity_id == file_id
            && property.namespace == "cloudreve.public"
            && property.name == "author"
            && property.value.as_deref() == Some("Cloudreve")
    }));
    assert_eq!(
        properties
            .iter()
            .filter(|property| property.namespace == "system.tags")
            .count(),
        2
    );
    assert!(properties.iter().any(|property| {
        property.entity_type == EntityType::Folder
            && property.entity_id == folder_id
            && property.namespace == "system.tags"
            && property.name == tag_id.to_string()
            && property.value.is_none()
    }));
    Ok(())
}

#[tokio::test]
async fn duplicate_metadata_key_rolls_back_tag_and_assignment_transaction() -> Result<()> {
    let database = database().await?;
    let setup = database.begin().await?;
    let (_, user_id, folder_id) = write_prerequisites(&setup).await?;
    setup.commit().await?;

    let transaction = database.begin().await?;
    let writer = AsterDriveWriter::new(&transaction);
    let now = chrono::Utc::now();
    let tag_id = writer
        .write_tag(ResolvedTag {
            source_metadata_id: 90,
            owner_id: user_id,
            name: "Important".to_string(),
            normalized_name: "important".to_string(),
            color: "#aabbcc".to_string(),
            created_at: now,
            updated_at: now,
        })
        .await?;
    let assignment = || ResolvedTagAssignment {
        source_metadata_id: 90,
        target: ResolvedEntityTarget::Folder {
            target_id: folder_id,
        },
        tag_id,
    };
    writer.write_tag_assignment(assignment()).await?;
    let duplicate = writer.write_tag_assignment(assignment()).await;
    assert!(duplicate.is_err());
    transaction.rollback().await?;

    assert_eq!(
        aster_drive_schema::entities::tag::Entity::find()
            .count(&database)
            .await?,
        0
    );
    assert_eq!(
        aster_drive_schema::entities::entity_property::Entity::find()
            .filter(
                aster_drive_schema::entities::entity_property::Column::Namespace.eq("system.tags"),
            )
            .count(&database)
            .await?,
        0
    );
    Ok(())
}
