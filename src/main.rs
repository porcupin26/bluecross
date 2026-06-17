use std::path::Path;

use clap::{Parser, Subcommand};

mod client;
mod clipboard;
mod config;
mod daemon;
mod input_capture;
mod input_inject;
mod protocol;
mod screen;
mod secure;
mod server;

#[derive(Parser)]
#[command(
    name = "bluecross",
    about = "Share keyboard, mouse and clipboard across Linux computers"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the server
    Server {
        /// Path to config file
        #[arg(short, long, default_value = "bluecross.json")]
        config: String,
        /// Run in foreground (don't daemonize)
        #[arg(short, long)]
        foreground: bool,
        /// Enable debug logging
        #[arg(short, long)]
        debug: bool,
    },
    /// Run the client
    Client {
        /// Path to config file
        #[arg(short, long, default_value = "bluecross.json")]
        config: String,
        /// Run in foreground (don't daemonize)
        #[arg(short, long)]
        foreground: bool,
        /// Enable debug logging
        #[arg(short, long)]
        debug: bool,
    },
    /// Control utility (start, stop, restart, status, logs)
    Ctl {
        /// Command to run (start, stop, restart, status, logs)
        action: String,
        /// Additional arguments
        args: Vec<String>,
    },
}

/// Legacy CLI for backward-compat when invoked as bluecross-server / bluecross-client
#[derive(Parser)]
#[command(name = "bluecross")]
struct LegacyArgs {
    /// Path to config file
    #[arg(short, long, default_value = "bluecross.json")]
    config: String,
    /// Run in foreground
    #[arg(short, long)]
    foreground: bool,
    /// Enable debug logging
    #[arg(short, long)]
    debug: bool,
}

fn main() -> anyhow::Result<()> {
    let exe = std::env::args().next().unwrap_or_default();
    let exe_name = Path::new(&exe)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    match exe_name {
        "bluecross-server" => {
            let args = LegacyArgs::parse();
            server::run(&args.config, args.foreground, args.debug)
        }
        "bluecross-client" => {
            let args = LegacyArgs::parse();
            client::run(&args.config, args.foreground, args.debug)
        }
        "bluecrossctl" => {
            let args: Vec<String> = std::env::args().skip(1).collect();
            let command = args.first().map(|s| s.as_str()).unwrap_or("status");
            let rest = if args.len() > 1 { &args[1..] } else { &[] };
            daemon::handle_ctl(command, rest)
        }
        _ => {
            let cli = Cli::parse();
            match cli.command {
                Commands::Server {
                    config,
                    foreground,
                    debug,
                } => server::run(&config, foreground, debug),
                Commands::Client {
                    config,
                    foreground,
                    debug,
                } => client::run(&config, foreground, debug),
                Commands::Ctl { action, args } => daemon::handle_ctl(&action, &args),
            }
        }
    }
}
