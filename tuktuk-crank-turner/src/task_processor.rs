use std::{collections::HashSet, sync::Arc};

use futures::{Stream, StreamExt, TryStreamExt};
use solana_client::rpc_config::RpcSimulateTransactionConfig;
use solana_sdk::{
    address_lookup_table::{state::AddressLookupTable, AddressLookupTableAccount},
    commitment_config::CommitmentConfig,
    message::{v0, VersionedMessage},
    signer::Signer,
    transaction::VersionedTransaction,
};
use solana_transaction_utils::queue::{TransactionQueueError, TransactionTask};
use tokio_graceful_shutdown::SubsystemHandle;
use tracing::info;
use tuktuk_program::TaskQueueV0;
use tuktuk_sdk::compiled_transaction::{
    next_available_task_ids_excluding_in_progress, run_ix_with_free_tasks,
};

use crate::{
    metrics::{TASKS_COMPLETED, TASKS_FAILED, TASKS_IN_PROGRESS, TASK_IDS_RESERVED},
    task_context::TaskContext,
    task_queue::TimedTask,
};

impl TimedTask {
    pub async fn get_task_queue(&self, ctx: Arc<TaskContext>) -> TaskQueueV0 {
        let lock = ctx.task_queues.lock().await;

        lock.get(&self.task_queue_key).unwrap().clone()
    }

    pub async fn get_or_populate_luts(
        &self,
        ctx: Arc<TaskContext>,
    ) -> anyhow::Result<Vec<AddressLookupTableAccount>, tuktuk_sdk::error::Error> {
        let mut lookup_tables = ctx.lookup_tables.lock().await;
        let mut result: Vec<AddressLookupTableAccount> = Vec::new();
        let mut missing_addresses = Vec::new();
        let task_queue = self.get_task_queue(ctx.clone()).await;

        // Try to get LUTs from existing map
        for addr in &task_queue.lookup_tables {
            if let Some(lut) = lookup_tables.get(addr) {
                result.push(lut.clone());
            } else {
                missing_addresses.push(*addr);
            }
        }

        // If we have missing LUTs, fetch them
        if !missing_addresses.is_empty() {
            let fetched_luts = ctx
                .rpc_client
                .get_multiple_accounts(&missing_addresses)
                .await?
                .into_iter()
                .zip(missing_addresses.iter())
                .filter_map(|(acc, addr)| {
                    acc.map(|acc| {
                        let lut = AddressLookupTable::deserialize(&acc.data)
                            .map_err(tuktuk_sdk::error::Error::from)
                            .map(|lut| AddressLookupTableAccount {
                                key: *addr,
                                addresses: lut.addresses.to_vec(),
                            })?;
                        Ok::<_, tuktuk_sdk::error::Error>((*addr, lut))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;

            // Insert new LUTs into map and result
            for (addr, lut) in fetched_luts {
                lookup_tables.insert(addr, lut.clone());
                result.push(lut);
            }
        }

        Ok(result)
    }

    pub async fn get_next_available_task_ids(
        &self,
        ctx: Arc<TaskContext>,
    ) -> anyhow::Result<Vec<u16>> {
        let task_queue = self.get_task_queue(ctx.clone()).await;
        let mut in_progress = ctx.in_progress_tasks.lock().await;
        let mut task_ids = in_progress
            .entry(self.task_queue_key)
            .or_insert_with(HashSet::new)
            .clone();

        TASKS_IN_PROGRESS
            .with_label_values(&[self.task_queue_name.as_str()])
            .inc();
        TASK_IDS_RESERVED
            .with_label_values(&[self.task_queue_name.as_str()])
            .set(task_ids.len() as i64);
        let next_available = next_available_task_ids_excluding_in_progress(
            task_queue.capacity,
            &task_queue.task_bitmap,
            self.task.free_tasks,
            &task_ids,
        )?;
        task_ids.extend(next_available.clone());
        Ok(next_available)
    }

    async fn handle_ix_err(
        &self,
        ctx: Arc<TaskContext>,
        err: tuktuk_sdk::error::Error,
    ) -> anyhow::Result<()> {
        info!(?self, ?err, "getting instructions failed");
        let ctx = ctx.clone();
        match err {
            tuktuk_sdk::error::Error::AccountNotFound => {
                info!("lookup table accounts, removing from queue");
                self.handle_completion(ctx, None).await
            }
            _ => {
                self.handle_completion(
                    ctx,
                    Some(TransactionQueueError::SimulatedTransactionError(format!(
                        "Failed to get instructions: {:?}",
                        err
                    ))),
                )
                .await
            }
        }
    }

    pub async fn process(&mut self, ctx: Arc<TaskContext>) -> anyhow::Result<()> {
        let TaskContext {
            rpc_client,
            payer,
            tx_sender,
            ..
        } = &*ctx;

        let maybe_lookup_tables = self.get_or_populate_luts(ctx.clone()).await;
        if let Err(err) = maybe_lookup_tables {
            return self.handle_ix_err(ctx.clone(), err).await;
        }
        let lookup_tables = maybe_lookup_tables.unwrap();
        let next_available = self.get_next_available_task_ids(ctx.clone()).await?;
        self.in_flight_task_ids = next_available.clone();

        let maybe_run_ix = run_ix_with_free_tasks(
            self.task_key,
            &self.task,
            payer.pubkey(),
            next_available,
            lookup_tables,
        )
        .await;

        if let Err(err) = maybe_run_ix {
            return self.handle_ix_err(ctx.clone(), err).await;
        }
        let run_ix = maybe_run_ix.unwrap();

        let ctx = ctx.clone();
        if let Some(run_ix) = run_ix {
            let (recent_blockhash, _) = rpc_client
                .get_latest_blockhash_with_commitment(CommitmentConfig::finalized())
                .await?;
            let mut updated_instructions = vec![
                solana_sdk::compute_budget::ComputeBudgetInstruction::set_compute_unit_limit(
                    1900000,
                ),
            ];
            updated_instructions.extend(run_ix.instructions.clone());

            let message = VersionedMessage::V0(v0::Message::try_compile(
                &payer.pubkey(),
                &updated_instructions,
                &run_ix.lookup_tables,
                recent_blockhash,
            )?);
            let tx = VersionedTransaction::try_new(message, &[payer])?;
            let simulated = rpc_client
                .simulate_transaction_with_config(
                    &tx,
                    RpcSimulateTransactionConfig {
                        commitment: Some(
                            solana_sdk::commitment_config::CommitmentConfig::processed(),
                        ),
                        sig_verify: true,
                        ..Default::default()
                    },
                )
                .await;
            // info!(?simulated, "simulated");
            match simulated {
                Ok(simulated) => {
                    if let Some(err) = simulated.value.err {
                        info!(
                                    ?self,
                            ?err,
                            ?simulated.value.logs,
                            "task simulation failed",
                        );
                        return self
                            .handle_completion(
                                ctx,
                                Some(TransactionQueueError::TransactionError(err)),
                            )
                            .await;
                    }
                }
                Err(err) => {
                    info!(?self, ?err, "task simulation failed");
                    return self
                        .handle_completion(
                            ctx,
                            Some(TransactionQueueError::SimulatedTransactionError(
                                err.to_string(),
                            )),
                        )
                        .await;
                }
            }

            tx_sender
                .send(TransactionTask {
                    task: TimedTask {
                        in_flight_task_ids: run_ix.free_task_ids,
                        ..self.clone()
                    },
                    instructions: run_ix.instructions,
                    lookup_tables: Some(run_ix.lookup_tables),
                })
                .await?;
        }

        Ok(())
    }

    pub async fn handle_completion(
        &self,
        ctx: Arc<TaskContext>,
        err: Option<TransactionQueueError>,
    ) -> anyhow::Result<()> {
        let mut in_progress = ctx.in_progress_tasks.lock().await;
        let task_ids = in_progress
            .entry(self.task_queue_key)
            .or_insert_with(HashSet::new);
        for task_id in &self.in_flight_task_ids {
            task_ids.remove(task_id);
        }
        TASK_IDS_RESERVED
            .with_label_values(&[self.task_queue_name.as_str()])
            .set(task_ids.len() as i64);
        TASKS_IN_PROGRESS
            .with_label_values(&[self.task_queue_name.as_str()])
            .dec();
        drop(in_progress);
        if let Some(err) = err {
            let label = match err {
                TransactionQueueError::SimulatedTransactionError(_) => "Simulated",
                TransactionQueueError::TransactionError(_) => "Transaction",
                TransactionQueueError::RawTransactionError(_) => "RawTransaction",
                TransactionQueueError::FeeTooHigh => "FeeTooHigh",
                TransactionQueueError::IxGroupTooLarge => "IxGroupTooLarge",
            };
            TASKS_FAILED
                .with_label_values(&[self.task_queue_name.as_str(), label])
                .inc();
            match err {
                TransactionQueueError::FeeTooHigh => {
                    info!(?self, ?err, "task fee too high");
                    ctx.task_queue
                        .add_task(TimedTask {
                            task: self.task.clone(),
                            total_retries: 0,
                            // Try again in 10 seconds
                            task_time: self.task_time + 10,
                            task_queue_name: self.task_queue_name.clone(),
                            task_key: self.task_key,
                            task_queue_key: self.task_queue_key,
                            max_retries: self.max_retries,
                            in_flight_task_ids: vec![],
                        })
                        .await?;
                }
                TransactionQueueError::RawTransactionError(_)
                | TransactionQueueError::SimulatedTransactionError(_)
                | TransactionQueueError::TransactionError(_) => {
                    if self.total_retries < self.max_retries {
                        let retry_delay = 30 * (1 << self.total_retries);
                        info!(
                            ?self,
                            ?err,
                            ?retry_delay,
                            "task transaction failed, retrying"
                        );
                        let now = *ctx.now_rx.borrow();

                        ctx.task_queue
                            .add_task(TimedTask {
                                task: self.task.clone(),
                                total_retries: self.total_retries + 1,
                                // Try again in 30 seconds with exponential backoff
                                task_time: now + retry_delay,
                                task_key: self.task_key,
                                task_queue_key: self.task_queue_key,
                                task_queue_name: self.task_queue_name.clone(),
                                max_retries: self.max_retries,
                                in_flight_task_ids: self.in_flight_task_ids.clone(),
                            })
                            .await?;
                    } else {
                        info!(
                            "task {:?} failed after {} retries",
                            self.task_key, self.max_retries
                        );
                        TASKS_FAILED
                            .with_label_values(&[self.task_queue_name.as_str(), "RetriesExceeded"])
                            .inc();
                    }
                }
                _ => {}
            }
        } else {
            TASKS_COMPLETED
                .with_label_values(&[self.task_queue_name.as_str()])
                .inc();
        }
        Ok(())
    }
}

pub async fn process_tasks<T: Stream<Item = TimedTask> + Sized>(
    tasks: Box<T>,
    ctx: Arc<TaskContext>,
    handle: SubsystemHandle,
) -> anyhow::Result<()> {
    let fut = tasks
        .map(anyhow::Ok)
        .try_for_each_concurrent(Some(5), |task| {
            let ctx = ctx.clone();
            async move { task.clone().process(ctx).await }
        });
    tokio::select! {
        _ = handle.on_shutdown_requested() => {
            info!("shutdown requested, stopping tasks queue");
            Ok(())
        }
        res = fut => {
            info!("tasks queue finished");
            res
        }
    }
}
