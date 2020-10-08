#![cfg(feature = "program")]
#![cfg(not(feature = "no-entrypoint"))]

use solana_sdk::{
    account_info::AccountInfo, entrypoint::ProgramResult, entrypoint_deprecated, pubkey::Pubkey,
};


use crate::{state::State};

entrypoint_deprecated!(process_instruction);
fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    Ok(State::process(
        program_id,
        accounts,
        instruction_data,
    )?)
}
