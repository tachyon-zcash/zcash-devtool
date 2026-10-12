//! Experimental mock-proof Tachyon wallet for private development networks.

#[cfg(not(zcash_unstable = "nutachyon"))]
compile_error!("Tachyon requires RUSTFLAGS='--cfg zcash_unstable=\"nutachyon\"'");

mod chain;
mod funding_balance;
mod note;
mod proofs;
mod rpc;
mod store;
mod transaction;
mod wallet;

pub use wallet::Command;

#[cfg(test)]
mod tests;
