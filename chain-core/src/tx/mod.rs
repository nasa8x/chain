use std::prelude::v1::Vec;

/// Transaction internal structure
pub mod data;
/// Transaction fee calculation
pub mod fee;
/// Witness structures (e.g. signatures) for transactions
pub mod witness;

use std::fmt;

use parity_codec::{Decode, Encode};

use self::data::Tx;
use self::witness::TxWitness;
use crate::state::account::{DepositBondTx, StakedStateOpWitness, UnbondTx, WithdrawUnbondedTx};
use crate::tx::data::{txid_hash, TxId};
use data::input::{TxoIndex, TxoPointer};

#[derive(Debug, PartialEq, Eq, Clone, Encode, Decode)]
/// FIXME: other TX to be moved here (when support added in enclave etc.)
pub enum PlainTxAux {
    /// normal value transfer Tx with the vector of witnesses
    TransferTx(Tx, TxWitness),
}

impl PlainTxAux {
    /// creates a new Tx with a vector of witnesses (mainly for testing/tools)
    pub fn new(tx: Tx, witness: TxWitness) -> Self {
        PlainTxAux::TransferTx(tx, witness)
    }
}

impl fmt::Display for PlainTxAux {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlainTxAux::TransferTx(tx, witness) => display_tx_witness(f, tx, witness),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Encode, Decode)]
/// TODO: custom Encode/Decode when data structures are finalized (for backwards/forwards compatibility, encoders/decoders should be able to work with old formats)
pub enum TxAux {
    /// normal value transfer Tx with the vector of witnesses
    TransferTx {
        txid: TxId,
        inputs: Vec<TxoPointer>,
        no_of_outputs: TxoIndex,
        nonce: [u8; 12],
        txpayload: Vec<u8>,
    },
    /// Tx "spends" utxos to be deposited as bonded stake in an account (witnesses as in transfer)
    DepositStakeTx(DepositBondTx, TxWitness),
    /// Tx that modifies account state -- moves some bonded stake into unbonded (witness for account)
    UnbondStakeTx(UnbondTx, StakedStateOpWitness),
    /// Tx that "creates" utxos out of account state; withdraws unbonded stake (witness for account)
    WithdrawUnbondedStakeTx(WithdrawUnbondedTx, StakedStateOpWitness),
}

pub trait TransactionId: Encode {
    /// retrieves a TX ID (currently blake2s(scale_codec_bytes(tx)))
    fn id(&self) -> TxId {
        txid_hash(&self.encode())
    }
}

impl TxAux {
    /// retrieves a TX ID (currently blake2s(scale_codec_bytes(tx)))
    pub fn tx_id(&self) -> TxId {
        match self {
            TxAux::TransferTx { txid, .. } => *txid,
            TxAux::DepositStakeTx(tx, _) => tx.id(),
            TxAux::UnbondStakeTx(tx, _) => tx.id(),
            TxAux::WithdrawUnbondedStakeTx(tx, _) => tx.id(),
        }
    }
}

fn display_tx_witness<T: fmt::Display, W: fmt::Debug>(
    f: &mut fmt::Formatter<'_>,
    tx: T,
    witness: W,
) -> fmt::Result {
    writeln!(f, "Tx:\n{}", tx)?;
    writeln!(f, "witnesses: {:?}\n", witness)
}

impl fmt::Display for TxAux {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TxAux::TransferTx { txid, inputs, .. } => {
                writeln!(f, "Transfer Tx id:\n{}", hex::encode(&txid[..]))?;
                writeln!(f, "tx inputs: {:?}\n", inputs)
            }
            TxAux::DepositStakeTx(tx, witness) => display_tx_witness(f, tx, witness),
            TxAux::UnbondStakeTx(tx, witness) => display_tx_witness(f, tx, witness),
            TxAux::WithdrawUnbondedStakeTx(tx, witness) => display_tx_witness(f, tx, witness),
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::common::MerkleTree;
    use crate::init::coin::Coin;
    use crate::tx::data::access::{TxAccess, TxAccessPolicy};
    use crate::tx::data::address::ExtendedAddr;
    use crate::tx::data::input::TxoPointer;
    use crate::tx::data::output::TxOut;
    use crate::tx::witness::tree::RawPubkey;
    use crate::tx::witness::TxInWitness;
    use parity_codec::{Decode, Encode};
    use secp256k1::{schnorrsig::schnorr_sign, Message, PublicKey, Secp256k1, SecretKey};

    // TODO: rewrite as quickcheck prop
    #[test]
    fn encode_decode() {
        // not a valid transaction, only to test enconding-decoding
        let mut tx = Tx::new();
        tx.add_input(TxoPointer::new([0x01; 32].into(), 1));
        tx.add_output(TxOut::new(
            ExtendedAddr::OrTree([0xbb; 32].into()),
            Coin::unit(),
        ));
        let secp = Secp256k1::new();
        let sk1 = SecretKey::from_slice(&[0xcc; 32][..]).expect("secret key");
        let pk1 = PublicKey::from_secret_key(&secp, &sk1);
        let raw_pk1 = RawPubkey::from(pk1.serialize());

        let raw_public_keys = vec![raw_pk1];

        tx.attributes
            .allowed_view
            .push(TxAccessPolicy::new(pk1.clone(), TxAccess::AllData));

        let msg = Message::from_slice(&tx.id()).expect("msg");

        let merkle = MerkleTree::new(raw_public_keys.clone());

        let w1 = TxInWitness::TreeSig(
            schnorr_sign(&secp, &msg, &sk1).0,
            merkle.generate_proof(raw_public_keys[0].clone()).unwrap(),
        );
        let txa = PlainTxAux::TransferTx(tx, vec![w1].into());
        let mut encoded: Vec<u8> = txa.encode();
        let mut data: &[u8] = encoded.as_mut();
        let decoded = PlainTxAux::decode(&mut data).expect("decode tx aux");
        assert_eq!(txa, decoded);
    }
}
