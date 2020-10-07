use serum_common::pack::Pack;
use serum_safe::accounts::{Safe, TokenVault, Whitelist};
use serum_safe::error::{SafeError, SafeErrorCode};
use solana_sdk::account_info::{next_account_info, AccountInfo};
use solana_sdk::info;
use solana_sdk::program_pack::Pack as TokenPack;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::sysvar::rent::Rent;
use solana_sdk::sysvar::Sysvar;
use std::convert::Into;

pub fn handler<'a>(
    program_id: &'a Pubkey,
    accounts: &'a [AccountInfo<'a>],
    amount: u64,
    instruction_data: Vec<u8>,
) -> Result<(), SafeError> {
    info!("handler: whitelist_withdraw");

    let acc_infos = &mut accounts.iter();

    // let safe_acc_info = next_account_info(acc_infos)?;

    access_control(AccessControlRequest {})?;

    state_transition(StateTransitionRequest {})?;

    Ok(())
}

fn access_control(req: AccessControlRequest) -> Result<(), SafeError> {
    info!("access-control: whitelist_withdraw");

    let AccessControlRequest {} = req;

    info!("access-control: success");

    Ok(())
}

fn state_transition(req: StateTransitionRequest) -> Result<(), SafeError> {
    info!("state-transition: whitelist_withdraw");

    let StateTransitionRequest {} = req;

    info!("state-transition: success");

    Ok(())
}

struct AccessControlRequest {}

struct StateTransitionRequest {}
