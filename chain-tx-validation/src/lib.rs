#![cfg_attr(all(feature = "mesalock_sgx", not(target_env = "sgx")), no_std)]
#![cfg_attr(
    all(target_env = "sgx", target_vendor = "mesalock"),
    feature(rustc_private)
)]
#![deny(missing_docs, unsafe_code, unstable_features)]
//! This crate contains functionality for transaction validation. It's currently tested in chain-abci. (TODO: move tests)
//! WARNING: all validation is pure functions / without DB access => it assumes double-spending BitVec is checked in chain-abci

/// transaction witness verification
pub mod witness;

#[cfg(all(feature = "mesalock_sgx", not(target_env = "sgx")))]
#[macro_use]
extern crate sgx_tstd as std;

use std::prelude::v1::Vec;

use chain_core::common::Timespec;
use chain_core::init::coin::{Coin, CoinError};
use chain_core::state::account::{DepositBondTx, StakedState, UnbondTx, WithdrawUnbondedTx};
use chain_core::tx::data::input::TxoPointer;
use chain_core::tx::data::output::TxOut;
use chain_core::tx::data::Tx;
use chain_core::tx::data::TxId;
use chain_core::tx::fee::Fee;
use chain_core::tx::witness::TxWitness;
use chain_core::tx::TransactionId;
use parity_codec::{Decode, Encode};
use secp256k1;
use std::collections::BTreeSet;
use std::{fmt, io};
use witness::verify_tx_address;

/// All possible TX validation errors
#[derive(Debug)]
pub enum Error {
    /// chain hex ID does not match
    WrongChainHexId,
    /// transaction has no inputs
    NoInputs,
    /// transaction has no outputs
    NoOutputs,
    /// transaction has duplicated inputs
    DuplicateInputs,
    /// output with no credited value
    ZeroCoin,
    /// input or output summation error
    InvalidSum(CoinError),
    /// transaction has more witnesses than inputs
    UnexpectedWitnesses,
    /// transaction has more inputs than witnesses
    MissingWitnesses,
    /// transaction spends an invalid input
    InvalidInput,
    /// transaction spends an input that was already spent
    InputSpent,
    /// transaction input output coin (plus fee) sums don't match
    InputOutputDoNotMatch,
    /// output transaction is in timelock that hasn't passed
    OutputInTimelock,
    /// cryptographic library error
    EcdsaCrypto(secp256k1::Error),
    /// DB read error
    IoError(io::Error),
    /// enclave error or invalid TX,
    EnclaveRejected,
    /// staked state not found
    AccountNotFound,
    /// staked state not unbounded
    AccountNotUnbonded,
    /// outputs created out of a staked state are not time-locked to unbonding period
    AccountWithdrawOutputNotLocked,
    /// incorrect nonce supplied in staked state operation
    AccountIncorrectNonce,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use self::Error::*;
        match self {
            WrongChainHexId => write!(f, "chain hex ID does not match"),
            DuplicateInputs => write!(f, "duplicated inputs"),
            UnexpectedWitnesses => write!(f, "transaction has more witnesses than inputs"),
            MissingWitnesses => write!(f, "transaction has more inputs than witnesses"),
            NoInputs => write!(f, "transaction has no inputs"),
            NoOutputs => write!(f, "transaction has no outputs"),
            ZeroCoin => write!(f, "output with no credited value"),
            InvalidSum(ref err) => write!(f, "input or output sum error: {}", err),
            InvalidInput => write!(f, "transaction spends an invalid input"),
            InputSpent => write!(f, "transaction spends an input that was already spent"),
            InputOutputDoNotMatch => write!(
                f,
                "transaction input output coin (plus fee) sums don't match"
            ),
            OutputInTimelock => write!(f, "output transaction is in timelock"),
            EcdsaCrypto(ref err) => write!(f, "ECDSA crypto error: {}", err),
            IoError(ref err) => write!(f, "IO error: {}", err),
            EnclaveRejected => write!(f, "enclave error or invalid TX"),
            AccountNotFound => write!(f, "account not found"),
            AccountNotUnbonded => write!(f, "account not unbonded for withdrawal"),
            AccountWithdrawOutputNotLocked => write!(
                f,
                "account withdrawal outputs not time-locked to unbonded_from"
            ),
            AccountIncorrectNonce => write!(f, "incorrect transaction count for account operation"),
        }
    }
}

/// External information needed for TX validation
#[derive(Clone, Copy)]
pub struct ChainInfo {
    /// minimal fee computed for the transaction
    pub min_fee_computed: Fee,
    /// network hexamedical ID
    pub chain_hex_id: u8,
    /// time in the previous committed block
    pub previous_block_time: Timespec,
    /// how much time is required to wait until stake state's unbonded amount can be withdrawn
    pub unbonding_period: u32,
}

fn check_attributes(tx_chain_hex_id: u8, extra_info: &ChainInfo) -> Result<(), Error> {
    // TODO: check other attributes?
    // check that chain IDs match
    if extra_info.chain_hex_id != tx_chain_hex_id {
        return Err(Error::WrongChainHexId);
    }
    Ok(())
}

fn check_inputs_basic(inputs: &[TxoPointer], witness: &TxWitness) -> Result<(), Error> {
    // check that there are inputs
    if inputs.is_empty() {
        return Err(Error::NoInputs);
    }

    // check that there are no duplicate inputs
    let mut inputs_s = BTreeSet::new();
    if !inputs.iter().all(|x| inputs_s.insert(x)) {
        return Err(Error::DuplicateInputs);
    }

    // verify transaction witnesses
    if inputs.len() < witness.len() {
        return Err(Error::UnexpectedWitnesses);
    }

    if inputs.len() > witness.len() {
        return Err(Error::MissingWitnesses);
    }

    Ok(())
}

/// wrapper around transactions with outputs
#[derive(Encode, Decode)]
pub enum TxWithOutputs {
    /// normal transfer
    Transfer(Tx),
    /// withdrawing unbonded amount from a staked state
    StakeWithdraw(WithdrawUnbondedTx),
}

impl TxWithOutputs {
    /// returns the particular transaction type's outputs
    pub fn outputs(&self) -> &[TxOut] {
        match self {
            TxWithOutputs::Transfer(tx) => &tx.outputs,
            TxWithOutputs::StakeWithdraw(tx) => &tx.outputs,
        }
    }

    /// returns the particular transaction type's id (currently blake2s_hash(SCALE-encoded tx))
    pub fn id(&self) -> TxId {
        match self {
            TxWithOutputs::Transfer(tx) => tx.id(),
            TxWithOutputs::StakeWithdraw(tx) => tx.id(),
        }
    }
}

fn check_inputs(
    main_txid: &TxId,
    inputs: &[TxoPointer],
    witness: &TxWitness,
    extra_info: &ChainInfo,
    transaction_inputs: Vec<TxWithOutputs>,
) -> Result<Coin, Error> {
    let mut incoins = Coin::zero();
    // verify that txids of inputs correspond to the owner/signer
    // and it'd check they are not spent
    // TODO: zip3 / itertools?
    for (txin, (tx, in_witness)) in inputs
        .iter()
        .zip(transaction_inputs.iter().zip(witness.iter()))
    {
        if txin.id != tx.id() {
            return Err(Error::InvalidInput);
        }
        let input_index = txin.index as usize;
        let outputs = tx.outputs();
        if input_index >= outputs.len() {
            return Err(Error::InvalidInput);
        }
        let txout = &outputs[input_index];
        if let Some(valid_from) = &txout.valid_from {
            if *valid_from > extra_info.previous_block_time {
                return Err(Error::OutputInTimelock);
            }
        }
        let wv = verify_tx_address(&in_witness, main_txid, &txout.address);
        if let Err(e) = wv {
            return Err(Error::EcdsaCrypto(e));
        }
        let sum = incoins + txout.value;
        if let Err(e) = sum {
            return Err(Error::InvalidSum(e));
        } else {
            incoins = sum.unwrap();
        }
    }
    Ok(incoins)
}

fn check_outputs_basic(outputs: &[TxOut]) -> Result<(), Error> {
    // check that there are outputs
    if outputs.is_empty() {
        return Err(Error::NoOutputs);
    }

    // check that all outputs have a non-zero amount
    if !outputs.iter().all(|x| x.value > Coin::zero()) {
        return Err(Error::ZeroCoin);
    }

    // Note: we don't need to check against MAX_COIN because Coin's
    // constructor should already do it.

    // TODO: check address attributes?
    Ok(())
}

fn check_input_output_sums(
    incoins: Coin,
    outcoins: Coin,
    extra_info: &ChainInfo,
) -> Result<Fee, Error> {
    // check sum(input amounts) >= sum(output amounts) + minimum fee
    let min_fee: Coin = extra_info.min_fee_computed.to_coin();
    let total_outsum = outcoins + min_fee;
    if let Err(coin_err) = total_outsum {
        return Err(Error::InvalidSum(coin_err));
    }
    if incoins < total_outsum.unwrap() {
        return Err(Error::InputOutputDoNotMatch);
    }
    let fee_paid = (incoins - outcoins).unwrap();
    Ok(Fee::new(fee_paid))
}

/// checks TransferTx -- TODO: this will be moved to an enclave
/// WARNING: it assumes double-spending BitVec of inputs is checked in chain-abci
pub fn verify_transfer(
    maintx: &Tx,
    witness: &TxWitness,
    extra_info: ChainInfo,
    transaction_inputs: Vec<TxWithOutputs>,
) -> Result<Fee, Error> {
    check_attributes(maintx.attributes.chain_hex_id, &extra_info)?;
    check_inputs_basic(&maintx.inputs, witness)?;
    check_outputs_basic(&maintx.outputs)?;
    let incoins = check_inputs(
        &maintx.id(),
        &maintx.inputs,
        witness,
        &extra_info,
        transaction_inputs,
    )?;
    let outcoins = maintx.get_output_total();
    if let Err(coin_err) = outcoins {
        return Err(Error::InvalidSum(coin_err));
    }
    check_input_output_sums(incoins, outcoins.unwrap(), &extra_info)
}

/// checks depositing to a staked state -- TODO: this will be moved to an enclave
/// WARNING: it assumes double-spending BitVec of inputs is checked in chain-abci
pub fn verify_bonded_deposit(
    maintx: &DepositBondTx,
    witness: &TxWitness,
    extra_info: ChainInfo,
    transaction_inputs: Vec<TxWithOutputs>,
    maccount: Option<StakedState>,
) -> Result<(Fee, Option<StakedState>), Error> {
    check_attributes(maintx.attributes.chain_hex_id, &extra_info)?;
    check_inputs_basic(&maintx.inputs, witness)?;
    let incoins = check_inputs(
        &maintx.id(),
        &maintx.inputs,
        witness,
        &extra_info,
        transaction_inputs,
    )?;
    if incoins <= extra_info.min_fee_computed.to_coin() {
        return Err(Error::InputOutputDoNotMatch);
    }
    let deposit_amount = (incoins - extra_info.min_fee_computed.to_coin()).expect("init");
    let account = match maccount {
        Some(mut a) => {
            a.deposit(deposit_amount);
            Some(a)
        }
        None => Some(StakedState::new_init(
            deposit_amount,
            extra_info.previous_block_time,
            maintx.to_staked_account,
            true,
        )),
    };
    Ok((extra_info.min_fee_computed, account))
}

/// checks moving some amount from bonded to unbonded in staked states
/// NOTE: witness is assumed to be checked in chain-abci
pub fn verify_unbonding(
    maintx: &UnbondTx,
    extra_info: ChainInfo,
    mut account: StakedState,
) -> Result<(Fee, Option<StakedState>), Error> {
    check_attributes(maintx.attributes.chain_hex_id, &extra_info)?;

    // checks that account transaction count matches to the one in transaction
    if maintx.nonce != account.nonce {
        return Err(Error::AccountIncorrectNonce);
    }
    // check that a non-zero amount is being unbound
    if maintx.value == Coin::zero() {
        return Err(Error::ZeroCoin);
    }
    check_input_output_sums(account.bonded, maintx.value, &extra_info)?;
    account.unbond(
        maintx.value,
        extra_info.min_fee_computed.to_coin(),
        extra_info.previous_block_time + i64::from(extra_info.unbonding_period),
    );
    // only pay the minimal fee from the bonded amount if correct; the rest remains in bonded
    Ok((extra_info.min_fee_computed, Some(account)))
}

/// checks wihdrawing from a staked state -- TODO: this will be moved to an enclave
/// NOTE: witness is assumed to be checked in chain-abci
pub fn verify_unbonded_withdraw(
    maintx: &WithdrawUnbondedTx,
    extra_info: ChainInfo,
    mut account: StakedState,
) -> Result<(Fee, Option<StakedState>), Error> {
    check_attributes(maintx.attributes.chain_hex_id, &extra_info)?;
    check_outputs_basic(&maintx.outputs)?;
    // checks that account transaction count matches to the one in transaction
    if maintx.nonce != account.nonce {
        return Err(Error::AccountIncorrectNonce);
    }
    // checks that account can withdraw to outputs
    if account.unbonded_from > extra_info.previous_block_time {
        return Err(Error::AccountNotUnbonded);
    }
    // checks that there is something to wihdraw
    if account.unbonded == Coin::zero() {
        return Err(Error::ZeroCoin);
    }
    // checks that outputs are locked to the unbonded time
    if !maintx
        .outputs
        .iter()
        .all(|x| x.valid_from == Some(account.unbonded_from))
    {
        return Err(Error::AccountWithdrawOutputNotLocked);
    }
    let outcoins = maintx.get_output_total();
    if let Err(coin_err) = outcoins {
        return Err(Error::InvalidSum(coin_err));
    }
    let fee = check_input_output_sums(account.unbonded, outcoins.unwrap(), &extra_info)?;
    account.withdraw();
    Ok((fee, Some(account)))
}
