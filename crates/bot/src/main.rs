use anyhow::Context;
use clap::Parser;
use flowerss_bot::{
    cli::Args,
    config::Config,
    db::{self, repo::Repo},
    feed::fetch::Fetcher,
    preview::NoopPublisher,
    scheduler::{Scheduler, SchedulerOptions},
};
use tracing::info;
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let config = Config::load(args.config.as_deref()).context("load config")?;
    init_tracing(&config.log.level)?;

    info!(dry_run = args.dry_run, sqlite_path = %config.sqlite.path, "flowerss-bot starting");
    println!(
        "config loaded: sqlite_path={} update_interval={} dry_run={}",
        config.sqlite.path, config.update_interval, args.dry_run
    );

    if args.dry_run {
        let pool = db::connect(&config.sqlite.path).await.context("connect sqlite")?;
        let repo = Repo::new(pool);
        let fetcher = Fetcher::new(&config).context("build feed fetcher")?;
        let scheduler = Scheduler::new(
            repo,
            fetcher,
            NoopPublisher,
            config,
            SchedulerOptions { dry_run: true, ..SchedulerOptions::default() },
        );
        scheduler.run_once().await.context("run dry-run scheduler pass")?;
    }

    Ok(())
}

fn init_tracing(level: &str) -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env().or_else(|_| EnvFilter::try_new(level))?;
    fmt().with_env_filter(filter).init();
    Ok(())
}
