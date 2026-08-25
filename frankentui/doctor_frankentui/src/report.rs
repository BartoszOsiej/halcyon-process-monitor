use std::fs;
use std::path::{Path, PathBuf};

use clap::Args;
use serde::Serialize;

use crate::error::{DoctorError, Result};
use crate::runmeta::{RunMeta, normalize_loaded_run_meta_paths};
use crate::util::{
    OutputIntegration, output_for, relative_to, tmux_attach_command_literal, write_string,
};

#[derive(Debug, Clone, Args)]
pub struct ReportArgs {
    #[arg(long = "suite-dir")]
    pub suite_dir: PathBuf,

    #[arg(long = "output-html")]
    pub output_html: Option<PathBuf>,

    #[arg(long = "output-json")]
    pub output_json: Option<PathBuf>,

    #[arg(long, default_value = "TUI Inspector Report")]
    pub title: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportSummary {
    pub title: String,
    pub suite_dir: String,
    pub generated_at: String,
    pub total_runs: usize,
    pub ok_runs: usize,
    pub failed_runs: usize,
    pub trace_ids: Vec<String>,
    pub fallback_profiles: Vec<String>,
    pub capture_error_profiles: Vec<String>,
    pub runs: Vec<RunMeta>,
}

fn find_run_meta_files(suite_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    for entry in fs::read_dir(suite_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }

        let run_meta = entry.path().join("run_meta.json");
        if run_meta.exists() {
            files.push(run_meta);
        }
    }

    files.sort();
    Ok(files)
}

fn report_run_dir(suite_dir: &Path, run: &RunMeta) -> Option<PathBuf> {
    let trimmed = run.run_dir.trim();
    if trimmed.is_empty() {
        return None;
    }

    let run_dir = PathBuf::from(trimmed);
    Some(if run_dir.is_absolute() {
        run_dir
    } else {
        suite_dir.join(run_dir)
    })
}

fn clear_run_scoped_artifacts_for_report(run: RunMeta) -> RunMeta {
    RunMeta {
        output: String::new(),
        snapshot: String::new(),
        evidence_ledger: None,
        artifact_manifest: None,
        ttyd_shim_log: None,
        ttyd_runtime_log: None,
        vhs_docker_log: None,
        tmux_session_file: None,
        tmux_pane_capture: None,
        tmux_pane_log: None,
        tmux_attach_command: run
            .tmux_session
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(tmux_attach_command_literal),
        ..run
    }
}

fn sanitize_run_for_report(suite_dir: &Path, run: RunMeta) -> RunMeta {
    if let Some(run_dir) = report_run_dir(suite_dir, &run) {
        normalize_loaded_run_meta_paths(&run_dir, &run)
    } else {
        clear_run_scoped_artifacts_for_report(run)
    }
}

fn html_escape(value: &str) -> String {
    v_htmlescape::escape_fmt(value).to_string()
}

fn resolve_existing_artifact_path(run_dir: &Path, path_value: &str) -> Option<PathBuf> {
    let trimmed = path_value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let canonical_run_dir = fs::canonicalize(run_dir).ok()?;
    let path = PathBuf::from(trimmed);
    let candidate = if path.is_absolute() {
        path
    } else {
        run_dir.join(&path)
    };
    let canonical_candidate = fs::canonicalize(candidate).ok()?;
    canonical_candidate
        .starts_with(&canonical_run_dir)
        .then_some(canonical_candidate)
}

fn push_optional_artifact_link(
    html: &mut String,
    link_base: &Path,
    run_dir: &Path,
    label: &str,
    path_value: Option<&str>,
) {
    let Some(path_value) = path_value.filter(|value| !value.is_empty()) else {
        return;
    };
    let Some(path) = resolve_existing_artifact_path(run_dir, path_value) else {
        return;
    };

    let rel = relative_to(link_base, &path).unwrap_or(path.clone());
    html.push_str(&format!(
        "<div class=\"row\"><span class=\"label\">{}</span><a href=\"{}\">{}</a></div>\n",
        html_escape(label),
        html_escape(&rel.display().to_string()),
        html_escape(&rel.display().to_string())
    ));
}

fn render_html(summary: &ReportSummary, link_base: &Path) -> String {
    let mut html = String::new();

    html.push_str(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n  <meta charset=\"utf-8\">\n  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n",
    );
    html.push_str(&format!(
        "  <title>{}</title>\n",
        html_escape(&summary.title)
    ));
    html.push_str(
        "  <style>\n    body { font-family: ui-sans-serif, -apple-system, Segoe UI, Roboto, Arial, sans-serif; margin: 24px; background: #0f1115; color: #e7ebf3; }\n    h1, h2 { margin: 0 0 12px; }\n    h3 { margin: 14px 0 8px; font-size: 14px; color: #d9e2ff; }\n    .meta { margin-bottom: 20px; color: #a8b0c5; }\n    .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(380px, 1fr)); gap: 16px; }\n    .card { border: 1px solid #2a3142; border-radius: 10px; padding: 14px; background: #171b24; }\n    .ok { border-left: 5px solid #2cb67d; }\n    .fail { border-left: 5px solid #ef4565; }\n    .row { margin: 4px 0; font-size: 13px; color: #c8d0e3; }\n    .label { color: #8a95b5; display: inline-block; min-width: 130px; }\n    .snapshot { width: 100%; border-radius: 8px; border: 1px solid #2a3142; margin-top: 8px; }\n    video { width: 100%; margin-top: 8px; border-radius: 8px; border: 1px solid #2a3142; background: #090b10; }\n    a { color: #7da6ff; text-decoration: none; }\n    a:hover { text-decoration: underline; }\n    .pill { font-size: 11px; border: 1px solid #3a4460; border-radius: 999px; padding: 2px 8px; margin-left: 8px; color: #b9c6ee; }\n    .command { margin: 6px 0 0; padding: 8px 10px; background: #0d1118; border: 1px solid #2a3142; border-radius: 8px; overflow-x: auto; color: #d9e2ff; font-size: 12px; }\n  </style>\n</head>\n<body>\n",
    );

    html.push_str(&format!("<h1>{}</h1>\n", html_escape(&summary.title)));
    html.push_str(&format!(
        "<div class=\"meta\">generated_at={} | total={} | ok={} | failed={} | traces={} | fallback_profiles={} | capture_error_profiles={}</div>\n",
        html_escape(&summary.generated_at),
        summary.total_runs,
        summary.ok_runs,
        summary.failed_runs,
        summary.trace_ids.len(),
        summary.fallback_profiles.len(),
        summary.capture_error_profiles.len(),
    ));
    html.push_str("<div class=\"grid\">\n");

    for run in &summary.runs {
        let status = run.status.as_str();
        let class_name = if status == "ok" { "ok" } else { "fail" };
        let run_path = PathBuf::from(&run.run_dir);
        let run_name = run_path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());

        let output_path = resolve_existing_artifact_path(&run_path, &run.output);
        let snapshot_path = resolve_existing_artifact_path(&run_path, &run.snapshot);

        html.push_str(&format!("<section class=\"card {}\">\n", class_name));
        html.push_str(&format!(
            "<h2>{} <span class=\"pill\">{}</span></h2>\n",
            html_escape(&run.profile),
            html_escape(status)
        ));
        html.push_str(&format!(
            "<div class=\"row\"><span class=\"label\">run</span>{}</div>\n",
            html_escape(&run_name)
        ));
        html.push_str(&format!(
            "<div class=\"row\"><span class=\"label\">duration_seconds</span>{}</div>\n",
            run.duration_seconds
                .map_or_else(|| "null".to_string(), |value| value.to_string())
        ));
        html.push_str(&format!(
            "<div class=\"row\"><span class=\"label\">seed_demo</span>{}</div>\n",
            run.seed_demo
        ));
        html.push_str(&format!(
            "<div class=\"row\"><span class=\"label\">seed_exit_code</span>{}</div>\n",
            run.seed_exit_code
                .map_or_else(|| "null".to_string(), |value| value.to_string())
        ));
        html.push_str(&format!(
            "<div class=\"row\"><span class=\"label\">snapshot_status</span>{}</div>\n",
            html_escape(run.snapshot_status.as_deref().unwrap_or("unknown"))
        ));
        html.push_str(&format!(
            "<div class=\"row\"><span class=\"label\">vhs_exit_code</span>{}</div>\n",
            run.vhs_exit_code
                .map_or_else(|| "null".to_string(), |value| value.to_string())
        ));
        if let Some(trace_id) = run.trace_id.as_deref().filter(|value| !value.is_empty()) {
            html.push_str(&format!(
                "<div class=\"row\"><span class=\"label\">trace_id</span>{}</div>\n",
                html_escape(trace_id)
            ));
        }
        if let Some(fallback_reason) = run
            .fallback_reason
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            html.push_str(&format!(
                "<div class=\"row\"><span class=\"label\">fallback_reason</span>{}</div>\n",
                html_escape(fallback_reason)
            ));
        }
        if let Some(capture_error_reason) = run
            .capture_error_reason
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            html.push_str(&format!(
                "<div class=\"row\"><span class=\"label\">capture_error_reason</span>{}</div>\n",
                html_escape(capture_error_reason)
            ));
        }
        if let Some(tmux_session) = run
            .tmux_session
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            html.push_str(&format!(
                "<div class=\"row\"><span class=\"label\">tmux_session</span>{}</div>\n",
                html_escape(tmux_session)
            ));
        }
        if let Some(tmux_attach_command) = run
            .tmux_session
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(tmux_attach_command_literal)
        {
            html.push_str(&format!(
                "<div class=\"row\"><span class=\"label\">tmux_attach_command</span><code>{}</code></div>\n",
                html_escape(&tmux_attach_command)
            ));
        }
        push_optional_artifact_link(
            &mut html,
            link_base,
            &run_path,
            "evidence_ledger",
            run.evidence_ledger.as_deref(),
        );
        push_optional_artifact_link(
            &mut html,
            link_base,
            &run_path,
            "artifact_manifest",
            run.artifact_manifest.as_deref(),
        );
        push_optional_artifact_link(
            &mut html,
            link_base,
            &run_path,
            "ttyd_shim_log",
            run.ttyd_shim_log.as_deref(),
        );
        push_optional_artifact_link(
            &mut html,
            link_base,
            &run_path,
            "ttyd_runtime_log",
            run.ttyd_runtime_log.as_deref(),
        );
        push_optional_artifact_link(
            &mut html,
            link_base,
            &run_path,
            "tmux_session_file",
            run.tmux_session_file.as_deref(),
        );
        push_optional_artifact_link(
            &mut html,
            link_base,
            &run_path,
            "tmux_pane_capture",
            run.tmux_pane_capture.as_deref(),
        );
        push_optional_artifact_link(
            &mut html,
            link_base,
            &run_path,
            "tmux_pane_log",
            run.tmux_pane_log.as_deref(),
        );
        push_optional_artifact_link(
            &mut html,
            link_base,
            &run_path,
            "vhs_docker_log",
            run.vhs_docker_log.as_deref(),
        );

        let replay_commands = run.replay_commands();
        if !replay_commands.is_empty() {
            html.push_str("<h3>Replay Commands</h3>\n");
            for replay in replay_commands {
                html.push_str(&format!(
                    "<div class=\"row\"><span class=\"label\">{}</span>{}</div>\n",
                    html_escape(&replay.key),
                    html_escape(&replay.purpose)
                ));
                html.push_str(&format!(
                    "<pre class=\"command\">{}</pre>\n",
                    html_escape(&replay.command)
                ));
            }
        }

        if let Some(output_path) = output_path {
            let output_rel = relative_to(link_base, &output_path).unwrap_or(output_path.clone());
            let output_rel_str = output_rel.display().to_string();
            html.push_str(&format!(
                "<div class=\"row\"><a href=\"{}\">video file</a></div>\n",
                html_escape(&output_rel_str)
            ));
            html.push_str(&format!(
                "<video controls muted preload=\"metadata\" src=\"{}\"></video>\n",
                html_escape(&output_rel_str)
            ));
        }

        if let Some(snapshot_path) = snapshot_path {
            let snapshot_rel =
                relative_to(link_base, &snapshot_path).unwrap_or(snapshot_path.clone());
            let snapshot_rel_str = snapshot_rel.display().to_string();
            html.push_str(&format!(
                "<div class=\"row\"><a href=\"{}\">snapshot file</a></div>\n",
                html_escape(&snapshot_rel_str)
            ));
            html.push_str(&format!(
                "<img class=\"snapshot\" alt=\"snapshot {}\" src=\"{}\">\n",
                html_escape(&run.profile),
                html_escape(&snapshot_rel_str)
            ));
        }

        html.push_str("</section>\n");
    }

    html.push_str("</div>\n</body>\n</html>\n");
    html
}

fn build_report_machine_summary(
    integration: &OutputIntegration,
    summary: &ReportSummary,
    output_json: &Path,
    output_html: &Path,
) -> serde_json::Value {
    serde_json::json!({
        "command": "report",
        "status": "ok",
        "report_json": output_json.display().to_string(),
        "report_html": output_html.display().to_string(),
        "suite_dir": summary.suite_dir,
        "trace_ids": summary.trace_ids,
        "fallback_profiles": summary.fallback_profiles,
        "capture_error_profiles": summary.capture_error_profiles,
        "integration": integration,
    })
}

pub fn run_report(args: ReportArgs) -> Result<()> {
    let integration = OutputIntegration::detect();
    run_report_with_integration(args, &integration)
}

pub(crate) fn run_report_with_runs(
    args: ReportArgs,
    runs: Vec<RunMeta>,
    integration: &OutputIntegration,
) -> Result<()> {
    let ui = output_for(integration);

    if !args.suite_dir.exists() {
        return Err(DoctorError::MissingPath {
            path: args.suite_dir,
        });
    }

    if runs.is_empty() {
        return Err(DoctorError::invalid(format!(
            "No run_meta.json files found under {}",
            args.suite_dir.display()
        )));
    }

    let output_html = args
        .output_html
        .unwrap_or_else(|| args.suite_dir.join("index.html"));
    let output_json = args
        .output_json
        .unwrap_or_else(|| args.suite_dir.join("report.json"));

    let runs = runs
        .into_iter()
        .map(|run| sanitize_run_for_report(&args.suite_dir, run))
        .collect::<Vec<_>>();

    let ok_runs = runs.iter().filter(|run| run.status == "ok").count();
    let failed_runs = runs.len().saturating_sub(ok_runs);
    let trace_ids = runs
        .iter()
        .filter_map(|run| run.trace_id.as_ref())
        .filter(|value| !value.is_empty())
        .cloned()
        .collect::<Vec<_>>();
    let fallback_profiles = runs
        .iter()
        .filter(|run| {
            run.fallback_reason
                .as_ref()
                .is_some_and(|value| !value.is_empty())
        })
        .map(|run| run.profile.clone())
        .collect::<Vec<_>>();
    let capture_error_profiles = runs
        .iter()
        .filter(|run| {
            run.capture_error_reason
                .as_ref()
                .is_some_and(|value| !value.is_empty())
        })
        .map(|run| run.profile.clone())
        .collect::<Vec<_>>();

    let summary = ReportSummary {
        title: args.title,
        suite_dir: args.suite_dir.display().to_string(),
        generated_at: crate::util::now_utc_iso(),
        total_runs: runs.len(),
        ok_runs,
        failed_runs,
        trace_ids,
        fallback_profiles,
        capture_error_profiles,
        runs,
    };

    let json_content = serde_json::to_string_pretty(&summary)?;
    write_string(&output_json, &json_content)?;

    let raw_link_base = output_html.parent().unwrap_or(args.suite_dir.as_path());
    // Canonicalize so the relative-link calculation matches canonicalized
    // artifact paths produced by `resolve_existing_artifact_path` (otherwise
    // platform-specific symlinks like macOS's `/var/folders -> /private/var/folders`
    // produce absurd `../../../../private/var/folders/...` link prefixes).
    if let Some(parent) = output_html.parent() {
        crate::util::ensure_dir(parent)?;
    }
    let link_base_buf =
        fs::canonicalize(raw_link_base).unwrap_or_else(|_| raw_link_base.to_path_buf());
    let html = render_html(&summary, link_base_buf.as_path());
    write_string(&output_html, &html)?;

    ui.success(&format!("report JSON: {}", output_json.display()));
    ui.success(&format!("report HTML: {}", output_html.display()));

    if integration.should_emit_json() {
        println!(
            "{}",
            build_report_machine_summary(integration, &summary, &output_json, &output_html)
        );
    }

    Ok(())
}

fn run_report_with_integration(args: ReportArgs, integration: &OutputIntegration) -> Result<()> {
    if !args.suite_dir.exists() {
        return Err(DoctorError::MissingPath {
            path: args.suite_dir,
        });
    }

    let meta_files = find_run_meta_files(&args.suite_dir)?;
    if meta_files.is_empty() {
        return Err(DoctorError::invalid(format!(
            "No run_meta.json files found under {}",
            args.suite_dir.display()
        )));
    }
    let runs = meta_files
        .iter()
        .map(|path| {
            let run = RunMeta::from_path(path)?;
            let run_dir = path.parent().unwrap_or(args.suite_dir.as_path());
            Ok(normalize_loaded_run_meta_paths(run_dir, &run))
        })
        .collect::<Result<Vec<_>>>()?;

    run_report_with_runs(args, runs, integration)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::sync::{LazyLock, Mutex};

    use tempfile::tempdir;

    use crate::error::DoctorError;
    use crate::runmeta::RunMeta;
    use crate::util::OutputIntegration;

    use super::{
        ReportArgs, ReportSummary, build_report_machine_summary, find_run_meta_files, run_report,
        run_report_with_integration, run_report_with_runs,
    };

    static CWD_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    #[test]
    fn report_generation_writes_outputs() {
        let temp = tempdir().expect("tempdir");
        let suite_dir = temp.path().join("suite");
        let run_dir = suite_dir.join("run_01");
        std::fs::create_dir_all(&run_dir).expect("mkdir");

        let output_path = run_dir.join("capture.mp4");
        let snapshot_path = run_dir.join("snapshot.png");
        fs::write(&output_path, b"dummy").expect("write dummy video");
        fs::write(&snapshot_path, b"dummy").expect("write dummy snapshot");

        let run_meta = RunMeta {
            status: "ok".to_string(),
            started_at: "2026-02-17T00:00:00Z".to_string(),
            profile: "analytics-empty".to_string(),
            output: output_path.display().to_string(),
            snapshot: snapshot_path.display().to_string(),
            run_dir: run_dir.display().to_string(),
            ..RunMeta::default()
        };

        run_meta
            .write_to_path(&run_dir.join("run_meta.json"))
            .expect("write run_meta");

        let args = ReportArgs {
            suite_dir: suite_dir.clone(),
            output_html: None,
            output_json: None,
            title: "Report".to_string(),
        };

        run_report(args).expect("run report");

        assert!(suite_dir.join("index.html").exists());
        assert!(suite_dir.join("report.json").exists());
    }

    #[test]
    fn find_run_meta_files_sorts_and_skips_non_directories() {
        let temp = tempdir().expect("tempdir");
        let suite_dir = temp.path().join("suite");
        let a_run = suite_dir.join("a_run");
        let b_run = suite_dir.join("b_run");
        let c_run = suite_dir.join("c_run");
        fs::create_dir_all(&a_run).expect("mkdir a_run");
        fs::create_dir_all(&b_run).expect("mkdir b_run");
        fs::create_dir_all(&c_run).expect("mkdir c_run");
        fs::write(suite_dir.join("not_a_dir"), b"ignore").expect("write file");

        RunMeta {
            status: "ok".to_string(),
            started_at: "2026-02-17T00:00:00Z".to_string(),
            profile: "alpha".to_string(),
            output: a_run.join("capture.mp4").display().to_string(),
            run_dir: a_run.display().to_string(),
            ..RunMeta::default()
        }
        .write_to_path(&a_run.join("run_meta.json"))
        .expect("write a run meta");

        RunMeta {
            status: "failed".to_string(),
            started_at: "2026-02-17T00:00:01Z".to_string(),
            profile: "beta".to_string(),
            output: b_run.join("capture.mp4").display().to_string(),
            run_dir: b_run.display().to_string(),
            ..RunMeta::default()
        }
        .write_to_path(&b_run.join("run_meta.json"))
        .expect("write b run meta");

        let files = find_run_meta_files(&suite_dir).expect("find run meta files");
        let display = files
            .iter()
            .map(|path| {
                path.strip_prefix(&suite_dir)
                    .unwrap_or(path)
                    .display()
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(display, vec!["a_run/run_meta.json", "b_run/run_meta.json"]);
    }

    #[test]
    fn run_report_fails_when_suite_dir_missing() {
        let temp = tempdir().expect("tempdir");
        let missing_suite_dir = temp.path().join("does_not_exist");

        let error = run_report(ReportArgs {
            suite_dir: missing_suite_dir.clone(),
            output_html: None,
            output_json: None,
            title: "Report".to_string(),
        })
        .expect_err("missing suite dir should fail");

        assert!(matches!(&error, DoctorError::MissingPath { .. }));
        if let DoctorError::MissingPath { path } = error {
            assert_eq!(path, missing_suite_dir);
        }
    }

    #[test]
    fn run_report_fails_when_no_run_meta_files_present() {
        let temp = tempdir().expect("tempdir");
        let suite_dir = temp.path().join("suite");
        fs::create_dir_all(&suite_dir).expect("mkdir suite");

        let error = run_report(ReportArgs {
            suite_dir: suite_dir.clone(),
            output_html: None,
            output_json: None,
            title: "Report".to_string(),
        })
        .expect_err("suite dir without run meta files should fail");
        assert!(
            error
                .to_string()
                .contains("No run_meta.json files found under"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn run_report_with_runs_uses_preloaded_runmeta_without_scanning_suite_dir() {
        let temp = tempdir().expect("tempdir");
        let suite_dir = temp.path().join("suite");
        let run_dir = suite_dir.join("run_01");
        fs::create_dir_all(&run_dir).expect("mkdir");

        let output_path = run_dir.join("capture.mp4");
        let snapshot_path = run_dir.join("snapshot.png");
        fs::write(&output_path, b"dummy").expect("write dummy video");
        fs::write(&snapshot_path, b"dummy").expect("write dummy snapshot");

        let run_meta = RunMeta {
            status: "ok".to_string(),
            started_at: "2026-02-17T00:00:00Z".to_string(),
            profile: "analytics-empty".to_string(),
            output: output_path.display().to_string(),
            snapshot: snapshot_path.display().to_string(),
            run_dir: run_dir.display().to_string(),
            ..RunMeta::default()
        };

        let integration = OutputIntegration {
            fastapi_mode: "plain".to_string(),
            fastapi_agent: true,
            fastapi_ci: false,
            fastapi_tty: false,
            sqlmodel_mode: "plain".to_string(),
            sqlmodel_agent: false,
        };

        run_report_with_runs(
            ReportArgs {
                suite_dir: suite_dir.clone(),
                output_html: None,
                output_json: None,
                title: "Preloaded Report".to_string(),
            },
            vec![run_meta],
            &integration,
        )
        .expect("run report with preloaded runs");

        assert!(suite_dir.join("index.html").exists());
        assert!(suite_dir.join("report.json").exists());
    }

    #[test]
    fn run_report_respects_output_path_overrides() {
        let temp = tempdir().expect("tempdir");
        let suite_dir = temp.path().join("suite");
        let run_dir = suite_dir.join("run_01");
        fs::create_dir_all(&run_dir).expect("mkdir");

        let run_meta = RunMeta {
            status: "ok".to_string(),
            started_at: "2026-02-17T00:00:00Z".to_string(),
            profile: "analytics-empty".to_string(),
            output: run_dir.join("capture.mp4").display().to_string(),
            run_dir: run_dir.display().to_string(),
            ..RunMeta::default()
        };
        run_meta
            .write_to_path(&run_dir.join("run_meta.json"))
            .expect("write run meta");

        let output_html = temp.path().join("custom.html");
        let output_json = temp.path().join("custom.json");

        run_report(ReportArgs {
            suite_dir: suite_dir.clone(),
            output_html: Some(output_html.clone()),
            output_json: Some(output_json.clone()),
            title: "Custom Report".to_string(),
        })
        .expect("run report");

        assert!(output_html.exists());
        assert!(output_json.exists());

        let parsed: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&output_json).expect("read json"))
                .expect("parse json");
        assert_eq!(parsed["title"], "Custom Report");
        assert_eq!(parsed["suite_dir"], suite_dir.display().to_string());
        assert_eq!(parsed["total_runs"], 1);
        assert_eq!(parsed["ok_runs"], 1);
        assert_eq!(parsed["failed_runs"], 0);
    }

    #[test]
    fn run_report_uses_output_html_parent_for_relative_links() {
        let temp = tempdir().expect("tempdir");
        let suite_dir = temp.path().join("suite");
        let run_dir = suite_dir.join("run_01");
        fs::create_dir_all(&run_dir).expect("mkdir");

        let output_path = run_dir.join("capture.mp4");
        let snapshot_path = run_dir.join("snapshot.png");
        fs::write(&output_path, b"dummy video").expect("write dummy video");
        fs::write(&snapshot_path, b"dummy snapshot").expect("write dummy snapshot");

        let run_meta = RunMeta {
            status: "ok".to_string(),
            started_at: "2026-02-17T00:00:00Z".to_string(),
            profile: "analytics-empty".to_string(),
            output: output_path.display().to_string(),
            snapshot: snapshot_path.display().to_string(),
            run_dir: run_dir.display().to_string(),
            ..RunMeta::default()
        };
        run_meta
            .write_to_path(&run_dir.join("run_meta.json"))
            .expect("write run meta");

        let report_dir = temp.path().join("reports").join("nested");
        let output_html = report_dir.join("custom.html");
        let output_json = report_dir.join("custom.json");

        run_report(ReportArgs {
            suite_dir: suite_dir.clone(),
            output_html: Some(output_html.clone()),
            output_json: Some(output_json),
            title: "Custom Report".to_string(),
        })
        .expect("run report");

        let html = fs::read_to_string(&output_html).expect("read html");
        assert!(
            html.contains(r#"href="..&#x2f;..&#x2f;suite&#x2f;run_01&#x2f;capture.mp4""#),
            "expected video link relative to output html parent, got: {html}"
        );
        assert!(
            html.contains(r#"src="..&#x2f;..&#x2f;suite&#x2f;run_01&#x2f;snapshot.png""#),
            "expected snapshot link relative to output html parent, got: {html}"
        );
    }

    #[test]
    fn run_report_escapes_html_title() {
        let temp = tempdir().expect("tempdir");
        let suite_dir = temp.path().join("suite");
        let run_dir = suite_dir.join("run_01");
        fs::create_dir_all(&run_dir).expect("mkdir");

        let video_path = run_dir.join("capture.mp4");
        fs::write(&video_path, b"not-a-real-mp4").expect("write dummy video");

        let snapshot_path = run_dir.join("snapshot.png");
        fs::write(&snapshot_path, b"not-a-real-png").expect("write dummy snapshot");

        let run_meta = RunMeta {
            status: "ok".to_string(),
            started_at: "2026-02-17T00:00:00Z".to_string(),
            profile: "analytics-empty".to_string(),
            output: video_path.display().to_string(),
            snapshot: snapshot_path.display().to_string(),
            run_dir: run_dir.display().to_string(),
            ..RunMeta::default()
        };
        run_meta
            .write_to_path(&run_dir.join("run_meta.json"))
            .expect("write run meta");

        let title = "Report <script>alert(1)</script>";
        run_report(ReportArgs {
            suite_dir: suite_dir.clone(),
            output_html: None,
            output_json: None,
            title: title.to_string(),
        })
        .expect("run report");

        let html = fs::read_to_string(suite_dir.join("index.html")).expect("read html");
        assert!(
            html.contains("Report &lt;script&gt;alert(1)&lt;&#x2f;script&gt;"),
            "expected escaped title, got: {html}"
        );
        assert!(!html.contains("<script>alert(1)</script>"));

        // Ensure the file-exists conditionals emit links when the artifacts exist.
        assert!(html.contains("video file"));
        assert!(html.contains("snapshot file"));
    }

    #[test]
    fn run_report_html_surfaces_evidence_links_and_fallback_metadata() {
        let temp = tempdir().expect("tempdir");
        let suite_dir = temp.path().join("suite");
        let run_dir = suite_dir.join("run_01");
        fs::create_dir_all(&run_dir).expect("mkdir");

        let evidence_ledger = run_dir.join("evidence_ledger.jsonl");
        let artifact_manifest = run_dir.join("run_artifact_manifest.json");
        let capture_tape = run_dir.join("capture.tape");
        let ttyd_runtime_log = run_dir.join("ttyd-runtime.log");
        let tmux_session_file = run_dir.join("tmux_session.txt");
        let tmux_pane_capture = run_dir.join("tmux_pane.txt");
        let tmux_pane_log = run_dir.join("tmux_pane.log");
        fs::write(&evidence_ledger, b"{}\n").expect("write ledger");
        fs::write(&artifact_manifest, b"{}\n").expect("write artifact manifest");
        fs::write(&capture_tape, b"set FontSize 14\n").expect("write capture tape");
        fs::write(&ttyd_runtime_log, b"log").expect("write runtime log");
        fs::write(&tmux_session_file, b"session_name=tmux-demo\n").expect("write tmux session");
        fs::write(&tmux_pane_capture, b"pane snapshot").expect("write tmux pane capture");
        fs::write(&tmux_pane_log, b"pane log").expect("write tmux pane log");

        RunMeta {
            status: "degraded".to_string(),
            started_at: "2026-02-17T00:00:00Z".to_string(),
            profile: "analytics-empty".to_string(),
            run_dir: run_dir.display().to_string(),
            trace_id: Some("trace-123".to_string()),
            fallback_reason: Some("capture timed out".to_string()),
            capture_error_reason: Some("ffmpeg missing".to_string()),
            tmux_session: Some("tmux demo".to_string()),
            tmux_attach_command: Some("echo pwned".to_string()),
            tmux_session_file: Some(tmux_session_file.display().to_string()),
            tmux_pane_capture: Some(tmux_pane_capture.display().to_string()),
            tmux_pane_log: Some(tmux_pane_log.display().to_string()),
            evidence_ledger: Some(evidence_ledger.display().to_string()),
            artifact_manifest: Some(artifact_manifest.display().to_string()),
            ttyd_runtime_log: Some(ttyd_runtime_log.display().to_string()),
            ..RunMeta::default()
        }
        .write_to_path(&run_dir.join("run_meta.json"))
        .expect("write run meta");

        run_report(ReportArgs {
            suite_dir: suite_dir.clone(),
            output_html: None,
            output_json: None,
            title: "Artifact Report".to_string(),
        })
        .expect("run report");

        let html = fs::read_to_string(suite_dir.join("index.html")).expect("read html");
        assert!(html.contains("trace-123"));
        assert!(html.contains("capture timed out"));
        assert!(html.contains("ffmpeg missing"));
        assert!(html.contains("tmux demo"));
        assert!(html.contains("tmux attach-session -t &#x27;tmux demo&#x27;"));
        assert!(!html.contains("echo pwned"));
        assert!(html.contains("tmux_session_file"));
        assert!(html.contains("tmux_session.txt"));
        assert!(html.contains("tmux_pane_capture"));
        assert!(html.contains("tmux_pane.txt"));
        assert!(html.contains("tmux_pane_log"));
        assert!(html.contains("tmux_pane.log"));
        assert!(html.contains("evidence_ledger"));
        assert!(html.contains("evidence_ledger.jsonl"));
        assert!(html.contains("artifact_manifest"));
        assert!(html.contains("run_artifact_manifest.json"));
        assert!(html.contains("ttyd_runtime_log"));
        assert!(html.contains("ttyd-runtime.log"));
        assert!(html.contains("Replay Commands"));
        assert!(html.contains("inspect_run_meta"));
        assert!(html.contains("tail -n 80"));
        assert!(html.contains("replay_capture_tape"));
        assert!(html.contains("capture.tape"));

        let report_json: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(suite_dir.join("report.json")).expect("read report json"),
        )
        .expect("parse report json");
        assert_eq!(report_json["trace_ids"][0], "trace-123");
        assert_eq!(report_json["fallback_profiles"][0], "analytics-empty");
        assert_eq!(report_json["capture_error_profiles"][0], "analytics-empty");
        assert_eq!(
            report_json["runs"][0]["tmux_attach_command"],
            "tmux attach-session -t 'tmux demo'"
        );
    }

    #[test]
    fn run_report_with_runs_sanitizes_preloaded_runmeta_before_writing_outputs() {
        let temp = tempdir().expect("tempdir");
        let suite_dir = temp.path().join("suite");
        let run_dir = suite_dir.join("run_01");
        fs::create_dir_all(&run_dir).expect("mkdir");
        fs::write(run_dir.join("capture.mp4"), b"dummy").expect("write dummy video");

        let integration = OutputIntegration {
            fastapi_mode: "plain".to_string(),
            fastapi_agent: true,
            fastapi_ci: false,
            fastapi_tty: false,
            sqlmodel_mode: "plain".to_string(),
            sqlmodel_agent: false,
        };

        run_report_with_runs(
            ReportArgs {
                suite_dir: suite_dir.clone(),
                output_html: None,
                output_json: None,
                title: "Preloaded Report".to_string(),
            },
            vec![RunMeta {
                status: "ok".to_string(),
                started_at: "2026-02-17T00:00:00Z".to_string(),
                profile: "analytics-empty".to_string(),
                output: "capture.mp4".to_string(),
                run_dir: run_dir.display().to_string(),
                tmux_session: Some("tmux demo".to_string()),
                tmux_attach_command: Some("echo pwned".to_string()),
                evidence_ledger: Some("../outside.log".to_string()),
                ..RunMeta::default()
            }],
            &integration,
        )
        .expect("run report with preloaded runs");

        let html = fs::read_to_string(suite_dir.join("index.html")).expect("read html");
        assert!(html.contains("tmux attach-session -t &#x27;tmux demo&#x27;"));
        assert!(!html.contains("echo pwned"));
        assert!(!html.contains("../outside.log"));

        let report_json: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(suite_dir.join("report.json")).expect("read report json"),
        )
        .expect("parse report json");
        assert_eq!(report_json["runs"][0]["output"], "capture.mp4");
        assert_eq!(
            report_json["runs"][0]["tmux_attach_command"],
            "tmux attach-session -t 'tmux demo'"
        );
        assert!(report_json["runs"][0]["evidence_ledger"].is_null());
    }

    #[test]
    fn run_report_with_runs_clears_run_scoped_artifacts_when_run_dir_is_blank() {
        let _cwd_lock = CWD_TEST_LOCK.lock().expect("cwd lock poisoned");
        let temp = tempdir().expect("tempdir");
        let suite_dir = temp.path().join("suite");
        fs::create_dir_all(&suite_dir).expect("mkdir suite dir");

        let cwd_guard = tempdir().expect("cwd tempdir");
        let original_cwd = std::env::current_dir().expect("capture cwd");
        fs::write(cwd_guard.path().join("capture.mp4"), b"cwd video").expect("write cwd video");
        fs::write(cwd_guard.path().join("snapshot.png"), b"cwd snapshot")
            .expect("write cwd snapshot");
        fs::write(cwd_guard.path().join("run_artifact_manifest.json"), b"{}")
            .expect("write cwd manifest");
        fs::write(cwd_guard.path().join("evidence_ledger.jsonl"), b"{}\n")
            .expect("write cwd ledger");
        std::env::set_current_dir(cwd_guard.path()).expect("set cwd");

        let integration = OutputIntegration {
            fastapi_mode: "plain".to_string(),
            fastapi_agent: true,
            fastapi_ci: false,
            fastapi_tty: false,
            sqlmodel_mode: "plain".to_string(),
            sqlmodel_agent: false,
        };

        let report_result = run_report_with_runs(
            ReportArgs {
                suite_dir: suite_dir.clone(),
                output_html: None,
                output_json: None,
                title: "Blank Run Dir Report".to_string(),
            },
            vec![RunMeta {
                status: "ok".to_string(),
                started_at: "2026-02-17T00:00:00Z".to_string(),
                profile: "analytics-empty".to_string(),
                output: "capture.mp4".to_string(),
                snapshot: "snapshot.png".to_string(),
                run_dir: String::new(),
                tmux_session: Some("tmux demo".to_string()),
                tmux_attach_command: Some("echo pwned".to_string()),
                evidence_ledger: Some("evidence_ledger.jsonl".to_string()),
                artifact_manifest: Some("run_artifact_manifest.json".to_string()),
                ..RunMeta::default()
            }],
            &integration,
        );

        std::env::set_current_dir(&original_cwd).expect("restore cwd");
        report_result.expect("run report with blank run dir");

        let html = fs::read_to_string(suite_dir.join("index.html")).expect("read html");
        assert!(
            !html.contains("capture.mp4"),
            "report should not link cwd video: {html}"
        );
        assert!(
            !html.contains("snapshot.png"),
            "report should not link cwd snapshot: {html}"
        );
        assert!(
            !html.contains("evidence_ledger.jsonl"),
            "report should not link cwd ledger: {html}"
        );
        assert!(
            !html.contains("run_artifact_manifest.json"),
            "report should not link cwd manifest: {html}"
        );
        assert!(html.contains("tmux attach-session -t &#x27;tmux demo&#x27;"));
        assert!(!html.contains("echo pwned"));

        let report_json: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(suite_dir.join("report.json")).expect("read report json"),
        )
        .expect("parse report json");
        let run = &report_json["runs"][0];
        assert_eq!(run["run_dir"], "");
        assert_eq!(run["output"], "");
        assert_eq!(run["snapshot"], "");
        assert!(run["evidence_ledger"].is_null());
        assert!(run["artifact_manifest"].is_null());
        assert_eq!(
            run["tmux_attach_command"],
            "tmux attach-session -t 'tmux demo'"
        );
    }

    #[test]
    fn run_report_resolves_run_local_relative_artifact_paths() {
        let temp = tempdir().expect("tempdir");
        let suite_dir = temp.path().join("suite");
        let run_dir = suite_dir.join("run_01");
        fs::create_dir_all(&run_dir).expect("mkdir");

        fs::write(run_dir.join("capture.mp4"), b"dummy video").expect("write video");
        fs::write(run_dir.join("snapshot.png"), b"dummy snapshot").expect("write snapshot");
        fs::write(run_dir.join("evidence_ledger.jsonl"), b"{}\n").expect("write ledger");
        fs::write(run_dir.join("ttyd-runtime.log"), b"log").expect("write runtime log");

        RunMeta {
            status: "ok".to_string(),
            started_at: "2026-02-17T00:00:00Z".to_string(),
            profile: "analytics-empty".to_string(),
            output: "capture.mp4".to_string(),
            snapshot: "snapshot.png".to_string(),
            run_dir: run_dir.display().to_string(),
            evidence_ledger: Some("evidence_ledger.jsonl".to_string()),
            ttyd_runtime_log: Some("ttyd-runtime.log".to_string()),
            ..RunMeta::default()
        }
        .write_to_path(&run_dir.join("run_meta.json"))
        .expect("write run meta");

        run_report(ReportArgs {
            suite_dir: suite_dir.clone(),
            output_html: None,
            output_json: None,
            title: "Relative Artifact Report".to_string(),
        })
        .expect("run report");

        let html = fs::read_to_string(suite_dir.join("index.html")).expect("read html");
        assert!(html.contains("video file"));
        assert!(html.contains("snapshot file"));
        assert!(html.contains("evidence_ledger.jsonl"));
        assert!(html.contains("ttyd-runtime.log"));
    }

    #[test]
    fn run_report_prefers_run_dir_for_relative_artifact_paths_over_cwd_collisions() {
        let _cwd_lock = CWD_TEST_LOCK.lock().expect("cwd lock poisoned");
        let temp = tempdir().expect("tempdir");
        let suite_dir = temp.path().join("suite");
        let run_dir = suite_dir.join("run_01");
        fs::create_dir_all(&run_dir).expect("mkdir");

        fs::write(run_dir.join("capture.mp4"), b"run-local video").expect("write run video");

        RunMeta {
            status: "ok".to_string(),
            started_at: "2026-02-17T00:00:00Z".to_string(),
            profile: "analytics-empty".to_string(),
            output: "capture.mp4".to_string(),
            run_dir: run_dir.display().to_string(),
            ..RunMeta::default()
        }
        .write_to_path(&run_dir.join("run_meta.json"))
        .expect("write run meta");

        let cwd_guard = tempdir().expect("cwd tempdir");
        let original_cwd = std::env::current_dir().expect("capture cwd");
        fs::write(cwd_guard.path().join("capture.mp4"), b"cwd collision").expect("write cwd file");
        std::env::set_current_dir(cwd_guard.path()).expect("set cwd");

        let report_result = run_report(ReportArgs {
            suite_dir: suite_dir.clone(),
            output_html: None,
            output_json: None,
            title: "CWD Collision Report".to_string(),
        });

        std::env::set_current_dir(&original_cwd).expect("restore cwd");
        report_result.expect("run report");

        let html = fs::read_to_string(suite_dir.join("index.html")).expect("read html");
        assert!(
            html.contains(r#"href="run_01&#x2f;capture.mp4""#),
            "expected run-local relative path, got: {html}"
        );
        assert!(
            !html.contains("../capture.mp4"),
            "report should not resolve against cwd collision: {html}"
        );
    }

    #[test]
    fn run_report_does_not_resolve_missing_relative_artifacts_against_cwd() {
        let _cwd_lock = CWD_TEST_LOCK.lock().expect("cwd lock poisoned");
        let temp = tempdir().expect("tempdir");
        let suite_dir = temp.path().join("suite");
        let run_dir = suite_dir.join("run_01");
        fs::create_dir_all(&run_dir).expect("mkdir");

        RunMeta {
            status: "ok".to_string(),
            started_at: "2026-02-17T00:00:00Z".to_string(),
            profile: "analytics-empty".to_string(),
            output: "capture.mp4".to_string(),
            run_dir: run_dir.display().to_string(),
            ..RunMeta::default()
        }
        .write_to_path(&run_dir.join("run_meta.json"))
        .expect("write run meta");

        let cwd_guard = tempdir().expect("cwd tempdir");
        let original_cwd = std::env::current_dir().expect("capture cwd");
        fs::write(cwd_guard.path().join("capture.mp4"), b"cwd collision").expect("write cwd file");
        std::env::set_current_dir(cwd_guard.path()).expect("set cwd");

        let report_result = run_report(ReportArgs {
            suite_dir: suite_dir.clone(),
            output_html: None,
            output_json: None,
            title: "Missing Relative Artifact Report".to_string(),
        });

        std::env::set_current_dir(&original_cwd).expect("restore cwd");
        report_result.expect("run report");

        let html = fs::read_to_string(suite_dir.join("index.html")).expect("read html");
        assert!(
            !html.contains("video file"),
            "report should not resolve missing relative artifacts against cwd: {html}"
        );
        assert!(
            !html.contains("capture.mp4"),
            "report should not link cwd-relative artifacts: {html}"
        );
    }

    #[test]
    fn run_report_rejects_relative_artifact_paths_that_escape_run_dir() {
        let temp = tempdir().expect("tempdir");
        let suite_dir = temp.path().join("suite");
        let run_dir = suite_dir.join("run_01");
        fs::create_dir_all(&run_dir).expect("mkdir");

        let outside_video = suite_dir.join("outside.mp4");
        fs::write(&outside_video, b"outside video").expect("write outside video");

        RunMeta {
            status: "ok".to_string(),
            started_at: "2026-02-17T00:00:00Z".to_string(),
            profile: "analytics-empty".to_string(),
            output: "../outside.mp4".to_string(),
            run_dir: run_dir.display().to_string(),
            ..RunMeta::default()
        }
        .write_to_path(&run_dir.join("run_meta.json"))
        .expect("write run meta");

        run_report(ReportArgs {
            suite_dir: suite_dir.clone(),
            output_html: None,
            output_json: None,
            title: "Escaping Relative Artifact Report".to_string(),
        })
        .expect("run report");

        let html = fs::read_to_string(suite_dir.join("index.html")).expect("read html");
        assert!(
            !html.contains("video file"),
            "report should not link artifacts outside the run directory: {html}"
        );
        assert!(
            !html.contains("outside.mp4"),
            "report should reject escaping relative artifact paths: {html}"
        );
    }

    #[test]
    fn run_report_rejects_absolute_artifact_paths_outside_run_dir() {
        let temp = tempdir().expect("tempdir");
        let suite_dir = temp.path().join("suite");
        let run_dir = suite_dir.join("run_01");
        fs::create_dir_all(&run_dir).expect("mkdir");

        let outside_video = suite_dir.join("outside.mp4");
        fs::write(&outside_video, b"outside video").expect("write outside video");

        RunMeta {
            status: "ok".to_string(),
            started_at: "2026-02-17T00:00:00Z".to_string(),
            profile: "analytics-empty".to_string(),
            output: outside_video.display().to_string(),
            run_dir: run_dir.display().to_string(),
            ..RunMeta::default()
        }
        .write_to_path(&run_dir.join("run_meta.json"))
        .expect("write run meta");

        run_report(ReportArgs {
            suite_dir: suite_dir.clone(),
            output_html: None,
            output_json: None,
            title: "Escaping Absolute Artifact Report".to_string(),
        })
        .expect("run report");

        let html = fs::read_to_string(suite_dir.join("index.html")).expect("read html");
        assert!(
            !html.contains("video file"),
            "report should not link absolute artifacts outside the run directory: {html}"
        );
        assert!(
            !html.contains("outside.mp4"),
            "report should reject escaping absolute artifact paths: {html}"
        );
    }

    #[test]
    fn run_report_uses_discovered_run_dir_over_runmeta_run_dir() {
        let temp = tempdir().expect("tempdir");
        let suite_dir = temp.path().join("suite");
        let run_dir = suite_dir.join("run_01");
        let forged_run_dir = temp.path().join("forged_run");
        fs::create_dir_all(&run_dir).expect("mkdir run dir");
        fs::create_dir_all(&forged_run_dir).expect("mkdir forged run dir");

        fs::write(run_dir.join("capture.mp4"), b"run-local video").expect("write run-local video");
        fs::write(forged_run_dir.join("capture.mp4"), b"forged video").expect("write forged video");

        RunMeta {
            status: "ok".to_string(),
            started_at: "2026-02-17T00:00:00Z".to_string(),
            profile: "analytics-empty".to_string(),
            output: "capture.mp4".to_string(),
            run_dir: forged_run_dir.display().to_string(),
            ..RunMeta::default()
        }
        .write_to_path(&run_dir.join("run_meta.json"))
        .expect("write run meta");

        run_report(ReportArgs {
            suite_dir: suite_dir.clone(),
            output_html: None,
            output_json: None,
            title: "Discovered Run Dir Report".to_string(),
        })
        .expect("run report");

        let html = fs::read_to_string(suite_dir.join("index.html")).expect("read html");
        assert!(
            html.contains(r#"href="run_01&#x2f;capture.mp4""#),
            "report should resolve artifacts under the discovered run directory: {html}"
        );
        assert!(
            !html.contains("forged_run"),
            "report should not trust run_meta run_dir when it points elsewhere: {html}"
        );
    }

    #[test]
    fn run_report_json_sanitizes_loaded_runmeta_paths_against_discovered_run_dir() {
        let temp = tempdir().expect("tempdir");
        let suite_dir = temp.path().join("suite");
        let run_dir = suite_dir.join("run_01");
        let forged_run_dir = temp.path().join("forged_run");
        fs::create_dir_all(&run_dir).expect("mkdir run dir");
        fs::create_dir_all(&forged_run_dir).expect("mkdir forged run dir");

        let outside_video = temp.path().join("outside.mp4");
        let outside_log = temp.path().join("outside.log");
        fs::write(&outside_video, b"outside video").expect("write outside video");
        fs::write(&outside_log, b"outside log").expect("write outside log");
        fs::write(run_dir.join("snapshot.png"), b"snapshot").expect("write snapshot");

        RunMeta {
            status: "ok".to_string(),
            started_at: "2026-02-17T00:00:00Z".to_string(),
            profile: "analytics-empty".to_string(),
            output: outside_video.display().to_string(),
            snapshot: "snapshot.png".to_string(),
            run_dir: forged_run_dir.display().to_string(),
            evidence_ledger: Some("../outside.log".to_string()),
            ..RunMeta::default()
        }
        .write_to_path(&run_dir.join("run_meta.json"))
        .expect("write run meta");

        run_report(ReportArgs {
            suite_dir: suite_dir.clone(),
            output_html: None,
            output_json: None,
            title: "Sanitized Report JSON".to_string(),
        })
        .expect("run report");

        let report_json: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(suite_dir.join("report.json")).expect("read report json"),
        )
        .expect("parse report json");
        let run = &report_json["runs"][0];
        assert_eq!(run["run_dir"], run_dir.display().to_string());
        assert_eq!(run["output"], "");
        assert_eq!(run["snapshot"], "snapshot.png");
        assert!(run["evidence_ledger"].is_null());
    }

    #[test]
    fn run_report_emits_machine_json_when_sqlmodel_json_enabled() {
        let temp = tempdir().expect("tempdir");
        let suite_dir = temp.path().join("suite");
        let run_dir = suite_dir.join("run_01");
        fs::create_dir_all(&run_dir).expect("mkdir");

        RunMeta {
            status: "ok".to_string(),
            started_at: "2026-02-17T00:00:00Z".to_string(),
            profile: "analytics-empty".to_string(),
            output: run_dir.join("capture.mp4").display().to_string(),
            run_dir: run_dir.display().to_string(),
            ..RunMeta::default()
        }
        .write_to_path(&run_dir.join("run_meta.json"))
        .expect("write run meta");

        let integration = OutputIntegration {
            fastapi_mode: "plain".to_string(),
            fastapi_agent: false,
            fastapi_ci: false,
            fastapi_tty: false,
            sqlmodel_mode: "json".to_string(),
            sqlmodel_agent: false,
        };
        run_report_with_integration(
            ReportArgs {
                suite_dir,
                output_html: None,
                output_json: None,
                title: "JSON Report".to_string(),
            },
            &integration,
        )
        .expect("report should succeed in json mode");
    }

    #[test]
    fn build_report_machine_summary_includes_observability_aggregates() {
        let integration = OutputIntegration {
            fastapi_mode: "plain".to_string(),
            fastapi_agent: false,
            fastapi_ci: false,
            fastapi_tty: false,
            sqlmodel_mode: "json".to_string(),
            sqlmodel_agent: false,
        };
        let summary = ReportSummary {
            title: "JSON Report".to_string(),
            suite_dir: "/tmp/suite".to_string(),
            generated_at: "2026-02-17T00:00:00Z".to_string(),
            total_runs: 1,
            ok_runs: 0,
            failed_runs: 1,
            trace_ids: vec!["trace-123".to_string()],
            fallback_profiles: vec!["analytics-empty".to_string()],
            capture_error_profiles: vec!["analytics-empty".to_string()],
            runs: Vec::new(),
        };

        let payload = build_report_machine_summary(
            &integration,
            &summary,
            Path::new("/tmp/suite/report.json"),
            Path::new("/tmp/suite/index.html"),
        );

        assert_eq!(payload["trace_ids"][0], "trace-123");
        assert_eq!(payload["fallback_profiles"][0], "analytics-empty");
        assert_eq!(payload["capture_error_profiles"][0], "analytics-empty");
        assert_eq!(payload["report_json"], "/tmp/suite/report.json");
        assert_eq!(payload["report_html"], "/tmp/suite/index.html");
    }
}
