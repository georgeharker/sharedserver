use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;

mod commands;
mod output;
mod watcher;

const LONG_ABOUT: &str = "\
sharedserver - Manage shared servers with reference counting

A lightweight tool for managing long-running server processes that can be shared 
between multiple clients. Servers are automatically started when needed and shut 
down gracefully when no clients remain (after a configurable grace period).

EVERYDAY COMMANDS:
  use         Attach to a server (starts if needed)
  unuse       Detach from a server
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
    /// List all servers
    List,
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
enum AdminCommands {
    /// Start a new server with NO clients (low-level - use 'serverctl use' instead)
    ///
    /// This creates a server in a "waiting for clients" state (refcount=0).
    /// The server will immediately enter its grace period unless a client
    /// calls 'incref' to attach. Normal users should use 'serverctl use' instead,
    /// which combines start+incref atomically.
    Start {
        /// Server name
        name: String,
        /// Grace period before shutdown when refcount reaches 0 (e.g., "5m", "1h", "30s")
        #[arg(long, default_value = "5m")]
        grace_period: String,
        /// Server command and arguments
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
    /// Stop server immediately (emergency use)
    Stop {
        /// Server name
        name: String,
        /// Force kill if server doesn't stop gracefully
        #[arg(long)]
        force: bool,
    },
    /// Increment reference count (low-level - use 'serverctl use' instead)
    Incref {
        /// Server name
        name: String,
        /// Optional client metadata
        #[arg(long)]
        metadata: Option<String>,
        /// Client PID (defaults to current process)
        #[arg(long)]
        pid: Option<i32>,
    },
    /// Decrement reference count (low-level - use 'serverctl unuse' instead)
    Decref {
        /// Server name
        name: String,
        /// Client PID (defaults to current process)
        #[arg(long)]
        pid: Option<i32>,
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
            command,
        } => commands::r#use::execute(&name, &grace_period, metadata, pid, &command),
        Commands::Unuse { name, pid } => commands::unuse::execute(&name, pid),
        Commands::List => commands::list::execute(),
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
                command,
            } => commands::start::execute(&name, &grace_period, &command),
            AdminCommands::Stop { name, force } => commands::stop::execute(&name, force),
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
