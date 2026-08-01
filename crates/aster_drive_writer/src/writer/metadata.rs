use super::*;

impl AsterDriveWriter<'_> {
    pub async fn write_property(&self, resolved: ResolvedProperty) -> Result<i64> {
        let ResolvedProperty {
            source_metadata_id,
            target,
            namespace,
            name,
            value,
        } = resolved;
        let (entity_type, entity_id) = resolved_entity_target(target);
        let target = aster_drive_schema::entities::entity_property::ActiveModel {
            entity_type: Set(entity_type),
            entity_id: Set(entity_id),
            namespace: Set(namespace),
            name: Set(name),
            value: Set(value),
            ..Default::default()
        }
        .insert(self.transaction)
        .await
        .wrap_err_with(|| format!("migrate Cloudreve metadata {source_metadata_id}"))?;
        Ok(target.id)
    }

    pub async fn write_tag(&self, resolved: ResolvedTag) -> Result<i64> {
        let ResolvedTag {
            source_metadata_id,
            owner_id,
            name,
            normalized_name,
            color,
            created_at,
            updated_at,
        } = resolved;
        let target = aster_drive_schema::entities::tag::ActiveModel {
            scope_type: Set(TagScopeType::Personal),
            owner_user_id: Set(Some(owner_id)),
            team_id: Set(None),
            name: Set(name),
            normalized_name: Set(normalized_name),
            color: Set(color),
            sort_order: Set(0),
            created_at: Set(created_at),
            updated_at: Set(updated_at),
            ..Default::default()
        }
        .insert(self.transaction)
        .await
        .wrap_err_with(|| format!("migrate tag metadata {source_metadata_id}"))?;
        Ok(target.id)
    }

    pub async fn write_tag_assignment(&self, resolved: ResolvedTagAssignment) -> Result<i64> {
        let ResolvedTagAssignment {
            source_metadata_id,
            target,
            tag_id,
        } = resolved;
        let (entity_type, entity_id) = resolved_entity_target(target);
        let target = aster_drive_schema::entities::entity_property::ActiveModel {
            entity_type: Set(entity_type),
            entity_id: Set(entity_id),
            namespace: Set("system.tags".to_string()),
            name: Set(tag_id.to_string()),
            value: Set(None),
            ..Default::default()
        }
        .insert(self.transaction)
        .await
        .wrap_err_with(|| format!("attach migrated tag for metadata {source_metadata_id}"))?;
        Ok(target.id)
    }
}

fn resolved_entity_target(target: ResolvedEntityTarget) -> (EntityType, i64) {
    match target {
        ResolvedEntityTarget::File { target_id } => (EntityType::File, target_id),
        ResolvedEntityTarget::Folder { target_id } => (EntityType::Folder, target_id),
    }
}
