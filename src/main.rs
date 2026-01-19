use clap::Parser;
use std::env;
use std::fs::{create_dir_all, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Parser, Debug)]
#[command(name = "ralph", about = "Permissive Ralph loop runner")]
struct Args {
    #[arg(long)]
    runner: Option<String>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long, value_name = "EFFORT")]
    reasoning_effort: Option<String>,
    #[arg(long)]
    iterations: Option<u32>,
    #[arg(long)]
    sleep: Option<u64>,
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
    #[arg(long)]
    stop_token: Option<String>,
    #[arg(long)]
    prompt_flag: Option<String>,
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

fn env_or(name: &str, fallback: &str) -> String {
    env::var(name).unwrap_or_else(|_| fallback.to_string())
}

fn env_or_path(name: &str, fallback: PathBuf) -> PathBuf {
    env::var(name).map(PathBuf::from).unwrap_or(fallback)
}

fn env_or_u32(name: &str, fallback: u32) -> u32 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(fallback)
}

fn env_or_u64(name: &str, fallback: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(fallback)
}

fn load_prompt(template_path: &Path, prd_path: &Path, progress_path: &Path) -> io::Result<String> {
    let template = std::fs::read_to_string(template_path)?;
    let prd_ref = format!("@{}", prd_path.display());
    let progress_ref = format!("@{}", progress_path.display());
    Ok(template
        .replace("{{PRD}}", &prd_ref)
        .replace("{{PROGRESS}}", &progress_ref))
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

    let runner = args
        .runner
        .unwrap_or_else(|| env_or("RALPH_RUNNER", "codex"));
    let model = args
        .model
        .unwrap_or_else(|| env_or("RALPH_MODEL", "gpt-5.2-codex"));
    let reasoning_effort = args
        .reasoning_effort
        .unwrap_or_else(|| env_or("RALPH_EFFORT", "xhigh"));
    let iterations = args
        .iterations
        .unwrap_or_else(|| env_or_u32("RALPH_ITERATIONS", 24));
    let sleep_secs = args.sleep.unwrap_or_else(|| env_or_u64("RALPH_SLEEP", 15));
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
    let stop_token = args
        .stop_token
        .unwrap_or_else(|| env_or("RALPH_STOP_TOKEN", "__RALPH_DONE__"));
    let prompt_flag = args
        .prompt_flag
        .unwrap_or_else(|| env_or("RALPH_PROMPT_FLAG", "-p"));

    if !prompt_template.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Missing prompt template: {}", prompt_template.display()),
        ));
    }
    if !prd_path.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Missing PRD: {}", prd_path.display()),
        ));
    }
    if !progress_path.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Missing progress log: {}", progress_path.display()),
        ));
    }

    ensure_runner(&runner)?;

    let prompt = load_prompt(&prompt_template, &prd_path, &progress_path)?;
    let yolo = !args.no_yolo;

    for i in 1..=iterations {
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
            println!("[ralph] completion detected, stopping.");
            break;
        }

        if i < iterations {
            println!("[ralph] sleeping {sleep_secs}s before next iteration");
            std::thread::sleep(std::time::Duration::from_secs(sleep_secs));
        }
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
