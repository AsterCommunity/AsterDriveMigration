#[derive(Debug, Clone)]
pub struct CloudreveStoragePolicyRecord {
    pub policy: cloudreve_schema::storage_policies::Model,
    pub local_root: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CloudrevePolicyGroupRecord {
    pub group: cloudreve_schema::groups::Model,
}

#[derive(Debug, Clone)]
pub struct CloudreveUserRecord {
    pub user: cloudreve_schema::users::Model,
    pub group: Option<cloudreve_schema::groups::Model>,
    pub username: String,
}

#[derive(Debug, Clone)]
pub struct CloudreveFolderRecord {
    pub folder: cloudreve_schema::files::Model,
}

#[derive(Debug, Clone)]
pub struct CloudreveBlobRecord {
    pub entity: cloudreve_schema::entities::Model,
    pub reference_count: i64,
    pub local_root: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CloudreveFileRecord {
    pub file: cloudreve_schema::files::Model,
    pub entities: Vec<cloudreve_schema::entities::Model>,
}

#[derive(Debug, Clone)]
pub struct CloudreveShareRecord {
    pub share: cloudreve_schema::shares::Model,
    pub target: Option<cloudreve_schema::files::Model>,
}

#[derive(Debug, Clone)]
pub struct CloudreveMetadataRecord {
    pub metadata: cloudreve_schema::metadata::Model,
    pub target: Option<cloudreve_schema::files::Model>,
}

#[derive(Debug, Clone)]
pub struct CloudreveDirectLinkRecord {
    pub direct_link: cloudreve_schema::direct_links::Model,
    pub target: Option<cloudreve_schema::files::Model>,
}
