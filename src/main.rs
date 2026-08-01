#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::unreachable,
        clippy::expect_used,
        clippy::panic,
        clippy::unimplemented,
        clippy::todo
    )
)]

use std::path::{Path, PathBuf};

use aster_drive_migration::migration::{
    MigrationOptions, MigrationReport, abort_migration_run, cleanup_completed_migration_run,
    inspect, list_migration_runs, migrate, migration_run_report, migration_run_status,
    write_csv_mapping_report, write_json_report,
};
use clap::{Parser, Subcommand};
use color_eyre::eyre::{Result, bail};

#[derive(Debug, Parser)]
#[command(name = "aster-drive-migration", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Check(ConnectionArgs),
    Migrate(MigrateArgs),
    Resume(MigrateArgs),
    List(TargetArgs),
    Status(RunArgs),
    Report(ReportArgs),
    Abort(RunArgs),
    Cleanup(CleanupArgs),
}

#[derive(Debug, clap::Args)]
struct TargetArgs {
    #[arg(long, env = "ASTERDRIVE_DATABASE_URL")]
    target_url: String,
}

#[derive(Debug, clap::Args)]
struct RunArgs {
    #[command(flatten)]
    target: TargetArgs,
    #[arg(long, value_name = "ID")]
    run_id: String,
}

#[derive(Debug, clap::Args)]
struct ReportArgs {
    #[command(flatten)]
    run: RunArgs,
    #[arg(long, value_name = "PATH")]
    report_path: Option<PathBuf>,
    #[arg(long, value_name = "PATH")]
    csv_mapping_path: Option<PathBuf>,
}

#[derive(Debug, clap::Args)]
struct CleanupArgs {
    #[command(flatten)]
    run: RunArgs,
    #[arg(long)]
    confirm: bool,
}

#[derive(Debug, clap::Args)]
struct ConnectionArgs {
    #[arg(long, env = "CLOUDREVE_DATABASE_URL")]
    source_url: String,
    #[arg(long, env = "ASTERDRIVE_DATABASE_URL")]
    target_url: String,
    #[arg(long)]
    include_deleted: bool,
    #[arg(long, value_name = "PATH")]
    report_path: Option<PathBuf>,
}

#[derive(Debug, clap::Args)]
struct MigrateArgs {
    #[command(flatten)]
    connection: ConnectionArgs,
    #[arg(long, env = "ASTER_MIGRATION_DEFAULT_PASSWORD")]
    default_password: String,
    #[arg(long, default_value = ".")]
    local_base_path: String,
    #[arg(long = "local-policy-root", value_name = "SOURCE_POLICY_ID=PATH")]
    local_policy_roots: Vec<String>,
    #[arg(long)]
    verify_local_storage: bool,
    #[arg(long)]
    verify_remote_storage: bool,
    #[arg(long, env = "ASTER_DIRECT_LINK_SECRET", hide_env_values = true)]
    direct_link_secret: Option<String>,
    #[arg(long)]
    allow_non_empty_target: bool,
    #[arg(long)]
    skip_unsupported_policies: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long, value_name = "ID")]
    run_id: Option<String>,
    #[arg(long, requires = "run_id")]
    resume: bool,
    #[arg(long, default_value_t = 500, value_name = "COUNT")]
    blob_batch_size: usize,
    #[arg(long, default_value_t = 500, value_name = "COUNT")]
    file_batch_size: usize,
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    match Cli::parse().command {
        Command::Check(args) => {
            let report = inspect(&args.source_url, &args.target_url, args.include_deleted).await?;
            emit_report(args.report_path.as_deref(), &report)?;
        }
        Command::Migrate(args) => {
            run_migration(args, false).await?;
        }
        Command::Resume(args) => {
            run_migration(args, true).await?;
        }
        Command::List(args) => {
            for run in list_migration_runs(&args.target_url).await? {
                println!(
                    "{}\t{}\t{}\t{}",
                    run.run_id,
                    run.status,
                    run.last_completed_stage.as_deref().unwrap_or("-"),
                    run.updated_at
                );
            }
        }
        Command::Status(args) => {
            let status = migration_run_status(&args.target.target_url, &args.run_id).await?;
            println!("run_id: {}", status.run_id);
            println!("status: {}", status.status);
            println!(
                "last_completed_stage: {}",
                status.last_completed_stage.as_deref().unwrap_or("-")
            );
            println!("updated_at: {}", status.updated_at);
            if let Some(error) = status.last_error {
                println!("last_error: {error}");
            }
            let report = migration_run_report(&args.target.target_url, &args.run_id).await?;
            println!("{report}");
        }
        Command::Report(args) => {
            let report =
                migration_run_report(&args.run.target.target_url, &args.run.run_id).await?;
            emit_report(args.report_path.as_deref(), &report)?;
            if let Some(path) = args.csv_mapping_path.as_deref() {
                write_csv_mapping_report(path, &report)?;
            }
        }
        Command::Abort(args) => {
            abort_migration_run(&args.target.target_url, &args.run_id).await?;
            println!("migration run {} was marked aborted", args.run_id);
        }
        Command::Cleanup(args) => {
            if !args.confirm {
                bail!("cleanup only removes completed checkpoint metadata; rerun with --confirm");
            }
            cleanup_completed_migration_run(&args.run.target.target_url, &args.run.run_id).await?;
            println!(
                "completed migration checkpoint {} was removed",
                args.run.run_id
            );
        }
    }
    Ok(())
}

async fn run_migration(args: MigrateArgs, force_resume: bool) -> Result<()> {
    let report_path = args.connection.report_path.clone();
    if force_resume && args.run_id.is_none() {
        bail!("resume requires --run-id");
    }
    let report = migrate(MigrationOptions {
        source_url: args.connection.source_url,
        target_url: args.connection.target_url,
        default_password: args.default_password,
        local_base_path: args.local_base_path,
        local_policy_roots: parse_local_policy_roots(args.local_policy_roots)?,
        verify_local_storage: args.verify_local_storage,
        verify_remote_storage: args.verify_remote_storage,
        direct_link_secret: args.direct_link_secret,
        include_deleted: args.connection.include_deleted,
        allow_non_empty_target: args.allow_non_empty_target,
        skip_unsupported_policies: args.skip_unsupported_policies,
        dry_run: args.dry_run,
        run_id: args.run_id,
        resume: force_resume || args.resume,
        blob_batch_size: args.blob_batch_size,
        file_batch_size: args.file_batch_size,
    })
    .await?;
    emit_report(report_path.as_deref(), &report)
}

fn parse_local_policy_roots(
    values: Vec<String>,
) -> Result<std::collections::BTreeMap<i64, String>> {
    let mut roots = std::collections::BTreeMap::new();
    for value in values {
        let Some((source_policy_id, path)) = value.split_once('=') else {
            bail!("--local-policy-root must use SOURCE_POLICY_ID=PATH");
        };
        let source_policy_id = source_policy_id.parse::<i64>().map_err(|_| {
            color_eyre::eyre::eyre!("invalid local storage policy ID {source_policy_id}")
        })?;
        if source_policy_id <= 0 || path.trim().is_empty() {
            bail!("--local-policy-root must use a positive policy ID and non-empty path");
        }
        if roots.insert(source_policy_id, path.to_string()).is_some() {
            bail!("local storage policy {source_policy_id} was configured more than once");
        }
    }
    Ok(roots)
}

fn emit_report(report_path: Option<&Path>, report: &MigrationReport) -> Result<()> {
    if let Some(path) = report_path {
        write_json_report(path, report)?;
    }
    println!("{report}");
    if report.validation.performed && !report.validation.passed {
        bail!("migration committed but post-migration validation failed; inspect the JSON report");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_local_policy_roots() -> Result<()> {
        let roots = parse_local_policy_roots(vec![
            "1=D:/cloudreve-data".to_string(),
            "2=E:/cloudreve-archive".to_string(),
        ])?;
        assert_eq!(roots.get(&1).map(String::as_str), Some("D:/cloudreve-data"));
        assert_eq!(
            roots.get(&2).map(String::as_str),
            Some("E:/cloudreve-archive")
        );
        Ok(())
    }

    #[test]
    fn rejects_invalid_local_policy_roots() {
        assert!(parse_local_policy_roots(vec!["missing-separator".to_string()]).is_err());
        assert!(parse_local_policy_roots(vec!["0=C:/data".to_string()]).is_err());
        assert!(
            parse_local_policy_roots(vec!["1=C:/data".to_string(), "1=D:/data".to_string()])
                .is_err()
        );
    }

    #[test]
    fn parses_operational_commands() -> Result<()> {
        let list = Cli::try_parse_from([
            "aster-drive-migration",
            "list",
            "--target-url",
            "sqlite://target.db",
        ])?;
        assert!(matches!(list.command, Command::List(_)));

        let cleanup = Cli::try_parse_from([
            "aster-drive-migration",
            "cleanup",
            "--target-url",
            "sqlite://target.db",
            "--run-id",
            "cutover-1",
            "--confirm",
        ])?;
        assert!(matches!(cleanup.command, Command::Cleanup(_)));
        Ok(())
    }

    #[test]
    fn parses_check_and_complete_migration_flags() -> Result<()> {
        let check = Cli::try_parse_from([
            "aster-drive-migration",
            "check",
            "--source-url",
            "sqlite://source.db",
            "--target-url",
            "sqlite://target.db",
            "--include-deleted",
            "--report-path",
            "report.json",
        ])?;
        let Command::Check(check) = check.command else {
            panic!("expected check command");
        };
        assert!(check.include_deleted);
        assert_eq!(check.report_path.as_deref(), Some(Path::new("report.json")));

        let migrate = Cli::try_parse_from([
            "aster-drive-migration",
            "migrate",
            "--source-url",
            "sqlite://source.db",
            "--target-url",
            "sqlite://target.db",
            "--default-password",
            "temporary-password",
            "--direct-link-secret",
            "direct-link-secret",
            "--local-policy-root",
            "3=/storage",
            "--verify-local-storage",
            "--verify-remote-storage",
            "--allow-non-empty-target",
            "--skip-unsupported-policies",
            "--include-deleted",
            "--dry-run",
            "--blob-batch-size",
            "1",
            "--file-batch-size",
            "10000",
        ])?;
        let Command::Migrate(migrate) = migrate.command else {
            panic!("expected migrate command");
        };
        assert!(migrate.verify_local_storage);
        assert!(migrate.verify_remote_storage);
        assert!(migrate.allow_non_empty_target);
        assert!(migrate.skip_unsupported_policies);
        assert!(migrate.connection.include_deleted);
        assert!(migrate.dry_run);
        assert_eq!(migrate.local_policy_roots, ["3=/storage"]);
        assert_eq!(migrate.blob_batch_size, 1);
        assert_eq!(migrate.file_batch_size, 10_000);
        Ok(())
    }

    #[test]
    fn cli_rejects_missing_required_values_and_resume_without_run_id() {
        assert!(Cli::try_parse_from(["aster-drive-migration", "check"]).is_err());
        assert!(
            Cli::try_parse_from([
                "aster-drive-migration",
                "migrate",
                "--source-url",
                "sqlite://source.db",
                "--target-url",
                "sqlite://target.db",
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "aster-drive-migration",
                "migrate",
                "--source-url",
                "sqlite://source.db",
                "--target-url",
                "sqlite://target.db",
                "--default-password",
                "temporary-password",
                "--resume",
            ])
            .is_err()
        );
    }
}
