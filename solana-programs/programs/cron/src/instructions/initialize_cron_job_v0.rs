use std::str::FromStr;

use anchor_lang::{prelude::*, solana_program::instruction::Instruction, InstructionData};
use chrono::{DateTime, NaiveDateTime, Utc};
use clockwork_cron::Schedule;
use tuktuk_program::{
    compile_transaction,
    tuktuk::{
        cpi::{accounts::QueueTaskV0, queue_task_v0},
        program::Tuktuk,
    },
    types::QueueTaskArgsV0,
    TaskQueueV0, TransactionSourceV0, TriggerV0,
};

use super::QUEUED_TASKS_PER_QUEUE;
use crate::{
    error::ErrorCode,
    hash_name,
    state::{CronJobNameMappingV0, CronJobV0, UserCronJobsV0},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Default)]
pub struct InitializeCronJobArgsV0 {
    pub schedule: String,
    pub name: String,
    pub free_tasks_per_transaction: u8,
}

#[derive(Accounts)]
#[instruction(args: InitializeCronJobArgsV0)]
pub struct InitializeCronJobV0<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    pub queue_authority: Signer<'info>,
    /// CHECK: Just needed as a setting
    pub authority: Signer<'info>,
    #[account(
        init_if_needed,
        payer = payer,
        space = 8 + 60 + std::mem::size_of::<UserCronJobsV0>(),
        seeds = [b"user_cron_jobs", authority.key().as_ref()],
        bump
    )]
    pub user_cron_jobs: Box<Account<'info, UserCronJobsV0>>,
    #[account(
        init,
        payer = payer,
        space = 8 + 60 + std::mem::size_of::<CronJobV0>() + args.name.len() + args.schedule.len(),
        seeds = [b"cron_job", authority.key().as_ref(), &user_cron_jobs.next_cron_job_id.to_le_bytes()[..]],
        bump
    )]
    pub cron_job: Box<Account<'info, CronJobV0>>,
    #[account(
        init,
        payer = payer,
        space = 8 + 60 + std::mem::size_of::<CronJobNameMappingV0>() + args.name.len(),
        seeds = [
            b"cron_job_name_mapping",
            authority.key().as_ref(),
            &hash_name(args.name.as_str())
        ],
        bump
    )]
    pub cron_job_name_mapping: Account<'info, CronJobNameMappingV0>,
    #[account(mut)]
    pub task_queue: Box<Account<'info, TaskQueueV0>>,
    /// CHECK: Initialized in CPI
    #[account(mut)]
    pub task: AccountInfo<'info>,
    pub system_program: Program<'info, System>,
    pub tuktuk_program: Program<'info, Tuktuk>,
}

pub fn handler(ctx: Context<InitializeCronJobV0>, args: InitializeCronJobArgsV0) -> Result<()> {
    let schedule = Schedule::from_str(&args.schedule);

    if let Err(e) = schedule {
        msg!("Invalid schedule: {}", e);
        return Err(error!(ErrorCode::InvalidSchedule));
    }

    let ts = Clock::get().unwrap().unix_timestamp;
    let now =
        &DateTime::<Utc>::from_naive_utc_and_offset(NaiveDateTime::from_timestamp(ts, 0), Utc);

    ctx.accounts.user_cron_jobs.bump_seed = ctx.bumps.user_cron_jobs;
    ctx.accounts.user_cron_jobs.authority = ctx.accounts.authority.key();

    ctx.accounts.cron_job.set_inner(CronJobV0 {
        id: ctx.accounts.user_cron_jobs.next_cron_job_id,
        user_cron_jobs: ctx.accounts.user_cron_jobs.key(),
        task_queue: ctx.accounts.task_queue.key(),
        authority: ctx.accounts.authority.key(),
        free_tasks_per_transaction: args.free_tasks_per_transaction,
        schedule: args.schedule,
        name: args.name.clone(),
        current_exec_ts: schedule.unwrap().next_after(now).unwrap().timestamp(),
        current_transaction_id: 0,
        next_transaction_id: 0,
        bump_seed: ctx.bumps.cron_job,
        num_transactions: 0,
    });
    ctx.accounts.user_cron_jobs.next_cron_job_id += 1;
    ctx.accounts
        .cron_job_name_mapping
        .set_inner(CronJobNameMappingV0 {
            cron_job: ctx.accounts.cron_job.key(),
            name: args.name,
            bump_seed: ctx.bumps.cron_job_name_mapping,
        });

    let remaining_accounts = (ctx.accounts.cron_job.current_transaction_id
        ..ctx.accounts.cron_job.current_transaction_id + QUEUED_TASKS_PER_QUEUE as u32)
        .map(|i| {
            Pubkey::find_program_address(
                &[
                    b"cron_job_transaction",
                    ctx.accounts.cron_job.key().as_ref(),
                    &i.to_le_bytes(),
                ],
                &crate::ID,
            )
            .0
        })
        .collect::<Vec<Pubkey>>();
    let (queue_tx, _) = compile_transaction(
        vec![Instruction {
            program_id: crate::ID,
            accounts: [
                crate::__cpi_client_accounts_queue_cron_tasks_v0::QueueCronTasksV0 {
                    cron_job: ctx.accounts.cron_job.to_account_info(),
                    task_queue: ctx.accounts.task_queue.to_account_info(),
                }
                .to_account_metas(None),
                remaining_accounts
                    .iter()
                    .map(|pubkey| AccountMeta::new_readonly(*pubkey, false))
                    .collect::<Vec<AccountMeta>>(),
            ]
            .concat(),
            data: crate::instruction::QueueCronTasksV0.data(),
        }],
        vec![],
    )?;

    queue_task_v0(
        CpiContext::new(
            ctx.accounts.tuktuk_program.to_account_info(),
            QueueTaskV0 {
                payer: ctx.accounts.payer.to_account_info(),
                queue_authority: ctx.accounts.queue_authority.to_account_info(),
                task_queue: ctx.accounts.task_queue.to_account_info(),
                task: ctx.accounts.task.to_account_info(),
                system_program: ctx.accounts.system_program.to_account_info(),
            },
        ),
        QueueTaskArgsV0 {
            trigger: TriggerV0::Now,
            transaction: TransactionSourceV0::CompiledV0(queue_tx),
            crank_reward: None,
            free_tasks: QUEUED_TASKS_PER_QUEUE,
            id: ctx.accounts.task_queue.next_available_task_id().unwrap(),
        },
    )?;

    Ok(())
}
