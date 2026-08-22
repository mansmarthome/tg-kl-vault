use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use flowerss_bot::{
    bot::{
        runtime::run_bot,
        sender::{NoopSender, TeloxideSender},
        stocks::StockSvc,
    },
    cli::Args,
    config::Config,
    db::{self, repo::Repo},
    feed::fetch::Fetcher,
    preview::{NoopPublisher, TelegraphPublisher},
    scheduler::{Scheduler, SchedulerOptions},
    stock::{StockService, StockWorker, TwOfficialSource, YahooSource},
    tagging::{build_tagger, mcp::McpClient, worker::TagWorker},
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

    // One StockService, shared (Arc) by the bot handlers and the stock worker,
    // so the rate limiter / 429 cooldown / hard-lock state is process-wide.
    let stock: Arc<StockSvc> = Arc::new(StockService::new(
        repo.clone(),
        YahooSource::new(fetcher.client().clone(), config.stock.yahoo_endpoint.clone()),
        Some(TwOfficialSource::new(
            fetcher.client().clone(),
            config.stock.twse_endpoint.clone(),
            config.stock.tpex_endpoint.clone(),
        )),
        config.stock.clone(),
    ));

    // Embedded-replica reads can lag ~60s, so two instances against one Turso DB
    // could each send the daily close report inside that sync window. A
    // distributed lock would be half-correct; an honest warning is cheaper.
    if config.stock.enabled && repo.db().is_remote() {
        tracing::warn!(
            "embedded replica mode (TURSO_DATABASE_URL) is set: run only ONE instance, or a \
             second instance may duplicate each daily stock report within the 60s sync window"
        );
    }

    // Fourth background task: the stock close-report worker. Its own 60s tick
    // and shutdown clone; shares the StockService Arc with the bot handlers.
    // AI commentary reuses the bookmark MCP bridge, and only when both the
    // stock toggle and the bridge are configured.
    let stock_commentary = (config.stock.ai_commentary && config.bookmark.ai.mcp.is_configured())
        .then(|| McpClient::new(fetcher.client().clone(), config.bookmark.ai.mcp.clone()));
    let stock_worker = StockWorker::with_commentary(
        stock.clone(),
        TeloxideSender::new(bot.clone()),
        stock_commentary,
    );
    let stock_worker_rx = shutdown_rx.clone();
    let stock_worker_task =
        tokio::spawn(async move { stock_worker.run_until_shutdown(stock_worker_rx).await });

    let bot_rx = shutdown_rx.clone();
    let bot_task =
        tokio::spawn(async move { run_bot(bot, config, repo, fetcher, stock, bot_rx).await });

    let (scheduler_result, worker_result, stock_worker_result, bot_result) =
        tokio::try_join!(scheduler_task, worker_task, stock_worker_task, bot_task)
            .context("join tasks")?;
    scheduler_result.context("run scheduler")?;
    worker_result.context("run tag worker")?;
    stock_worker_result.context("run stock worker")?;
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
