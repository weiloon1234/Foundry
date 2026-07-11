use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use clap::{Arg, ArgAction, Command};
use serde::Serialize;

use crate::cli::{CommandInvocation, CommandRegistrar};
use crate::database::{DbValue, Expr, Query};
use crate::foundation::{AppContext, Error, Result};
use crate::storage::{path::normalize_prefix, StorageObject};
use crate::support::{CommandId, DateTime};

const ATTACHMENT_ORPHANS_COMMAND: CommandId = CommandId::new("attachment:orphans");
const ATTACHMENT_ORPHANS_LOCK: &str = "attachments:orphan_audit";
const LIST_PREFIX_UNSUPPORTED: &str = "storage adapter does not support prefix listing";
const LIST_PAGINATION_UNSUPPORTED: &str = "storage adapter does not support prefix pagination";

#[derive(Clone, Debug)]
pub(crate) struct AttachmentOrphanOptions {
    pub disk: Option<String>,
    pub prefix: String,
    pub limit: usize,
    pub older_than_seconds: u64,
    pub delete: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct AttachmentOrphanReport {
    pub prefix: String,
    pub older_than_seconds: u64,
    pub limit: usize,
    pub pages_scanned: usize,
    pub objects_scanned: usize,
    pub delete: bool,
    pub candidate_count: usize,
    pub deleted_count: usize,
    pub disks: Vec<AttachmentOrphanDiskReport>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AttachmentOrphanDiskReport {
    pub disk: String,
    pub supported: bool,
    pub pages_scanned: usize,
    pub objects_scanned: usize,
    pub candidate_count: usize,
    pub deleted_count: usize,
    pub candidates: Vec<AttachmentOrphanCandidate>,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct AttachmentOrphanCandidate {
    pub disk: String,
    pub path: String,
    pub size: u64,
    pub modified_at: DateTime,
    pub age_seconds: u64,
}

pub(crate) fn builtin_cli_registrar() -> CommandRegistrar {
    Arc::new(|registry| {
        registry.command(
            ATTACHMENT_ORPHANS_COMMAND,
            Command::new(ATTACHMENT_ORPHANS_COMMAND.as_str().to_string())
                .about("Audit or delete old attachment storage objects missing from the attachments table")
                .arg(
                    Arg::new("json")
                        .long("json")
                        .action(ArgAction::SetTrue)
                        .help("Print report as JSON"),
                )
                .arg(
                    Arg::new("disk")
                        .long("disk")
                        .value_name("NAME")
                        .help("Audit only one configured storage disk"),
                )
                .arg(
                    Arg::new("limit")
                        .long("limit")
                        .value_name("N")
                        .help("Storage objects fetched per scan page"),
                )
                .arg(
                    Arg::new("older_than_seconds")
                        .long("older-than-seconds")
                        .value_name("SECONDS")
                        .help("Only report objects older than this age"),
                )
                .arg(
                    Arg::new("delete")
                        .long("delete")
                        .action(ArgAction::SetTrue)
                        .help("Delete candidates; requires storage.attachment_orphan_delete_enabled = true"),
                ),
            |invocation| async move { attachment_orphans_command(invocation).await },
        )?;
        Ok(())
    })
}

pub(crate) async fn audit_attachment_orphans_with_lock(
    app: &AppContext,
    options: AttachmentOrphanOptions,
) -> Result<Option<AttachmentOrphanReport>> {
    let Ok(db) = app.database() else {
        return Ok(None);
    };
    if !db.is_configured() {
        return Ok(None);
    }

    let Ok(lock) = app.lock() else {
        return Ok(None);
    };
    let Some(_guard) = lock
        .acquire(ATTACHMENT_ORPHANS_LOCK, Duration::from_secs(60))
        .await?
    else {
        return Ok(None);
    };

    audit_attachment_orphans(app, options).await.map(Some)
}

pub(crate) async fn audit_attachment_orphans(
    app: &AppContext,
    options: AttachmentOrphanOptions,
) -> Result<AttachmentOrphanReport> {
    let storage_config = app.config().storage()?;
    if options.delete && !storage_config.attachment_orphan_delete_enabled {
        return Err(Error::message(
            "attachment orphan deletion is disabled; set storage.attachment_orphan_delete_enabled = true before using --delete",
        ));
    }

    let storage = app.storage()?;
    let db = app.database()?;
    if !db.is_configured() {
        return Err(Error::message(
            "attachment orphan audit requires a configured database",
        ));
    }

    let options = AttachmentOrphanOptions {
        prefix: normalize_prefix(&options.prefix)?,
        ..options
    };
    if options.limit == 0 {
        return Err(Error::message(
            "attachment orphan scan page size must be greater than zero",
        ));
    }

    let disk_names = match options.disk.as_deref() {
        Some(name) => vec![name.to_string()],
        None => storage.configured_disks(),
    };
    let now = DateTime::now();
    let mut disks = Vec::new();
    let mut pages_scanned = 0;
    let mut objects_scanned = 0;
    let mut candidate_count = 0;
    let mut deleted_count = 0;

    for disk_name in disk_names {
        let disk = storage.disk(&disk_name)?;
        let mut disk_report = AttachmentOrphanDiskReport {
            disk: disk_name.clone(),
            supported: true,
            pages_scanned: 0,
            objects_scanned: 0,
            candidate_count: 0,
            deleted_count: 0,
            candidates: Vec::new(),
            errors: Vec::new(),
        };

        let referenced = referenced_attachment_paths(app, &disk_name, &options.prefix).await?;
        let mut after = None;
        loop {
            let objects = match disk
                .list_prefix_after(&options.prefix, after.as_deref(), options.limit)
                .await
            {
                Ok(objects) => objects,
                Err(error) if is_listing_unsupported(&error) => {
                    disk_report.supported = false;
                    disk_report.errors.push(error.to_string());
                    break;
                }
                Err(error) => return Err(error),
            };
            if objects.is_empty() {
                break;
            }
            validate_storage_page(&objects, after.as_deref())?;

            let page_len = objects.len();
            let next_after = objects
                .last()
                .map(|object| object.path.clone())
                .ok_or_else(|| Error::message("attachment orphan scan returned an empty page"))?;
            disk_report.pages_scanned += 1;
            disk_report.objects_scanned += page_len;

            let candidates = orphan_candidates_from_objects(
                &disk_name,
                objects,
                &referenced,
                options.older_than_seconds,
                now,
            );

            for candidate in &candidates {
                tracing::warn!(
                    target: "foundry.attachments",
                    disk = %candidate.disk,
                    path = %candidate.path,
                    age_seconds = candidate.age_seconds,
                    size = candidate.size,
                    "attachment orphan candidate found"
                );
                if options.delete {
                    match disk.delete(&candidate.path).await {
                        Ok(()) => {
                            disk_report.deleted_count += 1;
                            deleted_count += 1;
                        }
                        Err(error) => {
                            let message = format!("failed to delete `{}`: {error}", candidate.path);
                            tracing::warn!(
                                target: "foundry.attachments",
                                disk = %candidate.disk,
                                path = %candidate.path,
                                error = %error,
                                "failed to delete attachment orphan candidate"
                            );
                            disk_report.errors.push(message);
                        }
                    }
                }
            }
            disk_report.candidates.extend(candidates);

            if page_len < options.limit {
                break;
            }
            after = Some(next_after);
        }

        disk_report.candidate_count = disk_report.candidates.len();
        pages_scanned += disk_report.pages_scanned;
        objects_scanned += disk_report.objects_scanned;
        candidate_count += disk_report.candidate_count;
        disks.push(disk_report);
    }

    Ok(AttachmentOrphanReport {
        prefix: options.prefix,
        older_than_seconds: options.older_than_seconds,
        limit: options.limit,
        pages_scanned,
        objects_scanned,
        delete: options.delete,
        candidate_count,
        deleted_count,
        disks,
    })
}

pub(crate) fn orphan_candidates_from_objects(
    disk: &str,
    objects: Vec<StorageObject>,
    referenced: &HashSet<String>,
    older_than_seconds: u64,
    now: DateTime,
) -> Vec<AttachmentOrphanCandidate> {
    let now_ms = now.timestamp_millis();
    let min_age_ms = older_than_seconds.saturating_mul(1000) as i64;
    objects
        .into_iter()
        .filter(|object| !referenced.contains(&object.path))
        .filter_map(|object| {
            let age_ms = now_ms.saturating_sub(object.modified_at.timestamp_millis());
            if age_ms < min_age_ms {
                return None;
            }
            Some(AttachmentOrphanCandidate {
                disk: disk.to_string(),
                path: object.path,
                size: object.size,
                modified_at: object.modified_at,
                age_seconds: (age_ms / 1000).max(0) as u64,
            })
        })
        .collect()
}

async fn attachment_orphans_command(invocation: CommandInvocation) -> Result<()> {
    let storage = invocation.app().config().storage()?;
    let options = AttachmentOrphanOptions {
        disk: invocation.matches().get_one::<String>("disk").cloned(),
        prefix: storage.attachment_orphan_prefix.clone(),
        limit: parse_optional_usize(&invocation, "limit")?
            .unwrap_or(storage.attachment_orphan_prune_batch_size as usize),
        older_than_seconds: parse_optional_u64(&invocation, "older_than_seconds")?
            .unwrap_or(storage.attachment_orphan_retention_seconds),
        delete: invocation.matches().get_flag("delete"),
    };

    let json = invocation.matches().get_flag("json");
    let report = audit_attachment_orphans(invocation.app(), options).await?;
    if json {
        invocation.line(serde_json::to_string_pretty(&report).map_err(Error::other)?)?;
    } else {
        for line in text_report_lines(&report) {
            invocation.line(line)?;
        }
    }
    Ok(())
}

async fn referenced_attachment_paths(
    app: &AppContext,
    disk: &str,
    prefix: &str,
) -> Result<HashSet<String>> {
    let db = app.database()?;
    let rows = Query::table("attachments")
        .select(["path"])
        .where_eq("disk", disk.to_string())
        .where_(Expr::column("path").like(escaped_like_prefix(prefix)))
        .get(db.as_ref())
        .await?;

    Ok(rows
        .iter()
        .filter_map(|row| match row.get("path") {
            Some(DbValue::Text(path)) => Some(path.clone()),
            _ => None,
        })
        .collect())
}

fn escaped_like_prefix(prefix: &str) -> String {
    let mut escaped = String::with_capacity(prefix.len() + 1);
    for ch in prefix.chars() {
        match ch {
            '\\' | '%' | '_' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    escaped.push('%');
    escaped
}

fn is_listing_unsupported(error: &Error) -> bool {
    let message = error.to_string();
    message.contains(LIST_PREFIX_UNSUPPORTED) || message.contains(LIST_PAGINATION_UNSUPPORTED)
}

fn validate_storage_page(objects: &[StorageObject], after: Option<&str>) -> Result<()> {
    let mut previous = after;
    for object in objects {
        if previous.is_some_and(|previous| object.path.as_str() <= previous) {
            return Err(Error::message(format!(
                "storage prefix pagination must return strictly increasing paths; received `{}` after `{}`",
                object.path,
                previous.unwrap_or_default()
            )));
        }
        previous = Some(&object.path);
    }
    Ok(())
}

fn parse_optional_usize(invocation: &CommandInvocation, name: &str) -> Result<Option<usize>> {
    invocation
        .matches()
        .get_one::<String>(name)
        .map(|value| {
            value.parse::<usize>().map_err(|_| {
                Error::message(format!(
                    "--{} must be a non-negative integer",
                    flag_name(name)
                ))
            })
        })
        .transpose()
}

fn parse_optional_u64(invocation: &CommandInvocation, name: &str) -> Result<Option<u64>> {
    invocation
        .matches()
        .get_one::<String>(name)
        .map(|value| {
            value.parse::<u64>().map_err(|_| {
                Error::message(format!(
                    "--{} must be a non-negative integer",
                    flag_name(name)
                ))
            })
        })
        .transpose()
}

fn flag_name(name: &str) -> String {
    name.replace('_', "-")
}

fn text_report_lines(report: &AttachmentOrphanReport) -> Vec<String> {
    let mut lines = vec![format!(
        "attachment orphan audit: {} object(s) across {} page(s), {} candidate(s), {} deleted",
        report.objects_scanned, report.pages_scanned, report.candidate_count, report.deleted_count
    )];
    for disk in &report.disks {
        if !disk.supported {
            lines.push(format!(
                "  {}: incomplete (prefix pagination unsupported)",
                disk.disk
            ));
            continue;
        }
        lines.push(format!(
            "  {}: {} object(s) across {} page(s), {} candidate(s), {} deleted",
            disk.disk,
            disk.objects_scanned,
            disk.pages_scanned,
            disk.candidate_count,
            disk.deleted_count
        ));
        for candidate in &disk.candidates {
            lines.push(format!(
                "    {} ({} bytes, age {}s)",
                candidate.path, candidate.size, candidate.age_seconds
            ));
        }
        for error in &disk.errors {
            lines.push(format!("    error: {error}"));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn escaped_like_prefix_escapes_wildcards() {
        assert_eq!(
            escaped_like_prefix("attachments/%/_"),
            "attachments/\\%/\\_%"
        );
    }

    #[test]
    fn orphan_candidates_filter_referenced_and_young_objects() {
        let now = DateTime::now();
        let objects = vec![
            StorageObject {
                path: "attachments/a.jpg".to_string(),
                size: 10,
                modified_at: now.sub_seconds(120),
            },
            StorageObject {
                path: "attachments/b.jpg".to_string(),
                size: 10,
                modified_at: now.sub_seconds(120),
            },
            StorageObject {
                path: "attachments/c.jpg".to_string(),
                size: 10,
                modified_at: now.sub_seconds(10),
            },
        ];
        let referenced = HashSet::from(["attachments/b.jpg".to_string()]);

        let candidates = orphan_candidates_from_objects("local", objects, &referenced, 60, now);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].path, "attachments/a.jpg");
        assert_eq!(candidates[0].disk, "local");
    }

    #[test]
    fn storage_page_validation_requires_strict_cursor_progress() {
        let now = DateTime::now();
        let page = vec![
            StorageObject {
                path: "attachments/b.jpg".to_string(),
                size: 1,
                modified_at: now,
            },
            StorageObject {
                path: "attachments/c.jpg".to_string(),
                size: 1,
                modified_at: now,
            },
        ];
        validate_storage_page(&page, Some("attachments/a.jpg")).unwrap();

        let error = validate_storage_page(&page, Some("attachments/b.jpg")).unwrap_err();
        assert!(error.to_string().contains("strictly increasing"));
    }
}
