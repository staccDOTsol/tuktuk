use std::{path, sync::Arc, time::Duration};

use anyhow::Result;
use clap::Parser;
use settings::Settings;
use solana_client::nonblocking::{pubsub_client::PubsubClient, rpc_client::RpcClient};
use solana_sdk::{commitment_config::CommitmentConfig, signature::Keypair, signer::EncodableKey};
use solana_transaction_utils::queue::{create_transaction_queue_handles, TransactionQueueArgs};
use task_completion_processor::process_task_completions;
use task_context::TaskContext;
use task_processor::process_tasks;
use task_queue::{create_task_queue, TaskQueueArgs};
use tokio::time::interval;
use tokio_graceful_shutdown::{SubsystemBuilder, Toplevel};
use tracing_subscriber::{fmt::format::FmtSpan, layer::SubscriberExt, util::SubscriberInitExt};
use transaction::TransactionSenderSubsystem;
use tuktuk_sdk::{prelude::*, watcher::PubsubTracker};
use watchers::{args::WatcherArgs, task_queues::get_and_watch_task_queues};

pub mod settings;
pub mod task_completion_processor;
pub mod task_context;
pub mod task_processor;
pub mod task_queue;
pub mod transaction;
pub mod watchers;

#[derive(Debug, clap::Parser)]
#[clap(version = env!("CARGO_PKG_VERSION"))]
pub struct Cli {
    /// Optional configuration file to use. If present the toml file at the
    /// given path will be loaded. Environment variables can override the
    /// settings in the given file.
    #[clap(short = 'c')]
    pub config: Option<path::PathBuf>,
}

impl Cli {
    pub async fn run(&self) -> Result<()> {
        let settings = Settings::new(self.config.as_ref())?;
        tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new(&settings.log))
            .with(tracing_subscriber::fmt::layer().with_span_events(FmtSpan::CLOSE))
            .init();

        let solana_url = settings.rpc_url;
        let solana_ws_url = settings.rpc_ws_url;

        // Create a non-blocking RPC client
        let rpc_client = Arc::new(RpcClient::new_with_commitment(
            solana_url.clone(),
            CommitmentConfig::confirmed(),
        ));
        let payer_path = settings.key_path;
        let payer = Arc::new(
            Keypair::read_from_file(payer_path).map_err(|e| anyhow::anyhow!(e.to_string()))?,
        );

        // Create a non-blocking PubSub client
        let pubsub_client = Arc::new(PubsubClient::new(&solana_ws_url).await?);
        let pubsub_tracker = Arc::new(PubsubTracker::new(
            Arc::clone(&rpc_client),
            pubsub_client,
            Duration::from_secs(60),
        ));

        let now_rx = clock::track(Arc::clone(&rpc_client), Arc::clone(&pubsub_tracker)).await?;

        let (tasks, task_queue) = create_task_queue(TaskQueueArgs {
            channel_capacity: 100,
            now: now_rx.clone(),
        })
        .await;
        let task_queue_arc = Arc::new(task_queue);

        let watcher_args = WatcherArgs {
            max_retries: settings.max_retries,
            rpc_client: rpc_client.clone(),
            pubsub_tracker: pubsub_tracker.clone(),
            now: now_rx.clone(),
            task_queue: task_queue_arc.clone(),
            min_crank_fee: settings.min_crank_fee,
        };

        let handles = create_transaction_queue_handles(1000);
        let tx_sender = handles.sender.clone();
        let completion_receiver = handles.result_receiver;

        let task_context = Arc::new(TaskContext {
            tx_sender,
            task_queue: task_queue_arc.clone(),
            now_rx: now_rx.clone(),
            rpc_client: rpc_client.clone(),
            payer: payer.clone(),
        });

        let pubsub_repoll = settings.pubsub_repoll;
        Toplevel::new(move |top_level| async move {
            let watcher_args_clone = watcher_args.clone();
            top_level.start(SubsystemBuilder::new("task-queue-watcher", {
                move |handle| get_and_watch_task_queues(watcher_args_clone, handle)
            }));
            let task_context_clone = task_context.clone();
            top_level.start(SubsystemBuilder::new("transaction-queue", {
                move |handle| {
                    TransactionSenderSubsystem::new(TransactionQueueArgs {
                        rpc_client,
                        ws_url: solana_ws_url.clone(),
                        payer,
                        batch_duration: Duration::from_secs(1),
                        receiver: handles.receiver,
                        result_sender: handles.result_sender,
                        max_sol_fee: settings.max_sol_fee,
                    })
                    .run(handle)
                }
            }));
            top_level.start(SubsystemBuilder::new("task-processor", {
                move |handle| process_tasks(Box::new(tasks), task_context_clone, handle)
            }));
            let task_context_clone = task_context.clone();
            top_level.start(SubsystemBuilder::new("completion-processor", {
                move |handle| {
                    process_task_completions(completion_receiver, task_context_clone, handle)
                }
            }));
            // Poll RPC for changes to pubsub keys every 30 seconds
            top_level.start(SubsystemBuilder::new("pubsub-tracker", {
                move |handle| async move {
                    let mut interval = interval(pubsub_repoll);
                    loop {
                        tokio::select! {
                            _ = interval.tick() => {
                                if let Err(e) = pubsub_tracker.check_for_changes().await {
                                    tracing::error!("Error checking for changes: {:?}", e);
                                }
                            }
                            _ = handle.on_shutdown_requested() => {
                                tracing::info!("Shutdown requested, exiting pubsub-tracker");
                                break;
                            }
                        }
                    }
                    anyhow::Ok(())
                }
            }));
        })
        .catch_signals()
        .handle_shutdown_requests(Duration::from_millis(5000))
        .await
        .map_err(Into::into)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    cli.run().await
}
