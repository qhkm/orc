use clap::{Parser, Subcommand};
use orc_tui::app::App;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "orc",
    about = "Conversational coding shell with smart model routing"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Open the TUI session hub
    Hub,
    /// List sessions
    List {
        /// Output format: "json" or "text" (default)
        #[arg(long, short)]
        format: Option<String>,
        /// Filter by tool name (e.g. claude, codex, gemini)
        #[arg(long)]
        tool: Option<String>,
    },
    /// Resume a session
    Resume {
        /// Session ID (prefix match is accepted)
        id: String,
        /// Target tool for cross-tool resume
        #[arg(long)]
        r#in: Option<String>,
    },
    /// Export session as SCP file
    Export {
        /// Session ID (prefix match is accepted)
        id: String,
        /// Output path (default: ./.orc/context.json)
        #[arg(long, short)]
        output: Option<PathBuf>,
    },
    /// Import SCP file and launch tool
    Import {
        /// Path to SCP file
        file: PathBuf,
        /// Target tool to launch (required)
        #[arg(long)]
        r#in: String,
    },
    /// Run one task with an agent, then verify the result.
    ///
    /// Exit codes: 0 pass, 1 fail, 2 stuck, 3 rate-limited, 4 orc error.
    Run {
        /// The task for the agent
        task: String,
        /// Agent that does the work (default: master provider)
        #[arg(long, short)]
        provider: Option<String>,
        /// Model for the agent
        #[arg(long, short)]
        model: Option<String>,
        /// Shell command that must exit 0 afterwards (repeatable), e.g. --check "cargo test"
        #[arg(long = "check", value_name = "CMD")]
        checks: Vec<String>,
        /// Plain-language criterion judged by a reviewer agent (repeatable)
        #[arg(long = "accept", value_name = "CRITERION")]
        accept: Vec<String>,
        /// Providers to hand off to when an attempt does not pass, in order, e.g. --fallback codex,gemini
        /// (default: every other installed provider that can run tasks)
        #[arg(long, value_delimiter = ',', value_name = "PROVIDERS")]
        fallback: Vec<String>,
        /// Never hand off to another provider (a failing attempt is still retried by the same one)
        #[arg(long, conflicts_with = "fallback")]
        no_fallback: bool,
        /// Total attempts, including the first
        #[arg(long, default_value_t = 3)]
        max_attempts: u32,
        /// Agent that reviews --accept criteria (default: master provider)
        #[arg(long)]
        reviewer: Option<String>,
        /// Model for the reviewer
        #[arg(long)]
        reviewer_model: Option<String>,
        /// Kill the agent after this many seconds
        #[arg(long, default_value_t = 1800)]
        timeout: u64,
        /// Kill the agent after this many seconds without output
        #[arg(long, default_value_t = 300)]
        idle: u64,
        /// Timeout for each --check command, in seconds
        #[arg(long, default_value_t = 600)]
        check_timeout: u64,
        /// Repository to work in (default: current directory)
        #[arg(long)]
        repo: Option<PathBuf>,
        /// Print the report as JSON instead of live output
        #[arg(long)]
        json: bool,
    },
    /// Flow management
    Flow {
        #[command(subcommand)]
        cmd: FlowCommands,
    },
    /// Initialize orc config
    Init,
    /// Provider health check
    Providers {
        #[command(subcommand)]
        cmd: ProvidersCommands,
    },
}

#[derive(Subcommand)]
enum FlowCommands {
    /// Create a new flow
    Create {
        /// Flow name
        name: String,
        /// Natural-language prompt describing the flow stages
        #[arg(long, short)]
        prompt: Option<String>,
    },
}

#[derive(Subcommand)]
enum ProvidersCommands {
    /// Check all providers
    Doctor,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        None => {
            // REPL mode
            orc_repl::repl::run_repl()?;
        }

        Some(Commands::Hub) => {
            // TUI hub mode
            let mut app = App::new();
            app.run()?;
        }

        Some(Commands::List { format, tool }) => {
            cmd_list(format.as_deref(), tool.as_deref())?;
        }

        Some(Commands::Resume { id, r#in }) => {
            cmd_resume(&id, r#in.as_deref())?;
        }

        Some(Commands::Export { id, output }) => {
            cmd_export(&id, output)?;
        }

        Some(Commands::Import { file, r#in }) => {
            cmd_import(&file, &r#in)?;
        }

        Some(Commands::Run {
            task,
            provider,
            model,
            checks,
            accept,
            fallback,
            no_fallback,
            max_attempts,
            reviewer,
            reviewer_model,
            timeout,
            idle,
            check_timeout,
            repo,
            json,
        }) => {
            let args = RunArgs {
                task, provider, model, checks, accept, fallback, no_fallback, max_attempts,
                reviewer, reviewer_model, timeout, idle, check_timeout, repo, json,
            };
            match cmd_run(args) {
                Ok(code) => std::process::exit(code),
                Err(e) => {
                    eprintln!("orc run: {e:#}");
                    std::process::exit(4);
                }
            }
        }

        Some(Commands::Flow {
            cmd: FlowCommands::Create { name, prompt },
        }) => {
            cmd_flow_create(&name, prompt.as_deref())?;
        }

        Some(Commands::Init) => {
            cmd_init()?;
        }

        Some(Commands::Providers {
            cmd: ProvidersCommands::Doctor,
        }) => {
            cmd_providers_doctor()?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Headless helpers
// ---------------------------------------------------------------------------

fn build_full_index() -> anyhow::Result<orc_core::session::index::SessionIndex> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home dir"))?;
    let claude_root = home.join(".claude").join("projects");
    let codex_root = home.join(".codex").join("sessions");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    rt.block_on(orc_core::session::index::build_index_full(
        &claude_root,
        &codex_root,
        None,
        None,
        None,
    ))
}

/// Find a session by exact ID or prefix match.
pub fn find_session_by_prefix<'a>(
    sessions: &'a [orc_core::session::types::UnifiedSession],
    prefix: &str,
) -> Option<&'a orc_core::session::types::UnifiedSession> {
    // Exact match first
    if let Some(s) = sessions.iter().find(|s| s.id == prefix) {
        return Some(s);
    }
    // Prefix match
    let matches: Vec<_> = sessions
        .iter()
        .filter(|s| s.id.starts_with(prefix))
        .collect();
    if matches.len() == 1 {
        return Some(matches[0]);
    }
    None
}

/// Format a duration as a human-readable "time ago" string.
pub fn time_ago(dt: chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let secs = now.signed_duration_since(dt).num_seconds();
    if secs < 60 {
        return "just now".to_string();
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{}m ago", mins);
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("{}h ago", hours);
    }
    let days = hours / 24;
    format!("{}d ago", days)
}

/// Format a session as a text table row.
/// Returns: "source  short_id  cwd_tail  time_ago  summary"
pub fn format_session_row(session: &orc_core::session::types::UnifiedSession) -> String {
    let source = format!("{:?}", session.source).to_lowercase();

    let short_id: String = session.id.chars().take(8).collect();

    // Last 2 path segments of cwd
    let cwd_parts: Vec<_> = session.cwd.components().collect();
    let cwd_tail = if cwd_parts.len() >= 2 {
        let tail_start = cwd_parts.len() - 2;
        let path: PathBuf = cwd_parts[tail_start..].iter().collect();
        path.to_string_lossy().to_string()
    } else {
        session.cwd.to_string_lossy().to_string()
    };

    let ago = time_ago(session.updated_at);

    let summary: String = session.summary.chars().take(50).collect();

    format!(
        "{:<10} {:<10} {:<30} {:<12} {}",
        source, short_id, cwd_tail, ago, summary
    )
}

fn source_label(source: &orc_core::session::types::SessionSource) -> &'static str {
    use orc_core::session::types::SessionSource;
    match source {
        SessionSource::Claude => "claude",
        SessionSource::Codex => "codex",
        SessionSource::Gemini => "gemini",
        SessionSource::Copilot => "copilot",
        SessionSource::OpenCode => "opencode",
        SessionSource::Kimi => "kimi",
        SessionSource::Amp => "amp",
        SessionSource::Droid => "droid",
        SessionSource::Scp => "scp",
    }
}

// ---------------------------------------------------------------------------
// Command implementations
// ---------------------------------------------------------------------------

fn cmd_list(format: Option<&str>, tool: Option<&str>) -> anyhow::Result<()> {
    let index = build_full_index()?;
    let sessions: Vec<_> = index
        .sessions
        .iter()
        .filter(|s| {
            if let Some(t) = tool {
                source_label(&s.source) == t
            } else {
                true
            }
        })
        .collect();

    match format {
        Some("json") => {
            let json = serde_json::to_string_pretty(&sessions)?;
            println!("{}", json);
        }
        _ => {
            // Plain text table
            println!(
                "{:<10} {:<10} {:<30} {:<12} {}",
                "TOOL", "ID", "CWD", "WHEN", "SUMMARY"
            );
            println!("{}", "-".repeat(80));
            for session in sessions {
                println!("{}", format_session_row(session));
            }
        }
    }

    Ok(())
}

fn cmd_resume(id: &str, target_tool: Option<&str>) -> anyhow::Result<()> {
    use orc_core::invoke::resume::{
        build_cross_tool_command, build_resume_command, spawn_and_exit,
    };
    use orc_core::session::scp::{default_scp_path, write_scp};
    use orc_core::session::types::{ConversationMessage, Role, SessionContext};

    let index = build_full_index()?;
    let session = find_session_by_prefix(&index.sessions, id)
        .ok_or_else(|| anyhow::anyhow!("session '{}' not found", id))?;

    // Auto-write SCP before resume so the target tool can load context
    let scp_path = default_scp_path(&session.cwd);
    let ctx = SessionContext {
        session: session.clone(),
        messages: vec![ConversationMessage {
            role: Role::User,
            content: session.summary.clone(),
            model: None,
            timestamp: Some(session.updated_at),
        }],
        tool_activity: vec![],
        files_modified: session.files_modified.clone(),
        reasoning_notes: vec![],
        pending_tasks: vec![],
        token_usage: None,
        markdown: String::new(),
    };
    let _ = write_scp(&scp_path, &ctx);

    if let Some(tool) = target_tool {
        // Cross-tool resume
        let target_source = parse_source(tool)?;
        let prompt = format!(
            "I'm continuing a coding session. Previous tool: {:?}. Session ID: {}. CWD: {}. Task: {}",
            session.source,
            session.id,
            session.cwd.display(),
            session.summary
        );
        let cmd = build_cross_tool_command(target_source, &prompt, &session.cwd.to_string_lossy());
        spawn_and_exit(cmd)?;
    } else {
        // Same-tool resume
        let cmd = build_resume_command(
            session.source,
            &session.id,
            &session.original_path.to_string_lossy(),
            &session.cwd.to_string_lossy(),
        );
        spawn_and_exit(cmd)?;
    }

    Ok(())
}

fn cmd_export(id: &str, output: Option<PathBuf>) -> anyhow::Result<()> {
    use orc_core::session::scp::{default_scp_path, write_scp};
    use orc_core::session::types::{ConversationMessage, Role, SessionContext};

    let index = build_full_index()?;
    let session = find_session_by_prefix(&index.sessions, id)
        .ok_or_else(|| anyhow::anyhow!("session '{}' not found", id))?;

    // Build a minimal SessionContext from the UnifiedSession metadata.
    // Full message content requires re-parsing the original JSONL, which is
    // not needed for a basic SCP export of session metadata.
    let ctx = SessionContext {
        session: session.clone(),
        messages: vec![ConversationMessage {
            role: Role::User,
            content: session.summary.clone(),
            model: None,
            timestamp: Some(session.created_at),
        }],
        tool_activity: vec![],
        files_modified: vec![],
        reasoning_notes: vec![],
        pending_tasks: vec![],
        token_usage: None,
        markdown: String::new(),
    };

    let out_path = output.unwrap_or_else(|| default_scp_path(&session.cwd));
    write_scp(&out_path, &ctx)?;
    println!("{}", out_path.display());

    Ok(())
}

fn cmd_import(file: &PathBuf, target_tool: &str) -> anyhow::Result<()> {
    use orc_core::invoke::resume::{build_cross_tool_command, spawn_and_exit};
    use orc_core::session::handoff::generate_handoff_markdown;
    use orc_core::session::scp::read_scp;

    let mut ctx = read_scp(file)?;
    ctx.markdown = generate_handoff_markdown(&ctx);

    let target_source = parse_source(target_tool)?;
    let prompt = format!(
        "I'm continuing a coding session. Here's the full context:\n\n---\n\n{}",
        ctx.markdown
    );

    let cmd = build_cross_tool_command(target_source, &prompt, &ctx.session.cwd.to_string_lossy());
    spawn_and_exit(cmd)?;

    Ok(())
}

struct RunArgs {
    task: String,
    provider: Option<String>,
    model: Option<String>,
    checks: Vec<String>,
    accept: Vec<String>,
    fallback: Vec<String>,
    no_fallback: bool,
    max_attempts: u32,
    reviewer: Option<String>,
    reviewer_model: Option<String>,
    timeout: u64,
    idle: u64,
    check_timeout: u64,
    repo: Option<PathBuf>,
    json: bool,
}

/// Master provider from ~/.orc/config.json, or "claude" if there is no config yet.
fn master_provider() -> String {
    orc_core::config::default_config_path()
        .and_then(|p| orc_core::config::load_config(&p))
        .map(|c| c.master_provider)
        .unwrap_or_else(|_| "claude".to_string())
}

fn lookup_provider(name: &str) -> anyhow::Result<Box<dyn orc_core::invoke::provider::Provider>> {
    orc_core::invoke::get_provider(name).ok_or_else(|| {
        anyhow::anyhow!(
            "provider '{name}' can't run tasks yet (available: {})",
            orc_core::invoke::providers::all_providers().join(", ")
        )
    })
}

/// `orc run`: returns the process exit code (the verdict's).
fn cmd_run(args: RunArgs) -> anyhow::Result<i32> {
    use orc_core::invoke::provider::Provider;
    use orc_core::relay::{run_relay, verdict_summary, RelayProgress};
    use orc_core::session::types::StreamEvent;
    use orc_core::verify::{Progress, StageSpec, Verdict};

    if args.task.trim().is_empty() {
        anyhow::bail!("task cannot be empty");
    }
    let working_dir = match args.repo {
        Some(repo) => repo.canonicalize()?,
        None => std::env::current_dir()?,
    };
    let master = master_provider();
    let primary = args.provider.clone().unwrap_or_else(|| master.clone());

    // Providers in hand-off order: the primary first, then the fallbacks.
    let mut names = vec![primary.clone()];
    if !args.no_fallback {
        if args.fallback.is_empty() {
            for name in orc_core::invoke::providers::all_providers() {
                let installed = lookup_provider(name).map(|p| p.detect().installed).unwrap_or(false);
                if installed {
                    names.push(name.to_string());
                }
            }
        } else {
            names.extend(args.fallback.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
        }
    }
    let mut seen = std::collections::HashSet::new();
    names.retain(|n| seen.insert(n.clone()));
    let agents = names.iter().map(|n| lookup_provider(n)).collect::<anyhow::Result<Vec<_>>>()?;
    let agent_refs: Vec<&dyn Provider> = agents.iter().map(|a| a.as_ref()).collect();

    let reviewer = if args.accept.is_empty() {
        None
    } else {
        let name = args.reviewer.clone().unwrap_or_else(|| master.clone());
        if name == primary && !args.json {
            eprintln!("note: reviewer is the same provider as the agent ({name}); use --reviewer for an independent check");
        }
        Some(lookup_provider(&name)?)
    };

    let now = chrono::Local::now();
    let run_id = format!("run-{}-{:04x}", now.format("%Y%m%d-%H%M%S"), now.timestamp_subsec_micros() & 0xffff);
    let spec = StageSpec {
        run_id,
        task: args.task,
        agent_prompt: None,
        attempt: 1,
        handoff_from: None,
        base_commit: None,
        working_dir,
        model: args.model,
        checks: args.checks,
        accept: args.accept,
        reviewer_model: args.reviewer_model,
        timeout_secs: args.timeout,
        idle_secs: args.idle,
        check_timeout_secs: args.check_timeout,
    };

    let json = args.json;
    if !json {
        eprintln!("▸ task: {}", spec.task);
        if names.len() > 1 {
            eprintln!("  hand-off order: {} (max {} attempts)", names.join(" → "), args.max_attempts);
        }
    }
    let progress = move |p: RelayProgress| {
        if json {
            return;
        }
        match p {
            RelayProgress::AttemptStarted { attempt, max_attempts, provider } => {
                eprintln!("\n▸ attempt {attempt}/{max_attempts}: {provider}")
            }
            RelayProgress::Stage(Progress::Agent(StreamEvent::Text { content })) => eprintln!("  {}", content.trim_end()),
            RelayProgress::Stage(Progress::Agent(StreamEvent::ToolStart { name, .. })) => eprintln!("  · {name}"),
            RelayProgress::Stage(Progress::Agent(StreamEvent::Error { message })) => eprintln!("  ! {message}"),
            RelayProgress::Stage(Progress::Agent(_)) => {}
            RelayProgress::Stage(Progress::AgentFinished { exit_code, exit_reason, duration_ms }) => {
                eprintln!("▸ agent finished: exit {exit_code} ({exit_reason:?}) in {}s", duration_ms / 1000)
            }
            RelayProgress::Stage(Progress::CheckStarted(cmd)) => eprintln!("▸ check: {cmd}"),
            RelayProgress::Stage(Progress::CheckFinished(r)) => {
                eprintln!("  {} {} ({}s)", if r.passed { "✓" } else { "✗" }, r.command, r.duration_ms / 1000)
            }
            RelayProgress::Stage(Progress::ReviewStarted(name)) => eprintln!("▸ review by {name}"),
            RelayProgress::Stage(Progress::Skipped(why)) => eprintln!("▸ {why}"),
            RelayProgress::AttemptFinished { attempt, provider, verdict } => {
                eprintln!("▸ attempt {attempt} ({provider}): {}", verdict_summary(&verdict))
            }
            RelayProgress::Handoff { from, to, .. } => eprintln!("▸ handing off {from} → {to}"),
            RelayProgress::Stopped(why) => eprintln!("▸ stopping: {why}"),
        }
    };

    let rt = tokio::runtime::Runtime::new()?;
    let report = rt.block_on(run_relay(&spec, args.max_attempts, &agent_refs, reviewer.as_deref(), progress))?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(report.exit_code);
    }

    let last = report.attempts.last().expect("at least one attempt");
    if let Some(review) = &last.review {
        for r in &review.results {
            eprintln!("  {} {} — {}", if r.pass { "✓" } else { "✗" }, r.criterion, r.reason);
        }
    }
    eprintln!();
    let by = if report.attempts.len() > 1 {
        let who: Vec<&str> = report.attempts.iter().map(|a| a.provider.as_str()).collect();
        format!(" after {} attempts ({})", report.attempts.len(), who.join(" → "))
    } else {
        String::new()
    };
    match &report.verdict {
        Verdict::Pass => println!("PASS{by}"),
        Verdict::Fail { reason, evidence } => {
            println!("FAIL{by}: {reason}");
            if !evidence.is_empty() {
                println!("{evidence}");
            }
        }
        Verdict::Stuck { reason, after_secs } => println!("STUCK{by}: {reason} (after {after_secs}s)"),
        Verdict::Limit { provider, message } => println!("LIMIT{by}: {provider}: {message}"),
    }
    if let Some(why) = &report.stop_reason {
        println!("stopped: {why}");
    }
    println!("run {} · log {}", report.run_id, report.log_path.display());
    Ok(report.exit_code)
}

fn cmd_flow_create(name: &str, prompt: Option<&str>) -> anyhow::Result<()> {
    use orc_core::flow::{compile, save_flow, CompileOptions};

    let prompt_text = match prompt {
        Some(p) => p.to_string(),
        None => {
            // Prompt user for input
            eprint!("Flow prompt: ");
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            input.trim().to_string()
        }
    };

    if prompt_text.is_empty() {
        return Err(anyhow::anyhow!("flow prompt cannot be empty"));
    }

    let opts = CompileOptions {
        name: name.to_string(),
        prompt: prompt_text,
        master_provider: "claude".to_string(),
        available_providers: vec![],
    };

    let def = compile(opts)?;

    // Save to .orc/flows/<name>.json
    let flows_dir = std::env::current_dir()?.join(".orc").join("flows");
    std::fs::create_dir_all(&flows_dir)?;
    let flow_path = flows_dir.join(format!("{}.json", def.name));
    save_flow(&flow_path, &def)?;

    println!("Flow '{}' saved to {}", def.name, flow_path.display());
    println!("Stages:");
    for (i, stage) in def.stages.iter().enumerate() {
        println!(
            "  {}. {} -> {} ({})",
            i + 1,
            stage.intent,
            stage.provider,
            stage.id
        );
    }

    Ok(())
}

fn cmd_init() -> anyhow::Result<()> {
    use orc_core::config::{
        default_config_path, load_config, save_config, AuthStatus, Config, ProviderConfig,
    };
    use orc_core::invoke::provider::Provider;
    use orc_core::invoke::providers::{ClaudeProvider, CodexProvider};

    let config_path = default_config_path()?;

    // Check if config already exists
    if config_path.exists() {
        println!("Config already exists at: {}", config_path.display());
        let cfg = load_config(&config_path)?;
        println!("Master provider: {}", cfg.master_provider);
        println!("Registered providers: {}", cfg.providers.len());
        return Ok(());
    }

    println!("Initializing orc config...");

    // Run doctor on known providers
    let providers: Vec<Box<dyn Provider>> = vec![Box::new(ClaudeProvider), Box::new(CodexProvider)];

    let mut provider_configs = Vec::new();
    let mut master_provider = String::from("claude");

    for provider in &providers {
        let report = provider.detect();
        let status = if report.auth_ok {
            AuthStatus::LoggedIn
        } else if report.installed {
            AuthStatus::LoggedOut
        } else {
            AuthStatus::Unknown
        };

        let label = if report.installed {
            "installed"
        } else {
            "not found"
        };
        let auth_label = if report.auth_ok {
            "authenticated"
        } else {
            "not authenticated"
        };
        println!("  {} -> {} / {}", report.name, label, auth_label);

        if report.installed && master_provider.is_empty() {
            master_provider = report.name.clone();
        }

        provider_configs.push(ProviderConfig {
            name: report.name.clone(),
            binary: provider.binary().to_string(),
            installed: report.installed,
            auth_status: status,
        });
    }

    let now = chrono::Utc::now();
    let cfg = Config {
        master_provider,
        providers: provider_configs,
        routing: Some(orc_core::config::default_routing()),
        created_at: now,
        updated_at: now,
    };

    save_config(&config_path, &cfg)?;
    println!("Config saved to: {}", config_path.display());

    Ok(())
}

fn cmd_providers_doctor() -> anyhow::Result<()> {
    use orc_core::invoke::provider::Provider;
    use orc_core::invoke::providers::{ClaudeProvider, CodexProvider};

    let providers: Vec<(&str, Box<dyn Provider>)> = vec![
        ("claude", Box::new(ClaudeProvider)),
        ("codex", Box::new(CodexProvider)),
    ];

    // Also check additional providers by binary detection
    let extra_binaries = ["gemini", "copilot", "opencode", "kimi", "amp", "droid"];

    println!(
        "{:<12} {:<12} {:<30} {}",
        "PROVIDER", "STATUS", "BINARY PATH", "AUTH"
    );
    println!("{}", "-".repeat(70));

    for (name, provider) in &providers {
        let report = provider.detect();
        let status = if report.installed {
            "installed"
        } else {
            "missing"
        };
        let path = report.binary_path.as_deref().unwrap_or("-");
        let auth = if report.auth_ok {
            "ok"
        } else if report.installed {
            "not authenticated"
        } else {
            "-"
        };
        let error_note = report
            .error
            .as_deref()
            .map(|e| format!(" ({})", e))
            .unwrap_or_default();
        println!(
            "{:<12} {:<12} {:<30} {}{}",
            name, status, path, auth, error_note
        );
    }

    // Extra providers: just check if binary is on PATH
    for name in &extra_binaries {
        let path = which_binary(name);
        let (status, path_str, auth) = if let Some(p) = &path {
            ("installed", p.as_str(), "unknown")
        } else {
            ("missing", "-", "-")
        };
        println!("{:<12} {:<12} {:<30} {}", name, status, path_str, auth);
    }

    Ok(())
}

/// Check if a binary exists on PATH; returns the path string if found.
fn which_binary(name: &str) -> Option<String> {
    std::process::Command::new("which")
        .arg(name)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Parse a tool name string into a SessionSource.
fn parse_source(tool: &str) -> anyhow::Result<orc_core::session::types::SessionSource> {
    use orc_core::session::types::SessionSource;
    match tool {
        "claude" => Ok(SessionSource::Claude),
        "codex" => Ok(SessionSource::Codex),
        "gemini" => Ok(SessionSource::Gemini),
        "copilot" => Ok(SessionSource::Copilot),
        "opencode" => Ok(SessionSource::OpenCode),
        "kimi" => Ok(SessionSource::Kimi),
        "amp" => Ok(SessionSource::Amp),
        "droid" => Ok(SessionSource::Droid),
        other => Err(anyhow::anyhow!("unknown tool: '{}'. Valid tools: claude, codex, gemini, copilot, opencode, kimi, amp, droid", other)),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use orc_core::session::types::{SessionSource, UnifiedSession};
    use std::path::PathBuf;

    #[test]
    fn test_hub_subcommand_exists() {
        let result = Cli::try_parse_from(["orc", "hub"]);
        assert!(result.is_ok(), "orc hub should be a valid subcommand");
    }

    fn make_session(
        id: &str,
        source: SessionSource,
        cwd: &str,
        summary: &str,
        updated_at: chrono::DateTime<chrono::Utc>,
    ) -> UnifiedSession {
        UnifiedSession {
            id: id.to_string(),
            source,
            cwd: PathBuf::from(cwd),
            repo: None,
            summary: summary.to_string(),
            updated_at,
            created_at: updated_at,
            message_count: 2,
            files_modified: vec![],
            original_path: PathBuf::from(format!("/tmp/{}.jsonl", id)),
        }
    }

    // Test 1: orc --help exits 0
    #[test]
    fn test_help_exits_zero() {
        let cargo_manifest = env!("CARGO_MANIFEST_DIR");
        // Build the binary first (result intentionally ignored; we check binary existence below)
        let _status = std::process::Command::new("cargo")
            .args(["build", "--manifest-path"])
            .arg(format!("{}/../../Cargo.toml", cargo_manifest))
            .status();

        // If build fails (e.g. in restricted env), skip by looking for pre-built binary
        let binary_path = PathBuf::from(cargo_manifest).join("../../target/debug/orc");

        if !binary_path.exists() {
            // Binary not built yet — skip this test
            eprintln!(
                "Skipping test_help_exits_zero: binary not found at {:?}",
                binary_path
            );
            return;
        }

        let output = std::process::Command::new(&binary_path)
            .arg("--help")
            .output()
            .expect("failed to run orc --help");

        assert!(
            output.status.success(),
            "orc --help should exit 0, got: {:?}\nstdout: {}\nstderr: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let help_text = String::from_utf8_lossy(&output.stdout);
        assert!(
            help_text.contains("orc") || help_text.contains("Session hub"),
            "Help output should mention 'orc' or 'Session hub', got: {}",
            help_text
        );
    }

    // Test 2: format_session_row produces expected columns
    #[test]
    fn test_list_text_format() {
        let now = chrono::Utc.with_ymd_and_hms(2026, 2, 20, 10, 0, 0).unwrap();
        let session = make_session(
            "abc12345-0000-0000-0000-000000000001",
            SessionSource::Claude,
            "/Users/me/projects/myapp",
            "Fix the auth bug",
            now,
        );

        let row = format_session_row(&session);

        // Check source is present
        assert!(
            row.contains("claude"),
            "Row should contain source 'claude', got: {}",
            row
        );
        // Check short ID (first 8 chars)
        assert!(
            row.contains("abc12345"),
            "Row should contain short ID 'abc12345', got: {}",
            row
        );
        // Check last 2 cwd segments
        assert!(
            row.contains("projects"),
            "Row should contain 'projects' cwd segment, got: {}",
            row
        );
        assert!(
            row.contains("myapp"),
            "Row should contain 'myapp' cwd segment, got: {}",
            row
        );
        // Check summary
        assert!(
            row.contains("Fix the auth bug"),
            "Row should contain summary, got: {}",
            row
        );
    }

    // Test 3: find_session_by_prefix works correctly
    #[test]
    fn test_id_prefix_match() {
        let now = chrono::Utc::now();
        let sessions = vec![
            make_session(
                "abc12345-0000-0000-0000-000000000001",
                SessionSource::Claude,
                "/tmp/a",
                "task A",
                now,
            ),
            make_session(
                "def67890-0000-0000-0000-000000000002",
                SessionSource::Codex,
                "/tmp/b",
                "task B",
                now,
            ),
            make_session(
                "abc99999-0000-0000-0000-000000000003",
                SessionSource::Gemini,
                "/tmp/c",
                "task C",
                now,
            ),
        ];

        // Exact match
        let found = find_session_by_prefix(&sessions, "abc12345-0000-0000-0000-000000000001");
        assert!(found.is_some(), "Should find by exact ID");
        assert_eq!(found.unwrap().id, "abc12345-0000-0000-0000-000000000001");

        // Unique prefix
        let found = find_session_by_prefix(&sessions, "def");
        assert!(found.is_some(), "Should find by unique prefix 'def'");
        assert_eq!(found.unwrap().id, "def67890-0000-0000-0000-000000000002");

        // Ambiguous prefix (two sessions start with "abc") -> should return None
        let found = find_session_by_prefix(&sessions, "abc");
        assert!(found.is_none(), "Ambiguous prefix should return None");

        // Not found
        let found = find_session_by_prefix(&sessions, "xyz");
        assert!(found.is_none(), "Non-existent prefix should return None");
    }

    // Test 4: time_ago formats correctly
    #[test]
    fn test_time_ago() {
        let now = chrono::Utc::now();

        // Just now (< 60 seconds)
        let just_now = now - chrono::Duration::seconds(30);
        assert_eq!(time_ago(just_now), "just now");

        // Minutes ago
        let five_min = now - chrono::Duration::seconds(5 * 60 + 10);
        assert_eq!(time_ago(five_min), "5m ago");

        // Hours ago
        let two_hours = now - chrono::Duration::seconds(2 * 3600 + 10);
        assert_eq!(time_ago(two_hours), "2h ago");

        // Days ago
        let three_days = now - chrono::Duration::seconds(3 * 86400 + 10);
        assert_eq!(time_ago(three_days), "3d ago");

        // Exactly 1 hour (60 minutes)
        let one_hour = now - chrono::Duration::seconds(3600 + 10);
        assert_eq!(time_ago(one_hour), "1h ago");

        // Exactly 59 minutes
        let fifty_nine_min = now - chrono::Duration::seconds(59 * 60 + 10);
        assert_eq!(time_ago(fifty_nine_min), "59m ago");
    }

    // Test 5: cmd_resume would write SCP to the correct path
    #[test]
    fn test_cmd_resume_would_write_scp_path() {
        use orc_core::session::scp::default_scp_path;
        use std::path::PathBuf;
        let cwd = PathBuf::from("/Users/me/myproject");
        let path = default_scp_path(&cwd);
        assert!(path.to_string_lossy().ends_with(".orc/context.json"));
        assert!(path.to_string_lossy().contains("myproject"));
    }
}
