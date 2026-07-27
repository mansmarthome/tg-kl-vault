use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Clone, Parser)]
#[command(name = "tg-kl-vault")]
#[command(about = "Self-hosted Telegram RSS vault bot; flowerss-bot compatible")]
pub struct Args {
    /// Path to config TOML file.
    #[arg(short = 'c', long = "config")]
    pub config: Option<PathBuf>,

    /// Run scheduler/fetch/dedup logic without DB writes or Telegram sends.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
}
