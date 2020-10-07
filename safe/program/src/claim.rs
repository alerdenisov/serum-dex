use serum_common::pack::Pack;
use serum_safe::accounts::{TokenVault, Vesting};
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
) -> Result<(), SafeError> {
    info!("handler: mint_locked");

    let acc_infos = &mut accounts.iter();

    let vesting_acc_beneficiary_info = next_account_info(acc_infos)?;
    let vesting_acc_info = next_account_info(acc_infos)?;
    let safe_acc_info = next_account_info(acc_infos)?;
    let safe_vault_authority_acc_info = next_account_info(acc_infos)?;
    let token_program_acc_info = next_account_info(acc_infos)?;
    let rent_acc_info = next_account_info(acc_infos)?;
    let mint_acc_info = next_account_info(acc_infos)?;
    let token_acc_info = next_account_info(acc_infos)?;

    access_control(AccessControlRequest {
        program_id,
        vesting_acc_info,
        vesting_acc_beneficiary_info,
        token_program_acc_info,
        rent_acc_info,
        mint_acc_info,
        token_acc_info,
    })?;

    Vesting::unpack_mut(
        &mut vesting_acc_info.try_borrow_mut_data()?,
        &mut |vesting_acc: &mut Vesting| {
            state_transition(StateTransitionRequest {
                accounts,
                vesting_acc_info,
                vesting_acc,
                safe_acc_info,
                safe_vault_authority_acc_info,
                mint_acc_info,
                token_acc_info,
            })
            .map_err(Into::into)
        },
    )?;

    Ok(())
}

fn access_control<'a>(req: AccessControlRequest<'a>) -> Result<(), SafeError> {
    info!("access-control: mint");

    let AccessControlRequest {
        program_id,
        vesting_acc_info,
        vesting_acc_beneficiary_info,
        token_program_acc_info,
        rent_acc_info,
        mint_acc_info,
        token_acc_info,
    } = req;

    // Beneficiary authorization.
    {
        if !vesting_acc_beneficiary_info.is_signer {
            return Err(SafeErrorCode::Unauthorized)?;
        }
    }

    // Vesting.
    let vesting = Vesting::unpack(&vesting_acc_info.try_borrow_data()?)?;
    {
        if vesting_acc_info.owner != program_id {
            return Err(SafeErrorCode::InvalidAccount)?;
        }
        if !vesting.initialized {
            return Err(SafeErrorCode::NotInitialized)?;
        }
        if vesting.claimed {
            return Err(SafeErrorCode::AlreadyClaimed)?;
        }
        // Match the signing beneficiary to this account.
        if vesting.beneficiary != *vesting_acc_beneficiary_info.key {
            return Err(SafeErrorCode::Unauthorized)?;
        }
    }

    let rent = Rent::from_account_info(rent_acc_info)?;

    // Token account.
    {
        // unpack_unchecked because it's not yet initialized.
        let token_acc = spl_token::state::Account::unpack(&token_acc_info.try_borrow_data()?)?;
        if token_acc.state != spl_token::state::AccountState::Uninitialized {
            return Err(SafeErrorCode::TokenAccountAlreadyInitialized)?;
        }
        if *token_acc_info.owner != spl_token::ID {
            return Err(SafeErrorCode::InvalidAccountOwner)?;
        }
        if !rent.is_exempt(token_acc_info.lamports(), token_acc_info.try_data_len()?) {
            return Err(SafeErrorCode::NotRentExempt)?;
        }
        if token_acc.owner != vesting.beneficiary {
            return Err(SafeErrorCode::InvalidTokenAccountOwner)?;
        }
        if token_acc.mint != *mint_acc_info.key {
            return Err(SafeErrorCode::InvalidTokenAccountMint)?;
        }
    }

    // Mint.
    {
        // TODO: check mint authority is the program dervied addr
    }

    // Token program.
    {
        if *token_program_acc_info.key != spl_token::ID {
            return Err(SafeErrorCode::InvalidTokenProgram)?;
        }
    }

    // Rent sysvar.
    {
        if *rent_acc_info.key != solana_sdk::sysvar::rent::id() {
            return Err(SafeErrorCode::InvalidRentSysvar)?;
        }
    }

    info!("access-control: success");

    Ok(())
}

fn state_transition<'a, 'b>(req: StateTransitionRequest<'a, 'b>) -> Result<(), SafeError> {
    info!("state-transition: mint");

    let StateTransitionRequest {
        accounts,
        vesting_acc_info,
        safe_acc_info,
        safe_vault_authority_acc_info,
        mint_acc_info,
        token_acc_info,
        vesting_acc,
    } = req;

    // Mint all the tokens associated with the NFT. They're just
    // receipts and can't be redeemed for anything without the
    // beneficiary signing off.
    {
        info!("invoke: spl_token::instruction::mint_to");

        let mint_to_instr = spl_token::instruction::mint_to(
            &spl_token::ID,
            mint_acc_info.key,
            token_acc_info.key,
            safe_vault_authority_acc_info.key,
            &[],
            vesting_acc.start_balance,
        )?;

        let data = safe_acc_info.try_borrow_data()?;
        let nonce = data[data.len() - 1];
        let signer_seeds = TokenVault::signer_seeds(safe_acc_info.key, &nonce);

        solana_sdk::program::invoke_signed(&mint_to_instr, &accounts[..], &[&signer_seeds])?;
    }

    vesting_acc.claimed = true;

    info!("state-transition: success");

    Ok(())
}

struct AccessControlRequest<'a> {
    program_id: &'a Pubkey,
    vesting_acc_info: &'a AccountInfo<'a>,
    vesting_acc_beneficiary_info: &'a AccountInfo<'a>,
    token_program_acc_info: &'a AccountInfo<'a>,
    rent_acc_info: &'a AccountInfo<'a>,
    mint_acc_info: &'a AccountInfo<'a>,
    token_acc_info: &'a AccountInfo<'a>,
}

struct StateTransitionRequest<'a, 'b> {
    accounts: &'a [AccountInfo<'a>],
    vesting_acc_info: &'a AccountInfo<'a>,
    safe_acc_info: &'a AccountInfo<'a>,
    safe_vault_authority_acc_info: &'a AccountInfo<'a>,
    mint_acc_info: &'a AccountInfo<'a>,
    token_acc_info: &'a AccountInfo<'a>,
    vesting_acc: &'b mut Vesting,
}