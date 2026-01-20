use clap::Parser;
use serde_json::Value;
use std::env;
use std::fs::{create_dir_all, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output, Stdio};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

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
    println!(
        "[ralph] No prompt template found. What's the goal for this repo ({repo_name})?"
    );
    print!("[ralph] goal> ");
    io::stdout().flush()?;
    let mut input = String::new();
    let _ = io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

fn default_template_content() -> String {
    [
        "# Ralph loop bootstrap",
        "",
        "Goal context:",
        "{{GOAL}}",
        "",
        "If the goal is unclear, briefly infer it from the repo and list any clarifying",
        "questions in the progress log before proceeding.",
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

fn collect_repo_context(repo_name: &str, cwd: &Path) -> String {
    let mut lines = Vec::new();
    lines.push(format!("repo: {repo_name}"));
    lines.push(format!("path: {}", cwd.display()));

    if let Some(remote) = run_command_output("git", &["remote", "get-url", "origin"], cwd) {
        lines.push(format!("git origin: {remote}"));
    }
    if let Some(status) = run_command_output("git", &["status", "--short"], cwd) {
        lines.push(format!("git status:\n{status}"));
    }
    if let Some(files) = run_command_output("git", &["ls-files"], cwd) {
        let mut snippet = files;
        if snippet.len() > 2000 {
            snippet.truncate(2000);
            snippet.push_str("\n…");
        }
        lines.push(format!("tracked files:\n{snippet}"));
    }

    let readme_candidates = ["README.md", "Readme.md", "readme.md"];
    for name in readme_candidates {
        let path = cwd.join(name);
        if let Some(snippet) = read_file_snippet(&path, 4000) {
            lines.push(format!("README ({name}):\n{snippet}"));
            break;
        }
    }

    for name in ["Cargo.toml", "lakefile.lean", "package.json", "pyproject.toml"] {
        let path = cwd.join(name);
        if let Some(snippet) = read_file_snippet(&path, 1200) {
            lines.push(format!("{name}:\n{snippet}"));
        }
    }

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

fn parse_goal_from_output(output: &str) -> Option<String> {
    let candidate = extract_json_block(output)?;
    let value: Value = serde_json::from_str(&candidate).ok()?;
    value
        .get("goal")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn infer_goal_with_codex(
    repo_name: &str,
    cwd: &Path,
    model: &str,
    effort: &str,
    yolo: bool,
) -> io::Result<Option<String>> {
    let context = collect_repo_context(repo_name, cwd);
    let prompt = format!(
        "You are a repo analyst. Infer the primary project goal.\n\
Return ONLY JSON: {{\"goal\":\"...\"}}.\n\
Rules: goal is one sentence, no markdown, no extra keys.\n\n\
Context:\n{context}"
    );
    let output = run_codex(
        &prompt,
        model,
        effort,
        &[],
        false,
        yolo,
        false,
        None,
    )?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(parse_goal_from_output(&stdout))
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
) -> io::Result<Output> {
    let mut cmd = Command::new("codex");
    if !model.is_empty() {
        cmd.args(["--model", model]);
    }
    if !effort.is_empty() {
        cmd.args(["-c", &format!("model_reasoning_effort={}", effort)]);
    }
    if yolo {
        cmd.arg("--dangerously-bypass-approvals-and-sandbox");
    } else if full_auto {
        cmd.arg("--full-auto");
    }
    cmd.arg("exec");
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
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes())?;
    }
    child.wait_with_output()
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
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.output()
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

    let repo_name = cwd
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("repo");

    let mut goal = String::new();
    if !prompt_template.is_file() {
        let guessed = infer_goal_with_codex(repo_name, &cwd, &model, &reasoning_effort, yolo)?
            .unwrap_or_else(|| format!("Bootstrap {repo_name} with a PRD, progress log, and initial tasks."));
        println!("[ralph] Proposed goal: {guessed}");
        let accepted = prompt_yes_no("[ralph] Use this goal?");
        goal = match accepted {
            Ok(true) => guessed,
            Ok(false) => prompt_for_goal(repo_name)?,
            Err(_) => guessed,
        };
        let goal_text = if goal.is_empty() {
            "Goal: (unspecified) — infer from repo".to_string()
        } else {
            format!("Goal: {goal}")
        };
        let template = default_template_content().replace("{{GOAL}}", &goal_text);
        ensure_file(&prompt_template, &template)?;
    }

    if !prd_path.is_file() {
        let prd_goal = if goal.is_empty() {
            format!("# {repo_name} PRD\n\nGoal: (unspecified)\n")
        } else {
            format!("# {repo_name} PRD\n\nGoal: {goal}\n")
        };
        ensure_file(&prd_path, &prd_goal)?;
    }

    if !progress_path.is_file() {
        let progress = format!(
            "Initialized Ralph progress log for {repo_name}.\n"
        );
        ensure_file(&progress_path, &progress)?;
    }

    ensure_runner(&runner)?;

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
        let output = if runner == "codex" {
            run_codex(
                &prompt,
                &model,
                &reasoning_effort,
                &args.runner_arg,
                args.full_auto,
                yolo,
                args.resume,
                args.resume_id.as_deref(),
            )?
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
            )?
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
