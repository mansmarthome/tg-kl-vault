use anyhow::Context;
use clap::Parser;
use flowerss_bot::{
    bot::{
        runtime::run_bot,
        sender::{NoopSender, TeloxideSender},
    },
    cli::Args,
    config::Config,
    db::{self, repo::Repo},
    feed::fetch::Fetcher,
    preview::{NoopPublisher, TelegraphPublisher},
    scheduler::{Scheduler, SchedulerOptions},
    tagging::{build_tagger, worker::TagWorker},
};
use teloxide::Bot;
use tokio::sync::watch;
use tracing::info;
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let config = Config::load(args.config.as_deref()).context("load config")?;
    init_tracing(&config.log.level)?;

    info!(dry_run = args.dry_run, sqlite_path = %config.sqlite.path, "tg-kl-vault starting");
    println!(
        "config loaded: sqlite_path={} update_interval={} dry_run={}",
        config.sqlite.path, config.update_interval, args.dry_run
    );

    let pool = db::connect(&config.sqlite.path).await.context("connect sqlite")?;
    let repo = Repo::new(pool);
    let fetcher = Fetcher::new(&config).context("build feed fetcher")?;

    if args.dry_run {
        let scheduler = Scheduler::new(
            repo,
            fetcher,
            NoopPublisher,
            NoopSender,
            config,
            SchedulerOptions { dry_run: true, ..SchedulerOptions::default() },
        );
        scheduler.run_once().await.context("run dry-run scheduler pass")?;
        return Ok(());
    }

    if config.bot_token.is_empty() {
        anyhow::bail!("bot_token is required unless --dry-run is used");
    }
    let bot = Bot::new(config.bot_token.clone());

    let scheduler = Scheduler::new(
        repo.clone(),
        fetcher.clone(),
        TelegraphPublisher::new(&config.telegraph_token),
        TeloxideSender::new(bot.clone()),
        config.clone(),
        SchedulerOptions { dry_run: false, ..SchedulerOptions::default() },
    );

    // Sanctioned deviation D7: stop polling and finish in-flight sends on
    // SIGINT/SIGTERM instead of the Go original's immediate `os.Exit(0)`.
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let signal_task = tokio::spawn(async move {
        wait_for_shutdown_signal().await;
        info!("shutdown signal received, stopping scheduler and bot");
        let _ = shutdown_tx.send(true);
    });

    let scheduler_rx = shutdown_rx.clone();
    let scheduler_task = tokio::spawn(async move { scheduler.run_until_shutdown(scheduler_rx).await });

    // Background tag worker: a third writer on its own shutdown clone. Skipped
    // entirely for --dry-run (that path returns above).
    let tagger = build_tagger(&config);
    let meter_quota = tagger.is_gemini();
    let worker = TagWorker::new(
        repo.clone(),
        tagger,
        TeloxideSender::new(bot.clone()),
        config.clone(),
        fetcher.client().clone(),
        meter_quota,
    );
    let worker_rx = shutdown_rx.clone();
    let worker_task = tokio::spawn(async move { worker.run_until_shutdown(worker_rx).await });

    let bot_rx = shutdown_rx.clone();
    let bot_task = tokio::spawn(async move { run_bot(bot, config, repo, fetcher, bot_rx).await });

    let (scheduler_result, worker_result, bot_result) =
        tokio::try_join!(scheduler_task, worker_task, bot_task).context("join tasks")?;
    scheduler_result.context("run scheduler")?;
    worker_result.context("run tag worker")?;
    bot_result.context("run bot")?;
    signal_task.abort();
    Ok(())
}

async fn wait_for_shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sigterm) => {
                sigterm.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

fn init_tracing(level: &str) -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env().or_else(|_| EnvFilter::try_new(level))?;
    fmt().with_env_filter(filter).init();
    Ok(())
}
