//! Validated, replayable public chain inputs. No wallet secrets enter this module.

use anyhow::{Context, Result, ensure};
use ragu_circuits::polynomials::{ProductionRank, Rank};
use serde::{Deserialize, Serialize};
use zcash_tachyon::{
    Anchor, EpochIndex, Tachygram, TachygramSetCommit, TachygramSetPoly, constants,
};

pub const MAX_EPOCH_MEMBERS: usize = 65_536;

/// A set of N roots needs N + 1 coefficients, even with mock proofs.
pub fn max_polynomial_members() -> usize {
    ProductionRank::num_coeffs() - 1
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    pub hash: String,
    pub previous_block_hash: String,
    pub height: u32,
    pub activation_height: u32,
    pub pool_height: u32,
    pub epoch: u32,
    pub epoch_length: u32,
    pub anchor_before: String,
    pub epoch_start_anchor: Option<String>,
    pub anchor_after: String,
    pub stamps: Vec<Stamp>,
    #[serde(default)]
    pub transactions: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stamp {
    pub transaction_index: usize,
    pub txid: String,
    pub tachygram_set: String,
    pub tachygrams: Vec<String>,
}

pub fn bytes32(value: &str) -> Result<[u8; 32]> {
    hex::decode(value)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("expected 32 bytes"))
}

pub fn anchor(value: &str) -> Result<Anchor> {
    Ok(Anchor::read(&bytes32(value)?[..])?)
}

pub fn anchor_hex(value: Anchor) -> String {
    let mut bytes = [0; 32];
    value
        .write(&mut bytes[..])
        .expect("fixed-size anchor fits its encoding");
    hex::encode(bytes)
}

impl Stamp {
    pub fn members(&self) -> Result<Vec<Tachygram>> {
        ensure!(
            self.tachygrams.len() <= max_polynomial_members(),
            "stamp exceeds Tachyon's polynomial capacity"
        );
        self.tachygrams
            .iter()
            .map(|s| Ok(Tachygram::read(&bytes32(s)?[..])?))
            .collect()
    }

    pub fn commitment(&self) -> Result<TachygramSetCommit> {
        Ok(TachygramSetCommit::read(
            &bytes32(&self.tachygram_set)?[..],
        )?)
    }
}

impl Block {
    pub fn entry(&self) -> Result<Anchor> {
        self.epoch_start_anchor
            .as_deref()
            .map(anchor)
            .unwrap_or_else(|| anchor(&self.anchor_before))
    }

    pub fn validate(&self, previous: Option<&Block>, activation: u32) -> Result<()> {
        self.validate_inner(previous, activation, false)
    }

    /// Start at an epoch boundary trusted from the configured node. This checks
    /// the block's transitions, but does not prove the preceding chain history.
    pub fn validate_checkpoint(&self, activation: u32) -> Result<()> {
        ensure!(
            self.pool_height.is_multiple_of(constants::EPOCH_SIZE),
            "checkpoint must start at an epoch boundary"
        );
        self.validate_inner(None, activation, true)
    }

    fn validate_inner(
        &self,
        previous: Option<&Block>,
        activation: u32,
        checkpoint: bool,
    ) -> Result<()> {
        bytes32(&self.hash)?;
        bytes32(&self.previous_block_hash)?;
        ensure!(
            self.activation_height == activation,
            "NuTachyon activation height drift"
        );
        ensure!(
            self.height.checked_sub(activation) == Some(self.pool_height),
            "invalid pool height"
        );
        ensure!(
            self.epoch_length == constants::EPOCH_SIZE,
            "node and wallet epoch lengths differ"
        );
        ensure!(
            self.epoch == self.pool_height / constants::EPOCH_SIZE,
            "invalid epoch"
        );
        ensure!(
            self.epoch < constants::EPOCH_MAX,
            "final protocol epoch is unsupported"
        );
        ensure!(self.stamps.len() <= MAX_EPOCH_MEMBERS, "too many stamps");
        let mut cursor = anchor(&self.anchor_before)?;
        match previous {
            Some(previous) => {
                ensure!(
                    previous.height.checked_add(1) == Some(self.height),
                    "chain gap"
                );
                ensure!(
                    previous.hash == self.previous_block_hash,
                    "chain reorganized while syncing"
                );
                ensure!(
                    previous.anchor_after == self.anchor_before,
                    "anchor discontinuity"
                );
            }
            None => {
                ensure!(
                    checkpoint || self.height == activation,
                    "feed must start at activation"
                );
                if self.height == activation {
                    ensure!(cursor == Anchor::default(), "incorrect activation anchor");
                }
            }
        }
        if self.pool_height.is_multiple_of(constants::EPOCH_SIZE) {
            if self.epoch != 0 {
                cursor = cursor.next_epoch(EpochIndex::new(self.epoch))?;
            }
            ensure!(
                self.epoch_start_anchor.as_deref() == Some(anchor_hex(cursor).as_str()),
                "incorrect epoch crossing"
            );
        } else {
            ensure!(
                self.epoch_start_anchor.is_none(),
                "unexpected epoch crossing"
            );
        }
        let mut last_index = None;
        let mut member_count = 0usize;
        for stamp in &self.stamps {
            bytes32(&stamp.txid)?;
            ensure!(
                last_index.is_none_or(|last| stamp.transaction_index > last),
                "stamps are out of order"
            );
            last_index = Some(stamp.transaction_index);
            let members = stamp.members()?;
            member_count = member_count
                .checked_add(members.len())
                .context("member count overflow")?;
            ensure!(
                member_count <= MAX_EPOCH_MEMBERS,
                "block exceeds wallet's experimental capacity"
            );
            let unique: std::collections::BTreeSet<_> = members.iter().copied().collect();
            ensure!(
                unique.len() == members.len(),
                "duplicate tachygrams in a stamp"
            );
            let commit = members.into_iter().collect::<TachygramSetPoly>().commit();
            ensure!(
                commit == stamp.commitment()?,
                "tachygrams do not match their commitment"
            );
            cursor = cursor.next_stamp(EpochIndex::new(self.epoch), &commit)?;
        }
        ensure!(
            cursor == anchor(&self.anchor_after)?,
            "stamps do not reproduce the block anchor"
        );
        Ok(())
    }
}
