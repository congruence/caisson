//! Merkle-Patricia inclusion against an execution root, with `alloy-trie`: an account under the
//! state root, a storage slot under that account's storage root, and one log of a receipt under
//! the receipts root. Then conditions on the value the proof established.
//!
//! The checks mirror `lc/types/src/mpt.rs` in the `eth_lc` prototype (lez-hello-world), without
//! anything of LEZ around them. `alloy-trie` checks a proof for an expected value, so the input
//! carries the value: the account's fields, the slot's value, the receipt. The proof then
//! establishes exactly that value at that key, or fails.

use alloy_consensus::ReceiptEnvelope;
use alloy_eips::eip2718::Decodable2718;
use alloy_primitives::{B256, Bytes, U256, keccak256};
use alloy_rlp::Encodable;
use alloy_trie::{Nibbles, TrieAccount, proof::verify_proof};
use borsh::{BorshDeserialize, BorshSerialize};

pub type Root = [u8; 32];

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Account {
    pub nonce: u64,
    pub balance: Root,
    pub storage_root: Root,
    pub code_hash: Root,
}

/// An account under a state root, as `eth_getProof` returns it.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct AccountProof {
    pub address: [u8; 20],
    pub account: Account,
    pub account_proof: Vec<Vec<u8>>,
}

/// An account and one of its storage slots under a state root.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct StorageProof {
    pub account: AccountProof,
    pub slot: Root,
    /// The slot's value, big-endian.
    pub value: Root,
    pub storage_proof: Vec<Vec<u8>>,
}

/// One log of a receipt under a receipts root: the receipt, EIP-2718 encoded as it sits in the
/// trie, at transaction `tx_index`, and the log's position in it.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct ReceiptProof {
    pub tx_index: u64,
    pub receipt: Vec<u8>,
    pub proof: Vec<Vec<u8>>,
    pub log_index: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum Fact {
    Account(AccountProof),
    Storage(StorageProof),
    Log(ReceiptProof),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum Op {
    Equal,
    NotEqual,
    Less,
    AtMost,
    Greater,
    AtLeast,
}

/// `value op rhs`, compared as 32-byte big-endian words. The value is the one the fact
/// establishes: an account's balance, a slot's value, a log's emitter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Condition {
    pub op: Op,
    pub rhs: Root,
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Input {
    /// The state root for an account or a slot, the receipts root for a log.
    pub root: Root,
    pub fact: Fact,
    pub conditions: Vec<Condition>,
}

/// What the guest commits: the root it proved against, the value, and how many conditions held.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Output {
    pub root: Root,
    pub value: Root,
    pub conditions: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidInclusionProof,
    MalformedInput,
    RevertedTransaction,
    ConditionUnsatisfied,
}

fn nodes(proof: &[Vec<u8>]) -> Vec<Bytes> {
    proof.iter().map(|n| Bytes::copy_from_slice(n)).collect()
}

fn word(b: &[u8]) -> Root {
    let mut w = [0u8; 32];
    let n = b.len().min(32);
    w[32 - n..].copy_from_slice(&b[b.len() - n..]);
    w
}

/// The account is in the state trie under `state_root` with exactly these fields.
pub fn verify_account(state_root: &Root, p: &AccountProof) -> Result<TrieAccount, Error> {
    let account = TrieAccount {
        nonce: p.account.nonce,
        balance: U256::from_be_bytes(p.account.balance),
        storage_root: B256::from(p.account.storage_root),
        code_hash: B256::from(p.account.code_hash),
    };
    let mut encoded = Vec::new();
    account.encode(&mut encoded);
    let key = Nibbles::unpack(keccak256(p.address));
    verify_proof(B256::from(*state_root), key, Some(encoded), &nodes(&p.account_proof))
        .map_err(|_| Error::InvalidInclusionProof)?;
    Ok(account)
}

/// The account is under `state_root`, and the slot holds exactly this value under the account's
/// storage root. A zero value is proven absent, as Ethereum stores it.
pub fn verify_storage(state_root: &Root, p: &StorageProof) -> Result<(), Error> {
    let account = verify_account(state_root, &p.account)?;
    let value = U256::from_be_bytes(p.value);
    let expected = (!value.is_zero()).then(|| {
        let mut v = Vec::new();
        value.encode(&mut v);
        v
    });
    let key = Nibbles::unpack(keccak256(p.slot));
    verify_proof(account.storage_root, key, expected, &nodes(&p.storage_proof))
        .map_err(|_| Error::InvalidInclusionProof)
}

/// The receipt is at `tx_index` under `receipts_root`, its transaction succeeded, and the log at
/// `log_index` exists. Returns the log's emitter as a word.
pub fn verify_receipt_log(receipts_root: &Root, p: &ReceiptProof) -> Result<Root, Error> {
    let mut index = Vec::new();
    p.tx_index.encode(&mut index);
    let key = Nibbles::unpack(&index);
    verify_proof(B256::from(*receipts_root), key, Some(p.receipt.clone()), &nodes(&p.proof))
        .map_err(|_| Error::InvalidInclusionProof)?;
    let receipt = ReceiptEnvelope::decode_2718(&mut p.receipt.as_slice()).map_err(|_| Error::MalformedInput)?;
    // A reverted transaction's logs prove nothing happened.
    if !receipt.status() {
        return Err(Error::RevertedTransaction);
    }
    let log = receipt.logs().get(p.log_index as usize).ok_or(Error::MalformedInput)?;
    Ok(word(log.address.as_slice()))
}

impl Op {
    fn holds(self, lhs: &Root, rhs: &Root) -> bool {
        match self {
            Self::Equal => lhs == rhs,
            Self::NotEqual => lhs != rhs,
            Self::Less => lhs < rhs,
            Self::AtMost => lhs <= rhs,
            Self::Greater => lhs > rhs,
            Self::AtLeast => lhs >= rhs,
        }
    }
}

/// Proves the fact under the root, then checks every condition on the value it establishes.
pub fn check(input: &Input) -> Result<Output, Error> {
    let value = match &input.fact {
        Fact::Account(p) => {
            verify_account(&input.root, p)?;
            p.account.balance
        }
        Fact::Storage(p) => {
            verify_storage(&input.root, p)?;
            p.value
        }
        Fact::Log(p) => verify_receipt_log(&input.root, p)?,
    };
    for c in &input.conditions {
        if !c.op.holds(&value, &c.rhs) {
            return Err(Error::ConditionUnsatisfied);
        }
    }
    Ok(Output { root: input.root, value, conditions: input.conditions.len() as u32 })
}
