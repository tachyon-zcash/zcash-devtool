//! Rebuild spendability from durable public inputs, including closed/empty epochs.
//! Proofs are deliberately ephemeral; restarting never depends on serializing mock PCDs.

use anyhow::{Context, Result, ensure};
use ragu::{Pcd, Proof};
use zcash_tachyon::{
    EpochIndex, Note, Tachygram,
    keys::private::SpendingKey,
    nullifier::NF_DERIVATION_WIDTH,
    stamp::proof::{PROOF_SYSTEM, delegation, pool, qr, spendable},
    witness,
};

use crate::chain::{Block, anchor};

mod epoch;
use epoch::bucket;

fn trivial() -> Pcd<()> {
    Proof::trivial().carry(())
}

pub fn secret(key: &SpendingKey, note: Note) -> Result<Pcd<delegation::NoteSecret>> {
    Ok(PROOF_SYSTEM
        .seed(
            &mut rand::rng(),
            delegation::NoteSeed,
            witness::note_seed(((), ()), note, key.derive_proof_private()),
        )?
        .0)
}

fn derivation(
    secret: &Pcd<delegation::NoteSecret>,
    epoch: u32,
) -> Result<Pcd<delegation::NoteNullifiers>> {
    let width = u32::try_from(NF_DERIVATION_WIDTH)?;
    let start = EpochIndex::new(epoch - epoch % width);
    Ok(PROOF_SYSTEM
        .fuse(
            &mut rand::rng(),
            delegation::NullifierDerive,
            witness::nullifier_derive((*secret.data(), ()), start),
            secret.clone(),
            trivial(),
        )?
        .0)
}

pub fn spendable(
    blocks: &[Block],
    key: &SpendingKey,
    note: Note,
) -> Result<Pcd<spendable::NoteSpendable>> {
    let cm = Tachygram::from(note.commitment());
    let current = blocks.last().context("wallet has not synced")?.epoch;
    let mut creation = None;
    for block in blocks {
        let mut cursor = block.entry()?;
        for stamp in &block.stamps {
            let members = stamp.members()?;
            if members.contains(&cm) {
                creation = Some((block.epoch, cursor, members));
                break;
            }
            cursor = cursor.next_stamp(EpochIndex::new(block.epoch), &stamp.commitment()?)?;
        }
        if creation.is_some() {
            break;
        }
    }
    let (created, before, members) = creation.context("note commitment has not been mined")?;
    let secret = secret(key, note)?;
    if created == current {
        let derived = derivation(&secret, current)?;
        return Ok(PROOF_SYSTEM
            .fuse(
                &mut rand::rng(),
                spendable::SpendableInit,
                witness::spendable_init(
                    (*derived.data(), ()),
                    before,
                    &members,
                    EpochIndex::new(current),
                ),
                derived,
                trivial(),
            )?
            .0);
    }
    let master = key.derive_nullifier_private().derive_note_private(note.psi);
    let mut lineage = None;
    for epoch in created..current {
        let nf = master.derive_nullifier(EpochIndex::new(epoch));
        let (absence_bucket, members) = bucket(blocks, epoch, nf.into())?;
        let (arbitrary, ()) = PROOF_SYSTEM.fuse(
            &mut rand::rng(),
            qr::QrUnspentInit,
            witness::qr_unspent_init((*absence_bucket.data(), ()), nf.into(), &members),
            absence_bucket.clone(),
            trivial(),
        )?;
        let derived = derivation(&secret, epoch)?;
        let (_, first, _, last) = *derived.data();
        let window: Vec<_> = (u32::from(first)..=u32::from(last))
            .map(|e| master.derive_nullifier(EpochIndex::new(e)))
            .collect();
        let (unspent, ()) = PROOF_SYSTEM.fuse(
            &mut rand::rng(),
            pool::UnspentBind,
            witness::unspent_bind((*arbitrary.data(), *derived.data()), &window, &[nf]),
            arbitrary,
            derived,
        )?;
        lineage = Some(match lineage {
            None => {
                // The note commitment need not route to its nullifier's bucket.
                // Both proofs cover the same complete epoch and anchor bounds.
                let (creation_bucket, members) = if members.contains(&cm) {
                    (absence_bucket, members)
                } else {
                    bucket(blocks, epoch, cm)?
                };
                PROOF_SYSTEM
                    .fuse(
                        &mut rand::rng(),
                        spendable::QrSpendableInit,
                        witness::qr_spendable_init(
                            (*unspent.data(), *creation_bucket.data()),
                            &members,
                        ),
                        unspent,
                        creation_bucket,
                    )?
                    .0
            }
            Some(left) => {
                PROOF_SYSTEM
                    .fuse(
                        &mut rand::rng(),
                        spendable::SpendableLift,
                        (),
                        left,
                        unspent,
                    )?
                    .0
            }
        });
    }
    lineage.context("closed creation epoch must produce a spendable proof")
}

pub fn lift_stamp(
    blocks: &[Block],
    stamp: zcash_tachyon::ProofStamp,
    descriptors: &std::collections::BTreeSet<zcash_tachyon::action::Descriptor>,
) -> Result<zcash_tachyon::ProofStamp> {
    let tip = blocks.last().context("wallet has not synced")?;
    let target = anchor(&tip.anchor_after)?;
    if stamp.anchor == target {
        return Ok(stamp);
    }
    let mut cursor = stamp.anchor;
    let mut links = Vec::new();
    for block in blocks.iter().filter(|b| b.epoch == tip.epoch) {
        let mut before = block.entry()?;
        for published in &block.stamps {
            let commitment = published.commitment()?;
            let after = before.next_stamp(EpochIndex::new(tip.epoch), &commitment)?;
            if before == cursor {
                links.push((before, EpochIndex::new(tip.epoch), commitment));
                cursor = after;
            }
            before = after;
        }
    }
    ensure!(cursor == target, "cannot lift the stamp to the synced tip");
    Ok(stamp.lift(&mut rand::rng(), descriptors, links)?)
}
