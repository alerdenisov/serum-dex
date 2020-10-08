#![allow(clippy::try_err)]

#[macro_use]
pub mod error;

#[cfg(test)]
mod tests;

pub mod critbit;
mod fees;
pub mod instruction;
pub mod matching;
pub mod state;
pub mod entrypoint;