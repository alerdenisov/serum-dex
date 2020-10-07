//! Program entrypoint.

#![cfg_attr(feature = "strict", deny(warnings))]

use instruction::WlInstruction;
use serde::{Deserialize, Serialize};
use serum_common::pack::*;
use solana_client_gen::prelude::*;
use solana_sdk::account_info::{next_account_info, AccountInfo};
use solana_sdk::entrypoint::ProgramResult;
use solana_sdk::info;
use solana_sdk::pubkey::Pubkey;

solana_sdk::entrypoint!(process_instruction);
fn process_instruction(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    info!("process-instruction");

    let instruction: WlInstruction = WlInstruction::unpack(instruction_data).unwrap();

    let result = match instruction {
        WlInstruction::Initialize { nonce } => handlers::initialize(accounts, nonce),
        WlInstruction::Stake { amount } => handlers::stake(accounts, amount),
        WlInstruction::Unstake { amount } => handlers::unstake(accounts, amount),
    };

    result?;

    info!("process-instruction success");

    Ok(())
}

mod handlers {
    use super::*;
    pub fn initialize(accounts: &[AccountInfo], nonce: u8) -> ProgramResult {
        info!("hander: initialize");

        let acc_infos = &mut accounts.iter();
        let wl_acc_info = next_account_info(acc_infos)?;

        accounts::Wl::unpack_mut(
            &mut wl_acc_info.try_borrow_mut_data()?,
            &mut |wl: &mut accounts::Wl| {
                wl.nonce = nonce;
                Ok(())
            },
        )
    }

    pub fn stake(accounts: &[AccountInfo], amount: u64) -> ProgramResult {
        let acc_infos = &mut accounts.iter();

        let token_acc_info = next_account_info(acc_infos)?;
        let vault_acc_info = next_account_info(acc_infos)?;
        let vault_authority_acc_info = next_account_info(acc_infos)?;
        let token_program_acc_info = next_account_info(acc_infos)?;
        let wl_acc_info = next_account_info(acc_infos)?;

        let data = wl_acc_info.try_borrow_data()?;
        let nonce = data[0];
        let signer_seeds = accounts::signer_seeds(wl_acc_info.key, &nonce);

        // Delegate transfer to oneself.
        let transfer_instruction = spl_token::instruction::transfer(
            &spl_token::ID,
            token_acc_info.key,
            vault_acc_info.key,
            &vault_authority_acc_info.key,
            &[],
            amount,
        )?;
        solana_sdk::program::invoke_signed(
            &transfer_instruction,
            &[
                vault_acc_info.clone(),
                token_acc_info.clone(),
                vault_authority_acc_info.clone(),
                token_program_acc_info.clone(),
            ],
            &[&signer_seeds],
        )
    }

    pub fn unstake(accounts: &[AccountInfo], amount: u64) -> ProgramResult {
        let acc_infos = &mut accounts.iter();

        let token_acc_info = next_account_info(acc_infos)?;
        let vault_acc_info = next_account_info(acc_infos)?;
        let vault_authority_acc_info = next_account_info(acc_infos)?;
        let token_program_acc_info = next_account_info(acc_infos)?;
        let wl_acc_info = next_account_info(acc_infos)?;

        let data = wl_acc_info.try_borrow_data()?;
        let nonce = data[0];
        let signer_seeds = accounts::signer_seeds(wl_acc_info.key, &nonce);

        let transfer_instruction = spl_token::instruction::transfer(
            &spl_token::ID,
            vault_acc_info.key,
            token_acc_info.key,
            &vault_authority_acc_info.key,
            &[],
            amount,
        )?;
        solana_sdk::program::invoke_signed(
            &transfer_instruction,
            &[
                vault_acc_info.clone(),
                token_acc_info.clone(),
                vault_authority_acc_info.clone(),
                token_program_acc_info.clone(),
            ],
            &[&signer_seeds],
        )
    }
}

mod accounts {
    use super::*;
    #[derive(Serialize, Deserialize)]
    pub struct Wl {
        pub nonce: u8,
    }
    serum_common::packable!(Wl);

    pub fn signer_seeds<'a>(wl: &'a Pubkey, nonce: &'a u8) -> [&'a [u8]; 2] {
        [wl.as_ref(), bytemuck::bytes_of(nonce)]
    }
}

#[cfg_attr(feature = "client", solana_client_gen)]
pub mod instruction {
    use super::*;
    #[derive(serde::Serialize, serde::Deserialize)]
    pub enum WlInstruction {
        /// Accounts:
        ///
        /// 0. `[writable]` Whitelist to initialize.
        Initialize { nonce: u8 },
        /// Accounts:
        ///
        /// 0. `[writable]` Safe vault (to transfer tokens from).
        /// 1. `[writable]` Program token vault.
        /// 2. `[]`         Program vault authority.
        /// 3. `[]`         Token program id.
        /// 4. `[]`         Wl.
        Stake { amount: u64 },
        /// Accounts:
        ///
        /// 0. `[writable]` Safe vault (to transfer tokens to).
        /// 1. `[writable]` Program token vault.
        /// 2. `[]`         Program vault authority.
        /// 3. `[]`         Token program id.
        /// 4. `[]`         Wl.
        Unstake { amount: u64 },
        // todo: unstake
    }
    serum_common::packable!(WlInstruction);
}
