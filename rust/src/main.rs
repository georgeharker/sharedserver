use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;

mod cli;
use cli::{commands, output, watcher};

const LONG_ABOUT: &str = "\
sharedserver - Manage shared servers with reference counting

A lightweight tool for managing long-running server processes that can be shared 
between multiple clients. Servers are automatically started when needed and shut 
down gracefully when no clients remain (after a configurable grace period).

EVERYDAY COMMANDS:
  use         Attach to a server (starts if needed)
  unuse       Detach from a server
  up          Bring up every server in a profile (from the config)
  down        Release every server in a profile
  config      Register/inspect server defs and profiles (self-define)
  list        Show all running servers
  info        Get detailed server information
  check       Check if server is running
  completion  Generate shell completions

ADMIN COMMANDS:
  admin       Low-level server operations (start, stop, incref, decref, debug, doctor, kill)
  
See 'sharedserver <command> --help' for detailed command information.
See 'sharedserver admin --help' for administrative operations.
";

#[derive(Parser)]
#[command(name = "sharedserver")]
#[command(version, author)]
#[command(about = "Manage shared servers with reference counting")]
#[command(long_about = LONG_ABOUT)]
#[command(arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Use a server (start if not running, then attach)
    Use {
        /// Server name
        name: String,
        /// Grace period before shutdown when refcount reaches 0 (e.g., "5m", "1h", "30s")
        #[arg(long, default_value = "5m")]
        grace_period: String,
        /// Optional client metadata
        #[arg(long)]
        metadata: Option<String>,
        /// Client PID (defaults to parent process - the caller)
        #[arg(long)]
        pid: Option<i32>,
        /// Environment variables in KEY=VALUE format (can be specified multiple times)
        #[arg(long = "env", value_name = "KEY=VALUE")]
        env_vars: Vec<String>,
        /// Optional log file path for server stdout/stderr
        #[arg(long)]
        log_file: Option<String>,
        /// Server command and arguments (required if server not running)
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Detach from a server (decrement reference count)
    Unuse {
        /// Server name
        name: String,
        /// Client PID (defaults to parent process - the caller)
        #[arg(long)]
        pid: Option<i32>,
    },
    /// Bring up every server in a profile, resolving each def from the config.
    ///
    /// A "profile" is a named set of servers in servers.json; a host identity
    /// (opencode, claude, pi, neovim) is just a reserved profile name. Servers in
    /// no profile are universal and come up for any profile.
    Up {
        /// Profile to bring up (typically your host: opencode, claude, pi, ...)
        #[arg(long)]
        profile: String,
        /// Client PID this bring-up refs (defaults to parent process - the caller)
        #[arg(long)]
        pid: Option<i32>,
        /// Grace period for servers that don't set their own (e.g. "5m", "1h")
        #[arg(long, default_value = "5m")]
        grace_period: String,
        /// Explicit config file, overriding the discovery chain
        #[arg(long)]
        config: Option<String>,
        /// Directory to resolve the per-project config from (defaults to cwd)
        #[arg(long)]
        cwd: Option<String>,
        /// Treat a missing profile as normal (bring up only universal servers,
        /// no warning). Plugins asking for their own host profile pass this.
        #[arg(long)]
        profile_optional: bool,
        /// Emit a single JSON object (profile, per-server outcome, warnings) and
        /// nothing else on stdout — for programmatic callers.
        #[arg(long)]
        json: bool,
    },
    /// Release every server in a profile (the inverse of `up`).
    Down {
        /// Profile to release (must match the `up` that brought it up)
        #[arg(long)]
        profile: String,
        /// Client PID whose refs to release (defaults to parent process)
        #[arg(long)]
        pid: Option<i32>,
        /// Detach EVERY client from each server, not just this PID: the
        /// refcount drops to 0 and each server enters its grace period. The
        /// gentle "force" — no signals are sent; the watcher's grace countdown
        /// still owns the actual shutdown. Useful when `down` can't release
        /// because the refs belong to other (possibly dead) PIDs.
        #[arg(long, conflicts_with = "pid")]
        detach_all: bool,
        /// Explicit config file, overriding the discovery chain
        #[arg(long)]
        config: Option<String>,
        /// Directory to resolve the per-project config from (defaults to cwd)
        #[arg(long)]
        cwd: Option<String>,
        /// Treat a missing profile as normal (no warning); pair with `up`.
        #[arg(long)]
        profile_optional: bool,
        /// Emit a single JSON object and nothing else on stdout.
        #[arg(long)]
        json: bool,
    },
    /// Edit or inspect the servers.json config (the self-define surface):
    /// register/unregister scoped server defs and tag them into profiles.
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
    /// List all servers
    List {
        /// Output as JSON (for programmatic use)
        #[arg(long)]
        json: bool,
    },
    /// Get detailed server information
    Info {
        /// Server name
        name: String,
        /// Output as JSON (for programmatic use)
        #[arg(long)]
        json: bool,
    },
    /// Check server status
    Check {
        /// Server name
        name: String,
    },
    /// Generate shell completion scripts
    Completion {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },
    /// Administrative commands for low-level server operations
    Admin {
        #[command(subcommand)]
        command: AdminCommands,
    },
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Register (or overwrite) a scoped server def; optionally tag it into profiles
    Register {
        /// Owning scope id (e.g. your plugin or host name)
        #[arg(long)]
        scope: String,
        /// Server name
        name: String,
        /// Profile(s) to add this server to (repeatable, union)
        #[arg(long = "profile")]
        profiles: Vec<String>,
        /// Only register if the name doesn't already exist (still unions profiles)
        #[arg(long)]
        if_absent: bool,
        /// Grace period, e.g. "30m", "1h"
        #[arg(long)]
        grace_period: Option<String>,
        /// Env vars in KEY=VALUE form (repeatable)
        #[arg(long = "env", value_name = "KEY=VALUE")]
        env_vars: Vec<String>,
        /// Log file path for the server's stdout/stderr
        #[arg(long)]
        log_file: Option<String>,
        /// Optional metadata string
        #[arg(long)]
        metadata: Option<String>,
        /// Attach-only (no command needed)
        #[arg(long)]
        lazy: bool,
        /// Config file to edit (default: discovered, else the global one)
        #[arg(long)]
        config: Option<String>,
        /// Server command and arguments
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Remove a scope's server def(s): a specific name, or all of them
    Unregister {
        /// Owning scope id
        #[arg(long)]
        scope: String,
        /// Server name (omit to remove every server owned by this scope)
        name: Option<String>,
        /// Config file to edit (default: discovered, else the global one)
        #[arg(long)]
        config: Option<String>,
    },
    /// Look up one server: its def and the profiles it belongs to
    Lookup {
        /// Server name
        name: String,
        /// Config file (default: discovered)
        #[arg(long)]
        config: Option<String>,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },
    /// List registered servers and profiles
    List {
        /// Config file (default: discovered)
        #[arg(long)]
        config: Option<String>,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },
    /// Print the whole config document
    Show {
        /// Config file (default: discovered)
        #[arg(long)]
        config: Option<String>,
        /// Emit compact JSON
        #[arg(long)]
        json: bool,
    },
    /// Check for dangling profile members and structural issues
    Validate {
        /// Config file (default: discovered)
        #[arg(long)]
        config: Option<String>,
    },
    /// Manage profile membership directly
    Profile {
        #[command(subcommand)]
        command: ProfileCommands,
    },
}

#[derive(Subcommand)]
enum ProfileCommands {
    /// Add server(s) to a profile (union; a server need not exist yet)
    Add {
        /// Profile name
        profile: String,
        /// Server name(s)
        #[arg(required = true)]
        names: Vec<String>,
        /// Config file to edit (default: discovered, else the global one)
        #[arg(long)]
        config: Option<String>,
    },
    /// Remove server(s) from a profile (drops the profile if it becomes empty)
    Remove {
        /// Profile name
        profile: String,
        /// Server name(s)
        #[arg(required = true)]
        names: Vec<String>,
        /// Config file to edit (default: discovered, else the global one)
        #[arg(long)]
        config: Option<String>,
    },
}

#[derive(Subcommand)]
enum AdminCommands {
    /// Start a new server with NO clients (low-level - use 'sharedserver use' instead)
    ///
    /// This creates a server in a "waiting for clients" state (refcount=0).
    /// The server will immediately enter its grace period unless a client
    /// calls 'incref' to attach. Normal users should use 'sharedserver use' instead,
    /// which combines start+incref atomically.
    Start {
        /// Server name
        name: String,
        /// Grace period before shutdown when refcount reaches 0 (e.g., "5m", "1h", "30s")
        #[arg(long, default_value = "5m")]
        grace_period: String,
        /// Environment variables in KEY=VALUE format (can be specified multiple times)
        #[arg(long = "env", value_name = "KEY=VALUE")]
        env_vars: Vec<String>,
        /// Optional log file path for server stdout/stderr
        #[arg(long)]
        log_file: Option<String>,
        /// Server command and arguments
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
    /// Stop a server: SIGTERM, then wait for the watcher to tear it down
    Stop {
        /// Server name
        name: String,
        /// Escalate to SIGKILL if the server doesn't stop within the timeout
        #[arg(long)]
        force: bool,
        /// How long to wait for teardown to converge (e.g. "10s", "1m", "500ms")
        #[arg(long, default_value = "10s")]
        timeout: String,
    },
    /// Increment reference count (low-level - use 'sharedserver use' instead)
    Incref {
        /// Server name
        name: String,
        /// Optional client metadata
        #[arg(long)]
        metadata: Option<String>,
        /// Client PID this reference represents (required - must be a real,
        /// long-lived process; the watcher drops the ref when it dies)
        #[arg(long)]
        pid: i32,
    },
    /// Decrement reference count (low-level - use 'sharedserver unuse' instead)
    Decref {
        /// Server name
        name: String,
        /// Client PID whose reference to release (required)
        #[arg(long)]
        pid: i32,
    },
    /// Show invocation log for debugging
    Debug {
        /// Server name
        name: String,
    },
    /// Validate server state and clean up inconsistencies
    Doctor {
        /// Server name (if omitted, checks all servers)
        name: Option<String>,
    },
    /// Force kill a server and clean up all state
    Kill {
        /// Server name
        name: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Use {
            name,
            grace_period,
            metadata,
            pid,
            env_vars,
            log_file,
            command,
        } => commands::r#use::execute(
            &name,
            &grace_period,
            metadata,
            pid,
            &env_vars,
            log_file.as_deref(),
            &command,
        ),
        Commands::Unuse { name, pid } => commands::unuse::execute(&name, pid),
        Commands::Up {
            profile,
            pid,
            grace_period,
            config,
            cwd,
            profile_optional,
            json,
        } => commands::up::execute(
            &profile,
            pid,
            &grace_period,
            config.as_deref(),
            cwd.as_deref(),
            profile_optional,
            json,
        ),
        Commands::Down {
            profile,
            pid,
            detach_all,
            config,
            cwd,
            profile_optional,
            json,
        } => commands::down::execute(
            &profile,
            pid,
            detach_all,
            config.as_deref(),
            cwd.as_deref(),
            profile_optional,
            json,
        ),
        Commands::Config { command } => match command {
            ConfigCommands::Register {
                scope,
                name,
                profiles,
                if_absent,
                grace_period,
                env_vars,
                log_file,
                metadata,
                lazy,
                config,
                command,
            } => commands::config::register(
                &scope,
                &name,
                &command,
                grace_period.as_deref(),
                &env_vars,
                log_file.as_deref(),
                metadata.as_deref(),
                lazy,
                &profiles,
                if_absent,
                config.as_deref(),
            ),
            ConfigCommands::Unregister {
                scope,
                name,
                config,
            } => commands::config::unregister(&scope, name.as_deref(), config.as_deref()),
            ConfigCommands::Lookup { name, config, json } => {
                commands::config::lookup(&name, config.as_deref(), json)
            }
            ConfigCommands::List { config, json } => {
                commands::config::list(config.as_deref(), json)
            }
            ConfigCommands::Show { config, json } => {
                commands::config::show(config.as_deref(), json)
            }
            ConfigCommands::Validate { config } => commands::config::validate(config.as_deref()),
            ConfigCommands::Profile { command } => match command {
                ProfileCommands::Add {
                    profile,
                    names,
                    config,
                } => commands::config::profile_add(&profile, &names, config.as_deref()),
                ProfileCommands::Remove {
                    profile,
                    names,
                    config,
                } => commands::config::profile_remove(&profile, &names, config.as_deref()),
            },
        },
        Commands::List { json } => commands::list::execute(json),
        Commands::Info { name, json } => commands::info::execute(&name, json),
        Commands::Check { name } => commands::check::execute(&name),
        Commands::Completion { shell } => {
            let mut cmd = Cli::command();
            let bin_name = cmd.get_name().to_string();
            clap_complete::generate(shell, &mut cmd, bin_name, &mut std::io::stdout());
            Ok(())
        }
        Commands::Admin { command } => match command {
            AdminCommands::Start {
                name,
                grace_period,
                env_vars,
                log_file,
                command,
            } => commands::start::execute(
                &name,
                &grace_period,
                &env_vars,
                &command,
                log_file.as_deref(),
            ),
            AdminCommands::Stop {
                name,
                force,
                timeout,
            } => commands::stop::execute(&name, force, &timeout),
            AdminCommands::Incref {
                name,
                metadata,
                pid,
            } => commands::incref::execute(&name, metadata, pid),
            AdminCommands::Decref { name, pid } => commands::decref::execute(&name, pid),
            AdminCommands::Debug { name } => commands::debug::execute(&name, 50),
            AdminCommands::Doctor { name } => commands::doctor::execute(name),
            AdminCommands::Kill { name } => commands::kill::execute(&name),
        },
    }
}
