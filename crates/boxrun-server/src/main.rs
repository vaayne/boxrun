mod app;
mod box_manager;
mod cli;
mod events;
mod routes;
mod store;
mod ui;

use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;

#[derive(Parser)]
#[command(
    name = "boxrun",
    about = "BoxRun - Local execution platform built on BoxLite",
    version = concat!(env!("CARGO_PKG_VERSION"), " (rust)"),
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the BoxRun server
    Serve {
        /// Bind host
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Bind port
        #[arg(long, default_value_t = 9090)]
        port: u16,
        /// Unix socket path (overrides host/port)
        #[arg(long)]
        socket: Option<String>,
    },
    /// Create a new box
    Create {
        /// Container image or alias (e.g. ubuntu, python, node)
        #[arg(default_value = "default")]
        image: String,
        /// Box name
        #[arg(short, long)]
        name: Option<String>,
        /// CPU cores
        #[arg(long, default_value_t = 2)]
        cpu: i64,
        /// Memory in MB
        #[arg(short, long, default_value_t = 1024)]
        memory: i64,
        /// Disk size in GB
        #[arg(short, long, default_value_t = 8)]
        disk: i64,
        /// Enable networking
        #[arg(long, default_value_t = false)]
        network: bool,
        /// Volume mount /host:/guest[:ro]
        #[arg(short, long)]
        volume: Vec<String>,
    },
    /// List boxes
    Ls {
        /// Filter by status
        #[arg(short, long)]
        status: Option<String>,
    },
    /// Stop a running box
    Stop {
        /// Box ID or name
        box_id: String,
    },
    /// Start a stopped box
    Start {
        /// Box ID or name
        box_id: String,
    },
    /// Remove a box
    Rm {
        /// Box ID or name
        box_id: String,
        /// Force remove running box
        #[arg(short, long, default_value_t = false)]
        force: bool,
    },
    /// Execute a command in a box
    Exec {
        /// Box ID or name
        box_id: String,
        /// Command and arguments
        #[arg(trailing_var_arg = true, required = true)]
        cmd: Vec<String>,
        /// Return immediately
        #[arg(short, long, default_value_t = false)]
        detach: bool,
        /// Timeout in seconds
        #[arg(short, long)]
        timeout: Option<i64>,
    },
    /// Attach an interactive terminal to a box
    Attach {
        /// Box ID or name
        box_id: String,
        /// Shell to use
        #[arg(long, default_value = "/bin/bash")]
        shell: String,
    },
    /// Create a box and attach an interactive terminal
    Shell {
        /// Container image or alias (e.g. ubuntu, python, node)
        #[arg(default_value = "default")]
        image: String,
        /// Box name
        #[arg(short, long)]
        name: Option<String>,
        /// CPU cores
        #[arg(long, default_value_t = 2)]
        cpu: i64,
        /// Memory in MB
        #[arg(short, long, default_value_t = 1024)]
        memory: i64,
        /// Disk size in GB
        #[arg(short, long, default_value_t = 8)]
        disk: i64,
        /// Shell to use
        #[arg(long, default_value = "/bin/bash")]
        shell: String,
        /// Volume mount /host:/guest[:ro]
        #[arg(short, long)]
        volume: Vec<String>,
    },
    /// Copy files between host and box
    Cp {
        /// Source (LOCAL or BOX:PATH)
        src: String,
        /// Destination (LOCAL or BOX:PATH)
        dst: String,
    },
    /// Run a command in an ephemeral box (create + exec + rm)
    Run {
        /// Container image or alias
        image: String,
        /// Command and arguments
        #[arg(trailing_var_arg = true, required = true)]
        cmd: Vec<String>,
        /// Timeout in seconds
        #[arg(short, long)]
        timeout: Option<i64>,
        /// Disk size in GB
        #[arg(short, long, default_value_t = 8)]
        disk: i64,
    },
    /// Garbage collect old stopped boxes
    Gc {
        /// Remove boxes stopped for longer than N seconds
        #[arg(long, default_value_t = 3600)]
        older_than: i64,
    },
    /// List recommended images and aliases
    Images,
    /// Generate shell completions
    Completion {
        /// Shell type
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Serve { host, port, socket } => {
            cli::serve(&host, port, socket.as_deref()).await;
        }
        Commands::Create {
            image,
            name,
            cpu,
            memory,
            disk,
            network,
            volume,
        } => {
            cli::create(&image, name.as_deref(), cpu, memory, disk, network, &volume).await;
        }
        Commands::Ls { status } => {
            cli::ls(status.as_deref()).await;
        }
        Commands::Stop { box_id } => {
            cli::stop(&box_id).await;
        }
        Commands::Start { box_id } => {
            cli::start(&box_id).await;
        }
        Commands::Rm { box_id, force } => {
            cli::rm(&box_id, force).await;
        }
        Commands::Exec {
            box_id,
            cmd,
            detach,
            timeout,
        } => {
            cli::exec_cmd(&box_id, &cmd, detach, timeout).await;
        }
        Commands::Attach { box_id, shell } => {
            cli::attach(&box_id, &shell).await;
        }
        Commands::Shell {
            image,
            name,
            cpu,
            memory,
            disk,
            shell,
            volume,
        } => {
            cli::shell(&image, name.as_deref(), cpu, memory, disk, &shell, &volume).await;
        }
        Commands::Cp { src, dst } => {
            cli::cp(&src, &dst).await;
        }
        Commands::Run {
            image,
            cmd,
            timeout,
            disk,
        } => {
            cli::run_ephemeral(&image, &cmd, timeout, disk).await;
        }
        Commands::Gc { older_than } => {
            cli::gc(older_than).await;
        }
        Commands::Images => {
            cli::images();
        }
        Commands::Completion { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "boxrun", &mut std::io::stdout());
        }
    }
}
