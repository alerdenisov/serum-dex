use serum_common::pack::Pack;
use serum_safe::accounts::{Safe, TokenVault, Vesting, Whitelist};
use serum_safe::error::{SafeError, SafeErrorCode};
use solana_sdk::account_info::{next_account_info, AccountInfo};
use solana_sdk::info;
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::program_pack::Pack as TokenPack;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::sysvar::rent::Rent;
use solana_sdk::sysvar::Sysvar;
use std::convert::Into;

pub fn handler<'a>(
    program_id: &'a Pubkey,
    accounts: &'a [AccountInfo<'a>],
    amount: u64,
) -> Result<(), SafeError> {
    info!("handler: whitelist_withdraw_start");

    let acc_infos = &mut accounts.iter();

    let beneficiary_acc_info = next_account_info(acc_infos)?;
    let vesting_acc_info = next_account_info(acc_infos)?;
    let safe_acc_info = next_account_info(acc_infos)?;
    let safe_vault_auth_acc_info = next_account_info(acc_infos)?;
    let wl_prog_acc_info = next_account_info(acc_infos)?;

    // Below accounts are relayed.

    let safe_vault_acc_info = next_account_info(acc_infos)?;
    let wl_prog_vault_acc_info = next_account_info(acc_infos)?;
    let wl_prog_vault_authority_acc_info = next_account_info(acc_infos)?;
    let tok_prog_acc_info = next_account_info(acc_infos)?;

    let remaining_relay_accs: Vec<&AccountInfo> = acc_infos.collect();

    access_control(AccessControlRequest {
        beneficiary_acc_info,
        vesting_acc_info,
        wl_prog_vault_acc_info,
        wl_prog_vault_authority_acc_info,
        wl_prog_acc_info,
        safe_acc_info,
        safe_vault_auth_acc_info,
        safe_vault_acc_info,
        tok_prog_acc_info,
    })?;

    Vesting::unpack_mut(
        &mut vesting_acc_info.try_borrow_mut_data()?,
        &mut |vesting: &mut Vesting| {
            let safe = Safe::unpack(&safe_acc_info.try_borrow_data()?)?;
            state_transition(StateTransitionRequest {
                accounts,
                amount,
                safe_acc: safe_acc_info.key,
                nonce: safe.nonce,
                wl_prog_acc_info,
                wl_prog_vault_acc_info,
                wl_prog_vault_authority_acc_info,
                safe_vault_acc_info,
                safe_vault_auth_acc_info,
                tok_prog_acc_info,
                vesting,
                remaining_relay_accs: remaining_relay_accs.clone(),
            })
            .map_err(Into::into)
        },
    )?;

    Ok(())
}

fn access_control(req: AccessControlRequest) -> Result<(), SafeError> {
    info!("access-control: whitelist_withdraw_start");

    let AccessControlRequest {
        beneficiary_acc_info,
        vesting_acc_info,
        wl_prog_vault_acc_info,
        wl_prog_vault_authority_acc_info,
        wl_prog_acc_info,
        safe_acc_info,
        safe_vault_auth_acc_info,
        safe_vault_acc_info,
        tok_prog_acc_info,
    } = req;

    // TODO

    // TODO: beneficiary authorized
    // TODO: thie vesting.whitelist_pending amount *must* be 0.

    info!("access-control: success");

    Ok(())
}

fn state_transition(req: StateTransitionRequest) -> Result<(), SafeError> {
    info!("state-transition: whitelist_withdraw_start");

    let StateTransitionRequest {
        vesting,
        accounts,
        amount,
        nonce,
        safe_acc,
        safe_vault_acc_info,
        wl_prog_acc_info,
        wl_prog_vault_acc_info,
        wl_prog_vault_authority_acc_info,
        remaining_relay_accs,
        tok_prog_acc_info,
        safe_vault_auth_acc_info,
    } = req;

    let signer_seeds = TokenVault::signer_seeds(safe_acc, &nonce);

    // Approve delegate access for the amount.
    {
        info!("approving delegate");
        let approve_instr = spl_token::instruction::approve(
            &tok_prog_acc_info.key,
            &safe_vault_acc_info.key,
            &wl_prog_vault_authority_acc_info.key,
            &safe_vault_auth_acc_info.key,
            &[],
            amount,
        )?;
        solana_sdk::program::invoke_signed(&approve_instr, &accounts[..], &[&signer_seeds])?;
    }

    // Update vesting account.
    vesting.whitelist_pending_transfer = amount;

    info!("state-transition: success");

    Ok(())
}

struct AccessControlRequest<'a> {
    beneficiary_acc_info: &'a AccountInfo<'a>,
    vesting_acc_info: &'a AccountInfo<'a>,
    safe_acc_info: &'a AccountInfo<'a>,
    safe_vault_acc_info: &'a AccountInfo<'a>,
    safe_vault_auth_acc_info: &'a AccountInfo<'a>,
    wl_prog_acc_info: &'a AccountInfo<'a>,
    wl_prog_vault_acc_info: &'a AccountInfo<'a>,
    wl_prog_vault_authority_acc_info: &'a AccountInfo<'a>,
    tok_prog_acc_info: &'a AccountInfo<'a>,
}

struct StateTransitionRequest<'a, 'b> {
    vesting: &'b mut Vesting,
    accounts: &'a [AccountInfo<'a>],
    amount: u64,
    nonce: u8,
    safe_acc: &'a Pubkey,
    safe_vault_acc_info: &'a AccountInfo<'a>,
    safe_vault_auth_acc_info: &'a AccountInfo<'a>,
    wl_prog_acc_info: &'a AccountInfo<'a>,
    wl_prog_vault_acc_info: &'a AccountInfo<'a>,
    wl_prog_vault_authority_acc_info: &'a AccountInfo<'a>,
    remaining_relay_accs: Vec<&'a AccountInfo<'a>>,
    tok_prog_acc_info: &'a AccountInfo<'a>,
}
