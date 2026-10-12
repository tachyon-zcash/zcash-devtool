//! Explicit out-of-band receipts: Tachyon has no on-chain note ciphertext to scan.

use anyhow::{Result, ensure};
use ff::{Field, PrimeField};
use pasta_curves::Fp;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use zcash_tachyon::{Note, keys::private::SpendingKey, note::CommitmentTrapdoor, nullifier, value};

use crate::chain::bytes32;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub version: u32,
    pub network_id: String,
    pub payment_key: String,
    pub value_zat: u64,
    pub psi: String,
    pub rcm: String,
}

fn field(encoded: &str) -> Result<Fp> {
    Option::<Fp>::from(Fp::from_repr(bytes32(encoded)?))
        .ok_or_else(|| anyhow::anyhow!("non-canonical field encoding"))
}

pub fn payment_key(key: &SpendingKey) -> String {
    hex::encode(Fp::from(key.derive_payment_key()).to_repr())
}

pub fn address(key: &SpendingKey) -> String {
    format!("tachyon-test:{}", payment_key(key))
}

pub fn decode_address(address: &str) -> Result<String> {
    let key = address.strip_prefix("tachyon-test:").ok_or_else(|| {
        anyhow::anyhow!("expected experimental tachyon-test:<payment-key> address")
    })?;
    field(key)?;
    Ok(key.to_owned())
}

impl Receipt {
    pub fn new(network_id: &str, recipient: &str, value_zat: u64) -> Result<Self> {
        let pk = decode_address(recipient)?;
        let mut rng = rand::rng();
        let psi: Fp = nullifier::Trapdoor::random(&mut rng).into();
        let mut rcm: Fp = CommitmentTrapdoor::random(&mut rng).into();
        while rcm == Fp::ZERO {
            rcm = Fp::random(&mut rng);
        }
        let receipt = Self {
            version: 1,
            network_id: network_id.into(),
            payment_key: pk,
            value_zat,
            psi: hex::encode(psi.to_repr()),
            rcm: hex::encode(rcm.to_repr()),
        };
        receipt.note()?;
        Ok(receipt)
    }

    pub fn note(&self) -> Result<Note> {
        ensure!(self.version == 1, "unsupported receipt version");
        ensure!(
            self.value_zat > 0,
            "zero-value notes are not supported by this wallet"
        );
        let rcm = field(&self.rcm)?;
        ensure!(rcm != Fp::ZERO, "zero commitment trapdoor");
        Ok(Note {
            pk: field(&self.payment_key)?.into(),
            value: value::Positive::try_from(self.value_zat)?,
            psi: field(&self.psi)?.into(),
            rcm: rcm.into(),
        })
    }

    pub fn id(&self) -> Result<String> {
        let cm: Fp = self.note()?.commitment().into();
        Ok(hex::encode(cm.to_repr()))
    }

    pub fn check_owner(&self, key: &SpendingKey, network_id: &str) -> Result<()> {
        self.note()?;
        ensure!(
            self.network_id == network_id,
            "receipt belongs to another network"
        );
        ensure!(
            self.payment_key == payment_key(key),
            "receipt belongs to another wallet"
        );
        Ok(())
    }
}

pub fn random_seed() -> [u8; 32] {
    rand::rng().random()
}
