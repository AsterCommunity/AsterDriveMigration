use super::*;

pub(super) const REPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct MigrationOptions {
    pub source_url: String,
    pub target_url: String,
    pub default_password: String,
    pub local_base_path: String,
    pub local_policy_roots: BTreeMap<i64, String>,
    pub verify_local_storage: bool,
    pub verify_remote_storage: bool,
    pub direct_link_secret: Option<String>,
    pub include_deleted: bool,
    pub allow_non_empty_target: bool,
    pub skip_unsupported_policies: bool,
    pub dry_run: bool,
    pub run_id: Option<String>,
    pub resume: bool,
    pub blob_batch_size: usize,
    pub file_batch_size: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct MigrationRunSummary {
    pub run_id: String,
    pub status: String,
    pub last_completed_stage: Option<String>,
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
    pub updated_at: chrono::DateTime<chrono::FixedOffset>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationReport {
    pub schema_version: u32,
    pub generated_at: chrono::DateTime<chrono::Utc>,
    pub source_users: u64,
    pub source_groups: u64,
    pub source_policies: u64,
    pub source_folders: u64,
    pub source_files: u64,
    pub source_entities: u64,
    pub source_shares: u64,
    pub source_direct_links: u64,
    pub source_tag_assignments: u64,
    pub source_tasks: u64,
    pub migrated_users: usize,
    pub migrated_policy_groups: usize,
    pub migrated_policies: usize,
    pub migrated_folders: usize,
    pub migrated_files: usize,
    pub migrated_blobs: usize,
    pub migrated_versions: usize,
    pub migrated_shares: usize,
    pub migrated_properties: usize,
    pub migrated_tags: usize,
    pub migrated_tag_assignments: usize,
    pub migrated_direct_links: usize,
    pub skipped: usize,
    pub dry_run: bool,
    pub warnings: Vec<String>,
    pub skipped_by_type: BTreeMap<String, usize>,
    pub skipped_objects: Vec<SkippedObject>,
    pub mappings: MigrationMappings,
    pub direct_links: Vec<DirectLinkReport>,
    pub tag_assignments: Vec<TagAssignmentReport>,
    pub validation: MigrationValidation,
    #[serde(default)]
    pub preflight: MigrationPreflight,
    pub run_id: Option<String>,
    pub resumed: bool,
    pub completed_stages: Vec<String>,
}

impl Default for MigrationReport {
    fn default() -> Self {
        Self {
            schema_version: REPORT_SCHEMA_VERSION,
            generated_at: chrono::Utc::now(),
            source_users: 0,
            source_groups: 0,
            source_policies: 0,
            source_folders: 0,
            source_files: 0,
            source_entities: 0,
            source_shares: 0,
            source_direct_links: 0,
            source_tag_assignments: 0,
            source_tasks: 0,
            migrated_users: 0,
            migrated_policy_groups: 0,
            migrated_policies: 0,
            migrated_folders: 0,
            migrated_files: 0,
            migrated_blobs: 0,
            migrated_versions: 0,
            migrated_shares: 0,
            migrated_properties: 0,
            migrated_tags: 0,
            migrated_tag_assignments: 0,
            migrated_direct_links: 0,
            skipped: 0,
            dry_run: true,
            warnings: Vec::new(),
            skipped_by_type: BTreeMap::new(),
            skipped_objects: Vec::new(),
            mappings: MigrationMappings::default(),
            direct_links: Vec::new(),
            tag_assignments: Vec::new(),
            validation: MigrationValidation::default(),
            preflight: MigrationPreflight::default(),
            run_id: None,
            resumed: false,
            completed_stages: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedObject {
    pub object_type: String,
    pub source_id: Option<i64>,
    pub reason: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct MigrationMappings {
    pub policies: Vec<IdMapping>,
    pub policy_groups: Vec<IdMapping>,
    pub users: Vec<IdMapping>,
    pub folders: Vec<IdMapping>,
    pub blobs: Vec<IdMapping>,
    pub files: Vec<IdMapping>,
    pub shares: Vec<IdMapping>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IdMapping {
    pub source_id: i64,
    pub target_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectLinkReport {
    pub source_direct_link_id: i64,
    pub source_file_id: i64,
    pub target_file_id: i64,
    pub source_name: String,
    pub source_downloads: i64,
    pub source_speed_limit: i64,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagAssignmentReport {
    pub source_metadata_id: i64,
    pub source_entity_id: i64,
    pub target_entity_type: String,
    pub target_entity_id: i64,
    pub target_tag_id: i64,
    pub tag_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationValidation {
    pub performed: bool,
    pub passed: bool,
    pub checks: Vec<ValidationCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationPreflight {
    pub performed: bool,
    pub passed: bool,
    pub checks: Vec<ValidationCheck>,
}

impl Default for MigrationPreflight {
    fn default() -> Self {
        Self {
            performed: false,
            passed: true,
            checks: Vec::new(),
        }
    }
}

impl Default for MigrationValidation {
    fn default() -> Self {
        Self {
            performed: false,
            passed: true,
            checks: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationCheck {
    pub name: String,
    pub passed: bool,
    pub expected: String,
    pub actual: String,
    pub message: Option<String>,
}

impl MigrationReport {
    pub(super) fn record_skip(
        &mut self,
        object_type: &str,
        source_id: Option<i64>,
        reason: impl Into<String>,
    ) {
        self.skipped += 1;
        *self
            .skipped_by_type
            .entry(object_type.to_string())
            .or_default() += 1;
        self.skipped_objects.push(SkippedObject {
            object_type: object_type.to_string(),
            source_id,
            reason: reason.into(),
        });
    }

    pub(super) fn set_mappings(
        &mut self,
        context: &MigrationContext,
        blobs: &HashMap<i64, i64>,
        files: &HashMap<i64, i64>,
    ) {
        self.mappings = MigrationMappings {
            policies: sorted_id_mappings(&context.policies),
            policy_groups: sorted_id_mappings(&context.policy_groups),
            users: sorted_id_mappings(&context.users),
            folders: sorted_id_mappings(&context.folders),
            blobs: sorted_id_mappings(blobs),
            files: sorted_id_mappings(files),
            shares: sorted_id_mappings(&context.shares),
        };
    }
}

impl fmt::Display for MigrationReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut output = String::new();
        writeln!(output, "Cloudreve -> AsterDrive migration report")?;
        if let Some(run_id) = &self.run_id {
            writeln!(
                output,
                "run: {run_id}{}",
                if self.resumed { " (resumed)" } else { "" }
            )?;
        }
        writeln!(
            output,
            "source: users={}, groups={}, policies={}, folders={}, files={}, entities={}, shares={}, direct_links={}, tag_assignments={}, tasks={}",
            self.source_users,
            self.source_groups,
            self.source_policies,
            self.source_folders,
            self.source_files,
            self.source_entities,
            self.source_shares,
            self.source_direct_links,
            self.source_tag_assignments,
            self.source_tasks
        )?;
        if self.dry_run {
            writeln!(output, "mode: dry-run (target was not modified)")?;
        } else {
            writeln!(
                output,
                "migrated: users={}, policy_groups={}, policies={}, folders={}, files={}, blobs={}, versions={}, shares={}, properties={}, tags={}, tag_assignments={}, direct_links={}",
                self.migrated_users,
                self.migrated_policy_groups,
                self.migrated_policies,
                self.migrated_folders,
                self.migrated_files,
                self.migrated_blobs,
                self.migrated_versions,
                self.migrated_shares,
                self.migrated_properties,
                self.migrated_tags,
                self.migrated_tag_assignments,
                self.migrated_direct_links
            )?;
            writeln!(output, "skipped: {}", self.skipped)?;
            if !self.skipped_by_type.is_empty() {
                let categories = self
                    .skipped_by_type
                    .iter()
                    .map(|(object_type, count)| format!("{object_type}={count}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                writeln!(output, "skipped_by_type: {categories}")?;
            }
            if !self.skipped_objects.is_empty() {
                writeln!(output, "skipped_objects:")?;
                for skipped in &self.skipped_objects {
                    let source_id = skipped
                        .source_id
                        .map_or_else(|| "-".to_string(), |source_id| source_id.to_string());
                    writeln!(
                        output,
                        "- {} {}: {}",
                        skipped.object_type, source_id, skipped.reason
                    )?;
                }
            }
            if self.validation.performed {
                writeln!(
                    output,
                    "validation: {} ({} checks)",
                    if self.validation.passed {
                        "passed"
                    } else {
                        "failed"
                    },
                    self.validation.checks.len()
                )?;
            }
            if !self.completed_stages.is_empty() {
                writeln!(
                    output,
                    "completed_stages: {}",
                    self.completed_stages.join(",")
                )?;
            }
        }
        if self.preflight.performed {
            writeln!(
                output,
                "preflight: {} ({} checks)",
                if self.preflight.passed {
                    "passed"
                } else {
                    "failed"
                },
                self.preflight.checks.len()
            )?;
        }
        if !self.warnings.is_empty() {
            writeln!(output, "warnings:")?;
            for warning in &self.warnings {
                writeln!(output, "- {warning}")?;
            }
        }
        formatter.write_str(output.trim_end())
    }
}

pub fn write_json_report(path: impl AsRef<Path>, report: &MigrationReport) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .wrap_err_with(|| format!("create report directory {}", parent.display()))?;
    }
    let contents = serde_json::to_vec_pretty(report).wrap_err("serialize migration report")?;
    std::fs::write(path, contents)
        .wrap_err_with(|| format!("write migration report {}", path.display()))
}

pub fn write_csv_mapping_report(path: impl AsRef<Path>, report: &MigrationReport) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .wrap_err_with(|| format!("create mapping report directory {}", parent.display()))?;
    }
    let mut output = String::from("object_type,source_id,target_id\n");
    for (object_type, mappings) in [
        ("policy", &report.mappings.policies),
        ("policy_group", &report.mappings.policy_groups),
        ("user", &report.mappings.users),
        ("folder", &report.mappings.folders),
        ("blob", &report.mappings.blobs),
        ("file", &report.mappings.files),
        ("share", &report.mappings.shares),
    ] {
        for mapping in mappings {
            writeln!(
                output,
                "{object_type},{},{}",
                mapping.source_id, mapping.target_id
            )?;
        }
    }
    std::fs::write(path, output)
        .wrap_err_with(|| format!("write migration CSV mapping report {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn records_skipped_objects_by_type() {
        let mut report = MigrationReport::default();
        report.record_skip("file", Some(42), "missing blob");
        report.record_skip("file", Some(43), "symbolic file");
        report.record_skip("share", None, "missing target");

        assert_eq!(report.skipped, 3);
        assert_eq!(report.skipped_by_type.get("file"), Some(&2));
        assert_eq!(report.skipped_by_type.get("share"), Some(&1));
        assert_eq!(report.skipped_objects[0].source_id, Some(42));
        assert_eq!(report.skipped_objects[0].reason, "missing blob");
    }

    #[test]
    fn displays_skipped_object_type_id_and_reason() {
        let mut report = MigrationReport {
            dry_run: false,
            ..Default::default()
        };
        report.record_skip(
            "blob",
            Some(42),
            "Cloudreve encrypted entity is not supported",
        );

        let output = report.to_string();
        assert!(output.contains("skipped_by_type: blob=1"));
        assert!(output.contains("- blob 42: Cloudreve encrypted entity is not supported"));
    }

    #[test]
    fn writes_structured_json_report() -> Result<()> {
        let report_path = std::env::temp_dir().join(format!(
            "asterdrive-migration-report-{}.json",
            uuid::Uuid::new_v4()
        ));
        let mut report = MigrationReport {
            migrated_users: 1,
            validation: MigrationValidation {
                performed: true,
                passed: true,
                checks: vec![ValidationCheck {
                    name: "users_count".to_string(),
                    passed: true,
                    expected: "1".to_string(),
                    actual: "1".to_string(),
                    message: None,
                }],
            },
            ..Default::default()
        };
        report.mappings.users.push(IdMapping {
            source_id: 7,
            target_id: 11,
        });

        write_json_report(&report_path, &report)?;
        let value: Value = serde_json::from_slice(&std::fs::read(&report_path)?)?;
        assert_eq!(value["schema_version"], REPORT_SCHEMA_VERSION);
        assert_eq!(value["migrated_users"], 1);
        assert_eq!(value["mappings"]["users"][0]["source_id"], 7);
        assert_eq!(value["mappings"]["users"][0]["target_id"], 11);
        assert_eq!(value["validation"]["passed"], true);

        let _ = std::fs::remove_file(report_path);
        Ok(())
    }

    #[test]
    fn writes_csv_mapping_report_without_capability_urls() -> Result<()> {
        let report_path = std::env::temp_dir().join(format!(
            "asterdrive-migration-mappings-{}.csv",
            uuid::Uuid::new_v4()
        ));
        let mut report = MigrationReport::default();
        report.mappings.users.push(IdMapping {
            source_id: 7,
            target_id: 11,
        });
        report.direct_links.push(DirectLinkReport {
            source_direct_link_id: 1,
            source_file_id: 2,
            target_file_id: 3,
            source_name: "secret.txt".to_string(),
            source_downloads: 0,
            source_speed_limit: 0,
            url: "/d/capability".to_string(),
        });

        write_csv_mapping_report(&report_path, &report)?;
        let contents = std::fs::read_to_string(&report_path)?;
        assert!(contents.contains("user,7,11"));
        assert!(!contents.contains("/d/capability"));
        let _ = std::fs::remove_file(report_path);
        Ok(())
    }
}
