use clap::Parser;
use serde_json::Value;
use std::env;
use std::fs::{create_dir_all, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use wait_timeout::ChildExt;

#[derive(Parser, Debug)]
#[command(name = "ralph", about = "Permissive Ralph loop runner")]
struct Args {
    #[arg(long, default_value = "codex")]
    runner: String,
    #[arg(long, default_value = "gpt-5.2-codex")]
    model: String,
    #[arg(long, value_name = "EFFORT", default_value = "xhigh")]
    reasoning_effort: String,
    #[arg(long, default_value_t = 24)]
    iterations: u32,
    #[arg(long, default_value_t = 15)]
    sleep: u64,
    #[arg(long, default_value_t = 0)]
    max_seconds: u64,
    #[arg(long)]
    prompt_template: Option<PathBuf>,
    #[arg(long)]
    prd: Option<PathBuf>,
    #[arg(long)]
    progress: Option<PathBuf>,
    #[arg(long)]
    log: Option<PathBuf>,
    #[arg(long)]
    no_log: bool,
    #[arg(long, default_value = "__RALPH_DONE__")]
    stop_token: String,
    #[arg(long, default_value = "-p")]
    prompt_flag: String,
    #[arg(long)]
    extra: Option<String>,
    #[arg(long)]
    goal: Option<String>,
    #[arg(long)]
    next_action: Option<String>,
    #[arg(long)]
    specialization: Option<String>,
    #[arg(long, default_value_t = true)]
    codex_json: bool,
    #[arg(long, default_value_t = 0)]
    runner_timeout: u64,
    #[arg(long, default_value_t = 24)]
    sdk_max_turns: u32,
    #[arg(long, default_value_t = true)]
    ensure_mcp: bool,
    #[arg(long)]
    context_log: Option<PathBuf>,
    #[arg(long, default_value_t = false)]
    infer_only: bool,
    #[arg(long, default_value_t = false)]
    list_mcp: bool,
    #[arg(long, action = clap::ArgAction::Append)]
    runner_arg: Vec<String>,
    #[arg(long)]
    resume: bool,
    #[arg(long)]
    resume_id: Option<String>,
    #[arg(long)]
    full_auto: bool,
    #[arg(long)]
    no_yolo: bool,
}

fn env_or_path(name: &str, fallback: PathBuf) -> PathBuf {
    env::var(name).map(PathBuf::from).unwrap_or(fallback)
}

fn load_prompt(template_path: &Path, prd_path: &Path, progress_path: &Path) -> io::Result<String> {
    let template = std::fs::read_to_string(template_path)?;
    let prd_ref = format!("@{}", prd_path.display());
    let progress_ref = format!("@{}", progress_path.display());
    Ok(template
        .replace("{{PRD}}", &prd_ref)
        .replace("{{PROGRESS}}", &progress_ref))
}

fn prompt_for_goal(repo_name: &str) -> io::Result<String> {
    loop {
        println!(
            "[ralph] No prompt template found. What's the goal for this repo ({repo_name})?"
        );
        print!("[ralph] goal> ");
        io::stdout().flush()?;
        let mut input = String::new();
        let _ = io::stdin().read_line(&mut input)?;
        let trimmed = input.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
        println!("[ralph] Goal cannot be empty.");
    }
}

fn prompt_for_next_action() -> io::Result<String> {
    loop {
        println!("[ralph] What's the immediate next action you want taken?");
        print!("[ralph] next action> ");
        io::stdout().flush()?;
        let mut input = String::new();
        let _ = io::stdin().read_line(&mut input)?;
        let trimmed = input.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
        println!("[ralph] Next action cannot be empty.");
    }
}

fn default_template_content() -> String {
    [
        "# [Ralph] {{GOAL}}",
        "",
        "## Summary",
        "Use this prompt like a GitHub issue. Keep scope tight and actionable.",
        "",
        "## Ultimate Goal (North Star)",
        "{{GOAL}}",
        "",
        "## Proposed Next Action (Confirm Alignment)",
        "{{NEXT_ACTION}}",
        "",
        "## Context",
        "- Repo context is provided below.",
        "- Use MCP servers if available (especially `openaiDeveloperDocs` and `linear`).",
        "",
        "## Scope",
        "- In scope:",
        "- Out of scope:",
        "",
        "## Acceptance Criteria",
        "- [ ] ...",
        "",
        "## Tasks",
        "- [ ] ...",
        "",
        "## Risks / Open Questions",
        "- ...",
        "",
        "## Links",
        "- Linear project/doc links if available",
        "",
        "Tasks:",
        "1) Draft or update the PRD at {{PRD}} with goal, scope, milestones, risks.",
        "2) Update the progress log at {{PROGRESS}} with status and next steps.",
        "3) If Linear is available, create or link a project + initial issues that mirror",
        "   the PRD and add the repo link.",
        "4) Start the first actionable task.",
        "",
    ]
    .join("\n")
}

fn prompt_yes_no(message: &str) -> io::Result<bool> {
    print!("{message} [y/N] ");
    io::stdout().flush()?;
    let mut input = String::new();
    let read = io::stdin().read_line(&mut input)?;
    if read == 0 {
        return Ok(false);
    }
    let answer = input.trim().to_lowercase();
    Ok(answer == "y" || answer == "yes")
}

fn run_command_output(cmd: &str, args: &[&str], cwd: &Path) -> Option<String> {
    let out = Command::new(cmd).args(args).current_dir(cwd).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn append_context(lines: &mut Vec<String>, label: &str, value: Option<String>, limit: usize) {
    if let Some(text) = value {
        let truncated = truncate_string(&text, limit);
        lines.push(format!("{label}:\n{truncated}"));
    }
}

fn linear_token() -> Option<String> {
    for name in ["LINEAR_API_KEY", "LINEAR_TOKEN", "LINEAR_API_TOKEN"] {
        if let Ok(value) = env::var(name) {
            let trimmed = value.trim().to_string();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }
    let home = env::var("HOME").ok()?;
    let config = Path::new(&home).join(".codex/config.toml");
    let content = std::fs::read_to_string(config).ok()?;
    if let Some(idx) = content.find("lin_api_") {
        let tail = &content[idx..];
        let token: String = tail
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        if !token.is_empty() {
            return Some(token);
        }
    }
    None
}

fn linear_auth_header(token: &str) -> String {
    let mut t = token.trim().to_string();
    if let Some(stripped) = t.strip_prefix("Bearer ") {
        t = stripped.trim().to_string();
    }
    if t.starts_with("lin_api_") {
        format!("Authorization: {t}")
    } else {
        format!("Authorization: Bearer {t}")
    }
}

fn linear_graphql(query: &str, variables: Value) -> Option<Value> {
    let token = linear_token()?;
    let client = reqwest::blocking::Client::new();
    let payload = serde_json::json!({
        "query": query,
        "variables": variables,
    });
    let resp = client
        .post("https://api.linear.app/graphql")
        .header("Content-Type", "application/json")
        .header("Authorization", linear_auth_header(&token))
        .json(&payload)
        .send()
        .ok()?;
    let status = resp.status();
    let value: Value = resp.json().ok()?;
    if !status.is_success() {
        return None;
    }
    if value.get("errors").is_some() {
        return None;
    }
    Some(value)
}

fn truncate_string(input: &str, limit: usize) -> String {
    if input.len() <= limit {
        return input.to_string();
    }
    let mut out = input[..limit].to_string();
    out.push_str("\n…");
    out
}

fn linear_context() -> Option<String> {
    let projects_query = "query Projects($first: Int!) { projects(first: $first) { nodes { id name description url } } }";
    let docs_query = "query Docs($first: Int!) { documents(first: $first) { nodes { id title url content project { name url } } } }";
    let issues_query = "query Issues($first: Int!) { issues(first: $first) { nodes { id title url state { name } project { name url } } } }";

    let projects = linear_graphql(projects_query, serde_json::json!({ "first": 25 }))?;
    let docs = linear_graphql(docs_query, serde_json::json!({ "first": 10 }));
    let issues = linear_graphql(issues_query, serde_json::json!({ "first": 50 }));

    let mut parts = Vec::new();
    parts.push("Linear projects (raw JSON):".to_string());
    parts.push(truncate_string(&projects.to_string(), 20000));
    if let Some(docs_value) = docs {
        parts.push("Linear documents (raw JSON):".to_string());
        parts.push(truncate_string(&docs_value.to_string(), 20000));
    }
    if let Some(issues_value) = issues {
        parts.push("Linear issues (raw JSON):".to_string());
        parts.push(truncate_string(&issues_value.to_string(), 20000));
    }
    Some(parts.join("\n\n"))
}

fn read_file_snippet(path: &Path, limit: usize) -> Option<String> {
    let contents = std::fs::read_to_string(path).ok()?;
    let mut snippet = contents.trim().to_string();
    if snippet.len() > limit {
        snippet.truncate(limit);
        snippet.push_str("\n…");
    }
    if snippet.is_empty() {
        None
    } else {
        Some(snippet)
    }
}

fn ensure_openai_docs_mcp() -> io::Result<()> {
    let home = match env::var("HOME") {
        Ok(value) => value,
        Err(_) => return Ok(()),
    };
    let config_path = Path::new(&home).join(".codex/config.toml");
    let mut content = std::fs::read_to_string(&config_path).unwrap_or_default();
    if content.contains("[mcp_servers.openaiDeveloperDocs]") {
        return Ok(());
    }
    if !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(
        "\n[mcp_servers.openaiDeveloperDocs]\nurl = \"https://developers.openai.com/mcp\"\n",
    );
    if let Some(parent) = config_path.parent() {
        create_dir_all(parent)?;
    }
    std::fs::write(config_path, content)?;
    Ok(())
}

fn list_mcp_servers() -> Vec<String> {
    let home = match env::var("HOME") {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };
    let config_path = Path::new(&home).join(".codex/config.toml");
    let content = match std::fs::read_to_string(&config_path) {
        Ok(data) => data,
        Err(_) => return Vec::new(),
    };
    let mut servers = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("[mcp_servers.") && trimmed.ends_with(']') {
            let name = trimmed
                .trim_start_matches("[mcp_servers.")
                .trim_end_matches(']');
            if !name.is_empty() {
                servers.push(name.to_string());
            }
        }
    }
    servers.sort();
    servers.dedup();
    servers
}

fn write_context_snapshot(path: &Path, context: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        create_dir_all(parent)?;
    }
    std::fs::write(path, context)?;
    Ok(())
}

fn prepare_inference_context(
    repo_name: &str,
    cwd: &Path,
    context_log: Option<&Path>,
) -> io::Result<String> {
    let context = collect_repo_context(repo_name, cwd);
    if let Some(path) = context_log {
        let _ = write_context_snapshot(path, &context);
    }
    Ok(context)
}

fn write_temp_file(prefix: &str, contents: &str) -> io::Result<PathBuf> {
    let mut path = env::temp_dir();
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    path.push(format!("{prefix}-{ts}.txt"));
    std::fs::write(&path, contents)?;
    Ok(path)
}

fn read_with_limit(mut reader: impl Read, limit: usize) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    while buf.len() < limit {
        let remaining = limit - buf.len();
        let read_size = chunk.len().min(remaining);
        match reader.read(&mut chunk[..read_size]) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => break,
        }
    }
    buf
}

fn run_process_with_timeout(
    mut cmd: Command,
    input: Option<&str>,
    timeout: Option<Duration>,
    capture_stdout: bool,
    capture_stderr: bool,
) -> io::Result<Output> {
    cmd.stdin(Stdio::piped())
        .stdout(if capture_stdout {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(if capture_stderr {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    let mut child = cmd.spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        if let Some(text) = input {
            stdin.write_all(text.as_bytes())?;
        }
    }

    let stdout_handle = if capture_stdout {
        child.stdout.take().map(|stdout| {
            thread::spawn(move || read_with_limit(stdout, 2 * 1024 * 1024))
        })
    } else {
        None
    };
    let stderr_handle = if capture_stderr {
        child.stderr.take().map(|stderr| {
            thread::spawn(move || read_with_limit(stderr, 2 * 1024 * 1024))
        })
    } else {
        None
    };

    let status = if let Some(timeout) = timeout {
        match child.wait_timeout(timeout)? {
            Some(status) => status,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(io::ErrorKind::TimedOut, "Runner timed out"));
            }
        }
    } else {
        child.wait()?
    };

    let stdout = stdout_handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    let stderr = stderr_handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn collect_repo_context(repo_name: &str, cwd: &Path) -> String {
    let mut lines = Vec::new();
    lines.push(format!("repo: {repo_name}"));
    lines.push(format!("path: {}", cwd.display()));

    let readme_candidates = ["README.md", "Readme.md", "readme.md"];
    for name in readme_candidates {
        let path = cwd.join(name);
        if let Some(snippet) = read_file_snippet(&path, 20000) {
            lines.push(format!("README ({name}):\n{snippet}"));
            break;
        }
    }

    for name in ["AGENTS.md", "CLAUDE.md"] {
        let path = cwd.join(name);
        if let Some(snippet) = read_file_snippet(&path, 12000) {
            lines.push(format!("{name}:\n{snippet}"));
        }
    }

    for name in ["ralph/PRD.md", "PRD.md", "prd.md"] {
        let path = cwd.join(name);
        if let Some(snippet) = read_file_snippet(&path, 12000) {
            lines.push(format!("PRD ({name}):\n{snippet}"));
            break;
        }
    }

    for name in ["ralph/progress.txt", "progress.txt"] {
        let path = cwd.join(name);
        if let Some(snippet) = read_file_snippet(&path, 8000) {
            lines.push(format!("Ralph progress log ({name}):\n{snippet}"));
            break;
        }
    }

    for name in ["Cargo.toml", "lakefile.lean", "package.json", "pyproject.toml"] {
        let path = cwd.join(name);
        if let Some(snippet) = read_file_snippet(&path, 8000) {
            lines.push(format!("{name}:\n{snippet}"));
        }
    }

    if let Some(linear) = linear_context() {
        lines.push(format!("Linear context (use for ultimate goal if relevant):\n{linear}"));
    } else {
        lines.push("Linear context: unavailable".to_string());
    }

    append_context(
        &mut lines,
        "git origin",
        run_command_output("git", &["remote", "get-url", "origin"], cwd),
        2000,
    );
    append_context(
        &mut lines,
        "git last commit",
        run_command_output("git", &["log", "-1", "--oneline"], cwd),
        2000,
    );
    append_context(
        &mut lines,
        "git recent commits",
        run_command_output("git", &["log", "-10", "--oneline"], cwd),
        8000,
    );
    append_context(
        &mut lines,
        "tracked files",
        run_command_output("git", &["ls-files"], cwd),
        20000,
    );

    append_context(
        &mut lines,
        "worktree TODO/FIXME/XXX (use for next action)",
        run_command_output(
            "rg",
            &[
                "-n",
                "--max-count",
                "200",
                "-S",
                "TODO|FIXME|XXX",
                ".",
            ],
            cwd,
        ),
        12000,
    );

    append_context(
        &mut lines,
        "worktree git status (use for next action)",
        run_command_output("git", &["status", "--short"], cwd),
        4000,
    );
    append_context(
        &mut lines,
        "worktree git diff --stat (use for next action)",
        run_command_output("git", &["diff", "--stat"], cwd),
        4000,
    );

    lines.join("\n\n")
}

fn extract_json_block(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.starts_with("```") {
        let mut lines = trimmed.lines();
        let _ = lines.next();
        let mut body = String::new();
        for line in lines {
            if line.trim_start().starts_with("```") {
                break;
            }
            body.push_str(line);
            body.push('\n');
        }
        let body = body.trim().to_string();
        if !body.is_empty() {
            return Some(body);
        }
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end <= start {
        return None;
    }
    Some(trimmed[start..=end].to_string())
}

fn prompt_for_feedback() -> io::Result<String> {
    loop {
        println!("[ralph] Provide corrections or desired direction for the goal/next action.");
        print!("[ralph] feedback> ");
        io::stdout().flush()?;
        let mut input = String::new();
        let _ = io::stdin().read_line(&mut input)?;
        let trimmed = input.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
        println!("[ralph] Feedback cannot be empty.");
    }
}

fn parse_goal_payload(output: &str) -> Option<(String, String)> {
    let candidate = extract_json_block(output)?;
    let value: Value = serde_json::from_str(&candidate).ok()?;
    let ultimate = value
        .get("ultimate_goal")
        .or_else(|| value.get("goal"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())?;
    let next_action = value
        .get("next_action")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())?;
    Some((ultimate, next_action))
}

fn build_inference_prompt(
    context: &str,
    feedback: Option<&str>,
    previous: Option<(String, String)>,
) -> String {
    let mut prompt = format!(
        "You are a repo analyst. Infer the ultimate project goal and the next concrete action.\n\
Ultimate goal is a stable, long-horizon objective; next action is immediate and concrete.\n\
Prioritize README/AGENTS/CLAUDE/PRD/Linear for the ultimate goal; ignore uncommitted diffs for the goal.\n\
For next action, use worktree TODOs, git status/diff, and progress log; keep it small and concrete.\n\
If Linear context is present, only use entries that match the repo name or purpose.\n\
Return ONLY JSON: {{\"ultimate_goal\":\"...\",\"next_action\":\"...\"}}.\n\
Rules: both are single sentences, no markdown, no extra keys.\n\
Think as long as needed before answering; output must be ONLY the JSON.\n\n\
Context:\n{context}"
    );
    if let Some(prev) = previous {
        prompt.push_str(&format!(
            "\n\nPrevious proposal:\n- ultimate_goal: {}\n- next_action: {}\n",
            prev.0, prev.1
        ));
    }
    if let Some(note) = feedback {
        prompt.push_str(&format!("\n\nUser feedback:\n{note}\n"));
    }
    prompt
}

fn infer_goal_with_codex(
    context: &str,
    model: &str,
    effort: &str,
    yolo: bool,
    specialization: Option<&str>,
    feedback: Option<&str>,
    previous: Option<(String, String)>,
    runner_timeout: Option<Duration>,
    codex_json: bool,
) -> io::Result<Option<(String, String)>> {
    let prompt = build_inference_prompt(context, feedback, previous);
    let output = run_codex(
        &prompt,
        model,
        effort,
        &[],
        false,
        yolo,
        false,
        None,
        specialization,
        codex_json,
        runner_timeout,
    )?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(parse_goal_payload(&stdout))
}

fn infer_goal_with_sdk(
    context: &str,
    model: &str,
    effort: &str,
    specialization: Option<&str>,
    feedback: Option<&str>,
    previous: Option<(String, String)>,
    sdk_max_turns: u32,
    runner_timeout: Option<Duration>,
) -> io::Result<Option<(String, String)>> {
    let prompt = build_inference_prompt(context, feedback, previous);
    let output = run_sdk(
        &prompt,
        model,
        effort,
        specialization,
        sdk_max_turns,
        runner_timeout,
    )?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(parse_goal_payload(&stdout))
}

fn ensure_file(path: &Path, content: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        create_dir_all(parent)?;
    }
    if !path.exists() {
        std::fs::write(path, content)?;
    }
    Ok(())
}

fn append_log(
    log_path: &Path,
    iteration: u32,
    stdout: &[u8],
    stderr: &[u8],
    status: &ExitStatus,
) -> io::Result<()> {
    if let Some(parent) = log_path.parent() {
        create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    writeln!(file, "[iteration {iteration}] {ts}")?;
    if !stdout.is_empty() {
        writeln!(file, "\n[stdout]")?;
        file.write_all(stdout)?;
    }
    if !stderr.is_empty() {
        writeln!(file, "\n[stderr]")?;
        file.write_all(stderr)?;
    }
    writeln!(file, "\n[exit-code] {:?}", status.code())?;
    writeln!(file, "\n{}", "-".repeat(80))?;
    Ok(())
}

fn run_codex(
    prompt: &str,
    model: &str,
    effort: &str,
    runner_args: &[String],
    full_auto: bool,
    yolo: bool,
    resume_last: bool,
    resume_id: Option<&str>,
    specialization: Option<&str>,
    codex_json: bool,
    runner_timeout: Option<Duration>,
) -> io::Result<Output> {
    let mut cmd = Command::new("codex");
    if !model.is_empty() {
        cmd.args(["--model", model]);
    }
    if !effort.is_empty() {
        cmd.args(["-c", &format!("model_reasoning_effort={}", effort)]);
    }
    if let Some(spec) = specialization {
        if !spec.trim().is_empty() {
            cmd.args(["-c", &format!("specialization={}", spec)]);
        }
    }
    if yolo {
        cmd.arg("--dangerously-bypass-approvals-and-sandbox");
    } else if full_auto {
        cmd.arg("--full-auto");
    }
    cmd.arg("exec");
    if codex_json {
        cmd.arg("--json");
    }
    let output_path = write_temp_file("ralph-last-message", "")?;
    cmd.args(["--output-last-message", output_path.to_string_lossy().as_ref()]);
    if resume_last || resume_id.is_some() {
        cmd.arg("resume");
        if let Some(id) = resume_id {
            cmd.arg(id);
        } else {
            cmd.arg("--last");
        }
    }
    if !runner_args.is_empty() {
        cmd.args(runner_args);
    }
    cmd.arg("-");
    let mut output = run_process_with_timeout(
        cmd,
        Some(prompt),
        runner_timeout,
        !codex_json,
        true,
    )?;
    if let Ok(message) = std::fs::read_to_string(&output_path) {
        if !message.trim().is_empty() {
            output.stdout = message.into_bytes();
        }
    }
    Ok(output)
}

fn has_arg(args: &[String], needle: &str) -> bool {
    args.iter().any(|arg| arg == needle)
}

fn run_generic(
    runner: &str,
    model: &str,
    prompt_flag: &str,
    prompt: &str,
    runner_args: &[String],
    yolo: bool,
    runner_timeout: Option<Duration>,
) -> io::Result<Output> {
    let mut cmd = Command::new(runner);
    if !model.is_empty() {
        cmd.args(["--model", model]);
    }
    let mut args = runner_args.to_vec();
    if yolo && runner == "claude" && !has_arg(&args, "--dangerously-skip-permissions") {
        args.push("--dangerously-skip-permissions".to_string());
    }
    if !args.is_empty() {
        cmd.args(&args);
    }
    cmd.arg(prompt_flag).arg(prompt);
    run_process_with_timeout(cmd, None, runner_timeout, true, true)
}

fn run_sdk(
    prompt: &str,
    model: &str,
    effort: &str,
    specialization: Option<&str>,
    max_turns: u32,
    runner_timeout: Option<Duration>,
) -> io::Result<Output> {
    let prompt_path = write_temp_file("ralph-prompt", prompt)?;
    let mut cmd = Command::new("uv");
    cmd.args([
        "run",
        "python",
        "scripts/ralph_agent.py",
        "--prompt-file",
        prompt_path.to_string_lossy().as_ref(),
        "--model",
        model,
        "--max-turns",
        &max_turns.to_string(),
        "--reasoning-effort",
        effort,
    ]);
    if let Some(spec) = specialization {
        if !spec.trim().is_empty() {
            cmd.args(["--specialization", spec]);
        }
    }
    run_process_with_timeout(cmd, None, runner_timeout, true, true)
}
fn ensure_runner(runner: &str) -> io::Result<()> {
    let found = which::which(runner).map_err(|_| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("Runner not found: {runner}"),
        )
    })?;
    let _ = found;
    Ok(())
}

fn main() -> io::Result<()> {
    let args = Args::parse();
    let cwd = env::current_dir()?;

    let default_prd = cwd.join("ralph/PRD.md");
    let default_progress = cwd.join("ralph/progress.txt");
    let default_template = cwd.join("ralph/prompt-template.md");
    let default_log = cwd.join("ralph/overnight.log");

    let runner = args.runner;
    let model = args.model;
    let reasoning_effort = args.reasoning_effort;
    let iterations = args.iterations;
    let sleep_secs = args.sleep;
    let max_seconds = args.max_seconds;
    let specialization = args.specialization.as_deref();
    let codex_json = args.codex_json;
    let runner_timeout = if args.runner_timeout > 0 {
        Some(Duration::from_secs(args.runner_timeout))
    } else {
        None
    };
    let context_log = args
        .context_log
        .clone()
        .or_else(|| Some(cwd.join("ralph/context.txt")));
    let prompt_template = args
        .prompt_template
        .unwrap_or_else(|| env_or_path("RALPH_PROMPT_TEMPLATE", default_template));
    let prd_path = args.prd.unwrap_or_else(|| env_or_path("RALPH_PRD", default_prd));
    let progress_path = args
        .progress
        .unwrap_or_else(|| env_or_path("RALPH_PROGRESS", default_progress));
    let log_path = args
        .log
        .unwrap_or_else(|| env_or_path("RALPH_LOG", default_log));
    let stop_token = args.stop_token;
    let prompt_flag = args.prompt_flag;
    let yolo = !args.no_yolo;
    let use_sdk = runner == "sdk";

    if args.ensure_mcp {
        let _ = ensure_openai_docs_mcp();
    }

    if args.list_mcp {
        let servers = list_mcp_servers();
        if servers.is_empty() {
            println!("No MCP servers configured.");
        } else {
            println!("Configured MCP servers:");
            for name in servers {
                println!("- {name}");
            }
        }
        return Ok(());
    }

    let repo_name = cwd
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("repo");

    let mut goal = args.goal.unwrap_or_default();
    let mut next_action = args.next_action.unwrap_or_default();
    let mut inference_context: Option<String> = None;

    if args.infer_only {
        if use_sdk {
            ensure_runner("uv")?;
        } else {
            ensure_runner("codex")?;
        }
        let context = prepare_inference_context(repo_name, &cwd, context_log.as_deref())?;
        let result = if use_sdk {
            infer_goal_with_sdk(
                &context,
                &model,
                &reasoning_effort,
                specialization,
                None,
                None,
                args.sdk_max_turns,
                runner_timeout,
            )?
        } else {
            infer_goal_with_codex(
                &context,
                &model,
                &reasoning_effort,
                yolo,
                specialization,
                None,
                None,
                runner_timeout,
                codex_json,
            )?
        };
        if let Some((ultimate, action)) = result {
            let output = serde_json::json!({
                "ultimate_goal": ultimate,
                "next_action": action
            });
            println!("{output}");
            return Ok(());
        }
        return Err(io::Error::new(io::ErrorKind::Other, "Inference failed"));
    }
    if !prompt_template.is_file() {
        if goal.is_empty() || next_action.is_empty() {
            if use_sdk {
                ensure_runner("uv")?;
            } else {
                ensure_runner("codex")?;
            }
            if inference_context.is_none() {
                inference_context = Some(prepare_inference_context(
                    repo_name,
                    &cwd,
                    context_log.as_deref(),
                )?);
            }
            let context = inference_context.as_ref().unwrap();
            let mut proposal = if use_sdk {
                infer_goal_with_sdk(
                    context,
                    &model,
                    &reasoning_effort,
                    specialization,
                    None,
                    None,
                    args.sdk_max_turns,
                    runner_timeout,
                )?
            } else {
                infer_goal_with_codex(
                    context,
                    &model,
                    &reasoning_effort,
                    yolo,
                    specialization,
                    None,
                    None,
                    runner_timeout,
                    codex_json,
                )?
            }
            .unwrap_or_else(|| {
                (
                    format!("Bootstrap {repo_name} with a PRD, progress log, and initial tasks."),
                    "Draft PRD and create initial tasks in Linear.".to_string(),
                )
            });

            loop {
                if goal.is_empty() {
                    println!("[ralph] Proposed ultimate goal: {}", proposal.0);
                    if prompt_yes_no("[ralph] Use this ultimate goal?")? {
                        goal = proposal.0.clone();
                    }
                }
                if next_action.is_empty() {
                    println!("[ralph] Proposed next action: {}", proposal.1);
                    if prompt_yes_no("[ralph] Use this next action?")? {
                        next_action = proposal.1.clone();
                    }
                }

                if !goal.is_empty() && !next_action.is_empty() {
                    break;
                }

                let feedback = prompt_for_feedback()?;
                let refined = if use_sdk {
                    infer_goal_with_sdk(
                        context,
                        &model,
                        &reasoning_effort,
                        specialization,
                        Some(&feedback),
                        Some(proposal.clone()),
                        args.sdk_max_turns,
                        runner_timeout,
                    )?
                } else {
                    infer_goal_with_codex(
                        context,
                        &model,
                        &reasoning_effort,
                        yolo,
                        specialization,
                        Some(&feedback),
                        Some(proposal.clone()),
                        runner_timeout,
                        codex_json,
                    )?
                };
                match refined {
                    Some(pair) => proposal = pair,
                    None => {
                        if goal.is_empty() {
                            goal = prompt_for_goal(repo_name)?;
                        }
                        if next_action.is_empty() {
                            next_action = prompt_for_next_action()?;
                        }
                        break;
                    }
                }
            }
        }
        let goal_text = if goal.is_empty() {
            "Goal: (unspecified) — infer from repo".to_string()
        } else {
            format!("Goal: {goal}")
        };
        let next_action_text = if next_action.is_empty() {
            "Next action: (unspecified)".to_string()
        } else {
            format!("{next_action}")
        };
        let template = default_template_content()
            .replace("{{GOAL}}", &goal_text)
            .replace("{{NEXT_ACTION}}", &next_action_text);
        ensure_file(&prompt_template, &template)?;
    }

    if !prd_path.is_file() {
        let prd_goal = if goal.is_empty() {
            format!("# {repo_name} PRD\n\nGoal: (unspecified)\n")
        } else {
            format!("# {repo_name} PRD\n\nGoal: {goal}\n")
        };
        let prd_next = if next_action.is_empty() {
            "Next action: (unspecified)\n".to_string()
        } else {
            format!("Next action: {next_action}\n")
        };
        ensure_file(&prd_path, &format!("{prd_goal}\n{prd_next}"))?;
    }

    if !progress_path.is_file() {
        let progress = format!(
            "Initialized Ralph progress log for {repo_name}.\n"
        );
        ensure_file(&progress_path, &progress)?;
    }

    if runner == "sdk" {
        ensure_runner("uv")?;
    } else {
        ensure_runner(&runner)?;
    }

    let mut prompt = load_prompt(&prompt_template, &prd_path, &progress_path)?;
    if let Some(extra) = args.extra.as_deref() {
        if !extra.trim().is_empty() {
            prompt = format!("{extra}\n\n{prompt}");
        }
    }
    let start = Instant::now();
    let mut stop_reason: Option<String> = None;

    for i in 1..=iterations {
        if max_seconds > 0 && start.elapsed().as_secs() >= max_seconds {
            stop_reason = Some(format!("reached max runtime ({max_seconds}s)"));
            break;
        }
        println!("[ralph] iteration {i}/{iterations}");
        let output = {
            let result = if runner == "codex" {
                run_codex(
                    &prompt,
                    &model,
                    &reasoning_effort,
                    &args.runner_arg,
                    args.full_auto,
                    yolo,
                    args.resume,
                    args.resume_id.as_deref(),
                    specialization,
                    codex_json,
                    runner_timeout,
                )
            } else if use_sdk {
                run_sdk(
                    &prompt,
                    &model,
                    &reasoning_effort,
                    specialization,
                    args.sdk_max_turns,
                    runner_timeout,
                )
            } else {
                if (args.resume || args.resume_id.is_some()) && runner != "codex" {
                    eprintln!("[ralph] resume requested but runner is not codex; ignoring resume.");
                }
                run_generic(
                    &runner,
                    &model,
                    &prompt_flag,
                    &prompt,
                    &args.runner_arg,
                    yolo,
                    runner_timeout,
                )
            };
            match result {
                Ok(output) => output,
                Err(err) => {
                    if err.kind() == io::ErrorKind::TimedOut {
                        stop_reason = Some("runner timed out".to_string());
                        break;
                    } else {
                        return Err(err);
                    }
                }
            }
        };

        let stdout = output.stdout;
        let stderr = output.stderr;

        if !stdout.is_empty() {
            io::stdout().write_all(&stdout)?;
        }
        if !stderr.is_empty() {
            io::stderr().write_all(&stderr)?;
        }

        if !args.no_log {
            append_log(&log_path, i, &stdout, &stderr, &output.status)?;
        }

        if !output.status.success() {
            let code = output.status.code().unwrap_or(1);
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("Runner exited with code {code}"),
            ));
        }

        let stdout_text = String::from_utf8_lossy(&stdout);
        if stdout_text.contains(&stop_token) {
            stop_reason = Some("completion token detected".to_string());
            break;
        }

        if i < iterations {
            println!("[ralph] sleeping {sleep_secs}s before next iteration");
            std::thread::sleep(std::time::Duration::from_secs(sleep_secs));
        } else {
            stop_reason = Some("reached max iterations".to_string());
        }
    }

    if let Some(reason) = stop_reason {
        println!("[ralph] stop: {reason}.");
    }

    Ok(())
}

mod which {
    use std::env;
    use std::path::{Path, PathBuf};

    pub fn which<S: AsRef<std::ffi::OsStr>>(binary: S) -> Result<PathBuf, ()> {
        let binary = binary.as_ref();
        let path_var = env::var_os("PATH").ok_or(())?;
        for path in env::split_paths(&path_var) {
            let candidate = path.join(binary);
            if is_executable(&candidate) {
                return Ok(candidate);
            }
        }
        Err(())
    }

    fn is_executable(path: &Path) -> bool {
        path.is_file()
    }
}
