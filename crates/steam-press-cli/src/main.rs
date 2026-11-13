mod commands;
mod support;

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use std::process::ExitCode;

use steam_press_core::downloader::DownloaderKind;
use steam_press_core::Platform;

#[derive(Debug, Parser)]
#[command(
    name = "steam-press",
    version,
    about = "Packing tool for Steam games",
    long_about = "Steam Press downloads Steam depots and prepares clean, compressed release archives."
)]
pub struct Cli {
    /// Configuration directory (defaults to the XDG config directory).
    #[arg(long, global = true, value_name = "DIR")]
    pub config_dir: Option<PathBuf>,
    /// Working directory for downloads (defaults to ~/SteamPress).
    #[arg(long, global = true, value_name = "DIR")]
    pub work_dir: Option<PathBuf>,
    /// Output directory for archives.
    #[arg(long, global = true, value_name = "DIR")]
    pub output: Option<PathBuf>,
    /// Print downloader and compressor output.
    #[arg(short, long, global = true)]
    pub verbose: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Check tools, directories and depot data.
    Doctor,
    /// Save Steam credentials in the local encrypted vault.
    Login(LoginArgs),
    /// Show Steam metadata for an app.
    Info(InfoArgs),
    /// Manage game queue files.
    Queue {
        #[command(subcommand)]
        action: QueueAction,
    },
    /// Download and pack games.
    Pack(PackArgs),
    /// Download and pack workshop items.
    Workshop {
        #[command(subcommand)]
        action: WorkshopAction,
    },
    /// Synchronize or inspect depot names.
    Depots {
        #[command(subcommand)]
        action: DepotsAction,
    },
    /// Render release notes.
    Template {
        #[command(subcommand)]
        action: TemplateAction,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum DownloaderArg {
    Steamcmd,
    Depotdownloader,
}

impl From<DownloaderArg> for DownloaderKind {
    fn from(value: DownloaderArg) -> Self {
        match value {
            DownloaderArg::Steamcmd => Self::SteamCmd,
            DownloaderArg::Depotdownloader => Self::DepotDownloader,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OsArg {
    Win64,
    Win32,
    Lin64,
    Lin32,
    Macos,
}

impl From<OsArg> for Platform {
    fn from(value: OsArg) -> Self {
        match value {
            OsArg::Win64 => Platform::Win64,
            OsArg::Win32 => Platform::Win32,
            OsArg::Lin64 => Platform::Linux64,
            OsArg::Lin32 => Platform::Linux32,
            OsArg::Macos => Platform::Mac,
        }
    }
}

#[derive(Debug, clap::Args)]
pub struct LoginArgs {
    #[arg(long)]
    pub username: Option<String>,
    /// Read the password from standard input instead of prompting.
    #[arg(long)]
    pub password_stdin: bool,
}

#[derive(Debug, clap::Args)]
pub struct InfoArgs {
    pub appid: String,
    #[arg(long, default_value = "public")]
    pub branch: String,
}

#[derive(Debug, Subcommand)]
pub enum QueueAction {
    List {
        #[arg(long)]
        file: Option<PathBuf>,
    },
    Add {
        appid: String,
        #[arg(long, value_enum, default_value = "lin64")]
        os: OsArg,
        #[arg(long, default_value = "public")]
        branch: String,
        #[arg(long)]
        branch_password: Option<String>,
        #[arg(long)]
        file: Option<PathBuf>,
    },
    Remove {
        index: usize,
        #[arg(long)]
        file: Option<PathBuf>,
    },
    Move {
        from: usize,
        to: usize,
        #[arg(long)]
        file: Option<PathBuf>,
    },
    Clear {
        #[arg(long)]
        file: Option<PathBuf>,
    },
    Import {
        source: PathBuf,
        #[arg(long)]
        file: Option<PathBuf>,
    },
    Export {
        destination: PathBuf,
        #[arg(long)]
        file: Option<PathBuf>,
    },
}

#[derive(Debug, clap::Args)]
pub struct PackArgs {
    #[arg(long = "queue", value_name = "FILE")]
    pub queue_files: Vec<PathBuf>,
    #[arg(long)]
    pub appid: Option<String>,
    #[arg(long, value_enum, default_value = "lin64")]
    pub os: OsArg,
    #[arg(long, default_value = "public")]
    pub branch: String,
    #[arg(long)]
    pub branch_password: Option<String>,
    #[arg(long, value_enum)]
    pub downloader: Option<DownloaderArg>,
    /// `7z`, `rar` or `none`.
    #[arg(long)]
    pub compressor: Option<String>,
    #[arg(long)]
    pub no_compress: bool,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub anonymous: bool,
}

#[derive(Debug, Subcommand)]
pub enum WorkshopAction {
    Add {
        appid: String,
        workshop_id: String,
        #[arg(long)]
        file: Option<PathBuf>,
    },
    List {
        #[arg(long)]
        file: Option<PathBuf>,
    },
    Import {
        source: PathBuf,
        #[arg(long)]
        file: Option<PathBuf>,
    },
    Export {
        destination: PathBuf,
        #[arg(long)]
        file: Option<PathBuf>,
    },
    Run {
        #[arg(long = "queue", value_name = "FILE")]
        queue_files: Vec<PathBuf>,
        #[arg(long, value_enum)]
        downloader: Option<DownloaderArg>,
        #[arg(long)]
        compressor: Option<String>,
        #[arg(long)]
        no_compress: bool,
        #[arg(long)]
        anonymous: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum DepotsAction {
    Sync,
    Lookup { depot_id: String },
    List {
        #[arg(long)]
        search: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum TemplateAction {
    Render {
        /// JSON file containing a TemplateData object.
        job: PathBuf,
        template: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match commands::dispatch(cli) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("steam-press : {message}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
        let matches = Cli::try_parse_from(["steam-press", "depots", "lookup", "10"]).unwrap();
        assert!(matches!(matches.command, Command::Depots { .. }));
    }

    #[test]
    fn pack_defaults_to_linux64() {
        let matches = Cli::try_parse_from(["steam-press", "pack", "--appid", "10"]).unwrap();
        match matches.command {
            Command::Pack(args) => {
                assert!(matches!(args.os, OsArg::Lin64));
                assert_eq!(args.branch, "public");
            }
            _ => panic!("expected pack"),
        }
    }
}
