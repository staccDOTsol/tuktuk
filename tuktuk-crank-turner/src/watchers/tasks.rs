use futures::TryStreamExt;
use solana_sdk::pubkey::Pubkey;
use tokio_graceful_shutdown::SubsystemHandle;
use tracing::info;
use tuktuk::{task, types::TriggerV0, TaskV0};
use tuktuk_sdk::{prelude::*, tuktuk::TaskQueueV0};

use super::args::WatcherArgs;
use crate::task_queue::TimedTask;

pub async fn get_and_watch_tasks(
    task_queue_key: Pubkey,
    task_queue_account: TaskQueueV0,
    args: WatcherArgs,
    handle: SubsystemHandle,
) -> anyhow::Result<()> {
    info!(?task_queue_key, "watching tasks for queue");
    let WatcherArgs {
        rpc_client,
        pubsub_tracker,
        task_queue,
        now,
        ..
    } = args;
    let task_keys = task::keys(&task_queue_key, &task_queue_account)?;
    let tasks = rpc_client.anchor_accounts::<TaskV0>(&task_keys).await?;

    let task_queue = task_queue.clone();
    let now = now.clone();
    let rpc_client = rpc_client.clone();
    let (stream, _) = task::on_new(
        rpc_client.as_ref(),
        pubsub_tracker.as_ref(),
        &task_queue_key,
        &task_queue_account,
    )
    .await?;

    for (task_key, account) in tasks {
        let task = match account {
            Some(t) if t.crank_reward >= args.min_crank_fee => match t.trigger {
                TriggerV0::Now => TimedTask {
                    task_time: *now.borrow(),
                    task_key,
                    total_retries: 0,
                    max_retries: args.max_retries,
                },
                TriggerV0::Timestamp(ts) => TimedTask {
                    task_time: ts as u64,
                    task_key,
                    total_retries: 0,
                    max_retries: args.max_retries,
                },
            },
            _ => continue,
        };
        task_queue
            .add_task(task)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to add task: {}", e))?;
    }

    let stream_fut = stream
        .map_err(|e| anyhow::anyhow!("Error in queue nodes stream: {}", e))
        .try_for_each(|update| {
            let task_queue = task_queue.clone();
            let now = now.clone();
            async move {
                let now = *now.borrow();
                for (task_key, account) in update.tasks {
                    match &account {
                        Some(t) if t.crank_reward >= args.min_crank_fee => {
                            let task = match t.trigger {
                                TriggerV0::Now => TimedTask {
                                    task_time: now,
                                    task_key,
                                    total_retries: 0,
                                    max_retries: args.max_retries,
                                },
                                TriggerV0::Timestamp(ts) => TimedTask {
                                    task_time: ts as u64,
                                    task_key,
                                    total_retries: 0,
                                    max_retries: args.max_retries,
                                },
                            };
                            task_queue
                                .add_task(task)
                                .await
                                .map_err(|e| anyhow::anyhow!("Failed to add task: {}", e))?;
                        }
                        _ => (),
                    }
                }

                Ok(())
            }
        });

    tokio::select! {
        res = stream_fut => res.map_err(anyhow::Error::from),
        _ = handle.on_shutdown_requested() => anyhow::Ok(()),
    }
}
