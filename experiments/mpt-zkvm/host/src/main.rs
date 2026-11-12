//! Runs the Merkle-Patricia guest on recorded mainnet data (block 26376237) in one of two modes:
//!
//!   execute  the public path's work: the guest runs in RISC Zero's executor and its cycles are
//!            counted, as a sequencer meters a public transaction. No proof.
//!   prove    the private path's work: the guest is proven (a succinct receipt, the kind LEZ
//!            verifies), then the receipt is verified against the image ID.
//!
//!   mpt <account | storage | log> <execute | prove> [conditions]   (conditions: default 1)
//!
//! Prints one JSON line. Peak memory isn't measured here: proving runs in the `r0vm` process, so
//! wrap a run in the OS's `time`, which reports the largest process it waited on.

use std::time::Instant;

use alloy_rlp::{BufMut, Encodable, Header};
use alloy_trie::{HashBuilder, Nibbles, proof::ProofRetainer};
use anyhow::{Context, anyhow, bail, ensure};
use borsh::BorshDeserialize;
use methods::{MPT_ELF, MPT_ID};
use mpt_core::{Account, AccountProof, Condition, Fact, Input, Op, Output, ReceiptProof, Root, StorageProof, check};
use risc0_zkvm::{ExecutorEnv, ProverOpts, default_executor, default_prover};
use serde_json::{Value, json};

const BLOCK: u64 = 26_376_237;
/// The receipt and log the eth_lc prototype's log test proves (`tests/statement.rs`).
const TX_INDEX: u64 = 2;
const LOG_INDEX: u64 = 0;

fn data(name: &str) -> anyhow::Result<Value> {
    let path = format!("{}/../data/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_slice(&std::fs::read(&path).with_context(|| format!("read {path}"))?).with_context(|| format!("parse {path}"))
}

fn hex(v: &Value) -> anyhow::Result<Vec<u8>> {
    let s = v.as_str().ok_or_else(|| anyhow!("{v} is not a hex string"))?.trim_start_matches("0x");
    let s = if s.len() % 2 == 1 { format!("0{s}") } else { s.to_owned() };
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(Into::into)).collect()
}

/// A quantity or hash, left-padded to 32 bytes: nodes shorten quantities (`0x4a8`).
fn word(v: &Value) -> anyhow::Result<Root> {
    let b = hex(v)?;
    ensure!(b.len() <= 32, "{v} is longer than 32 bytes");
    let mut w = [0u8; 32];
    w[32 - b.len()..].copy_from_slice(&b);
    Ok(w)
}

fn uint(v: &Value) -> anyhow::Result<u64> {
    Ok(u64::from_str_radix(v.as_str().ok_or_else(|| anyhow!("{v} is not a quantity"))?.trim_start_matches("0x"), 16)?)
}

/// The block's state and receipts roots, from the finality update's finalized header.
fn roots() -> anyhow::Result<(Root, Root)> {
    let f = data("finality_update.json")?;
    let e = &f["data"]["finalized_header"]["execution"];
    let number: u64 = e["block_number"].as_str().unwrap_or("0").parse()?;
    ensure!(number == BLOCK, "the finalized header is block {number}, not {BLOCK}");
    Ok((word(&e["state_root"])?, word(&e["receipts_root"])?))
}

fn account_proof(r: &Value) -> anyhow::Result<AccountProof> {
    let nodes = r["accountProof"].as_array().ok_or_else(|| anyhow!("no accountProof"))?.iter().map(hex).collect::<anyhow::Result<_>>()?;
    Ok(AccountProof {
        address: hex(&r["address"])?.try_into().map_err(|_| anyhow!("an address is 20 bytes"))?,
        account: Account {
            nonce: uint(&r["nonce"])?,
            balance: word(&r["balance"])?,
            storage_root: word(&r["storageHash"])?,
            code_hash: word(&r["codeHash"])?,
        },
        account_proof: nodes,
    })
}

fn storage_proof() -> anyhow::Result<StorageProof> {
    let r = &data("getproof-26376237.json")?["result"];
    let s = &r["storageProof"][0];
    Ok(StorageProof {
        account: account_proof(r)?,
        slot: word(&s["key"])?,
        value: word(&s["value"])?,
        storage_proof: s["proof"].as_array().ok_or_else(|| anyhow!("no storage proof"))?.iter().map(hex).collect::<anyhow::Result<_>>()?,
    })
}

fn rlp<T: Encodable + ?Sized>(v: &T) -> Vec<u8> {
    let mut out = Vec::new();
    v.encode(&mut out);
    out
}

fn list(items: &[Vec<u8>]) -> Vec<u8> {
    let payload = items.concat();
    let mut out = Vec::new();
    Header { list: true, payload_length: payload.len() }.encode(&mut out);
    out.put_slice(&payload);
    out
}

/// A receipt from `eth_getBlockReceipts`, encoded as it sits in the receipt trie (EIP-2718).
fn encode_receipt(r: &Value) -> anyhow::Result<Vec<u8>> {
    let logs = r["logs"]
        .as_array()
        .ok_or_else(|| anyhow!("a receipt without logs"))?
        .iter()
        .map(|l| -> anyhow::Result<Vec<u8>> {
            let topics = l["topics"].as_array().ok_or_else(|| anyhow!("a log without topics"))?.iter().map(|t| Ok(rlp(&hex(t)?[..]))).collect::<anyhow::Result<Vec<_>>>()?;
            Ok(list(&[rlp(&hex(&l["address"])?[..]), list(&topics), rlp(&hex(&l["data"])?[..])]))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let body = list(&[rlp(&uint(&r["status"])?), rlp(&uint(&r["cumulativeGasUsed"])?), rlp(&hex(&r["logsBloom"])?[..]), list(&logs)]);
    Ok(match uint(&r["type"])? {
        0 => body,
        t => [vec![t as u8], body].concat(),
    })
}

/// A proof of transaction `tx`'s receipt, from the block's receipts: the rebuilt trie root, and
/// the proof.
fn receipt_proof(tx: u64) -> anyhow::Result<(Root, ReceiptProof)> {
    let j = data("receipts-26376237.json")?;
    let receipts = j["result"].as_array().ok_or_else(|| anyhow!("no receipts"))?;
    let mut leaves = receipts.iter().enumerate().map(|(i, r)| Ok((Nibbles::unpack(rlp(&(i as u64))), encode_receipt(r)?))).collect::<anyhow::Result<Vec<_>>>()?;
    leaves.sort_by(|a, b| a.0.cmp(&b.0));
    let target = Nibbles::unpack(rlp(&tx));
    let mut hb = HashBuilder::default().with_proof_retainer(ProofRetainer::new(vec![target]));
    for (k, v) in &leaves {
        hb.add_leaf(*k, v);
    }
    let root = hb.root();
    let proof = hb.take_proof_nodes().matching_nodes_sorted(&target).into_iter().map(|(_, n)| n.to_vec()).collect();
    let receipt = encode_receipt(receipts.get(tx as usize).ok_or_else(|| anyhow!("no transaction {tx}"))?)?;
    Ok((root.0, ReceiptProof { tx_index: tx, receipt, proof, log_index: LOG_INDEX }))
}

fn input(kind: &str, conditions: usize) -> anyhow::Result<Input> {
    let (state_root, receipts_root) = roots()?;
    // Each condition holds for every kind here: the value (a balance, a slot, an emitter) is nonzero.
    let conditions = vec![Condition { op: Op::NotEqual, rhs: [0; 32] }; conditions];
    let (root, fact) = match kind {
        "account" => (state_root, Fact::Account(storage_proof()?.account)),
        "storage" => (state_root, Fact::Storage(storage_proof()?)),
        "log" => {
            let (rebuilt, proof) = receipt_proof(TX_INDEX)?;
            ensure!(rebuilt == receipts_root, "the rebuilt receipt trie doesn't match the finalized header");
            (receipts_root, Fact::Log(proof))
        }
        _ => bail!("kind is account, storage or log, not {kind}"),
    };
    Ok(Input { root, fact, conditions })
}

fn env(bytes: &[u8]) -> anyhow::Result<ExecutorEnv<'static>> {
    let len = u32::try_from(bytes.len())?;
    let bytes = bytes.to_vec();
    Ok(ExecutorEnv::builder().write(&len)?.write_slice(&bytes).build()?)
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let (kind, mode) = match (args.get(1), args.get(2)) {
        (Some(k), Some(m)) => (k.as_str(), m.as_str()),
        _ => bail!("usage: mpt <account | storage | log> <execute | prove> [conditions]"),
    };
    let conditions: usize = args.get(3).map(|c| c.parse()).transpose()?.unwrap_or(1);

    let input = input(kind, conditions)?;
    // Natively first: a bad input fails here, not after a proof.
    let expected = check(&input).map_err(|e| anyhow!("refused natively: {e:?}"))?;
    let bytes = borsh::to_vec(&input)?;

    let report = match mode {
        "execute" => {
            let t = Instant::now();
            let session = default_executor().execute(env(&bytes)?, MPT_ELF)?;
            let seconds = t.elapsed().as_secs_f64();
            ensure!(Output::try_from_slice(&session.journal.bytes)? == expected, "the guest committed another output");
            json!({
                "kind": kind, "mode": mode, "conditions": conditions, "input_bytes": bytes.len(),
                "user_cycles": session.cycles(), "segments": session.segments.len(), "seconds": seconds,
            })
        }
        "prove" => {
            let t = Instant::now();
            let info = default_prover().prove_with_opts(env(&bytes)?, MPT_ELF, &ProverOpts::succinct())?;
            let seconds = t.elapsed().as_secs_f64();
            info.receipt.verify(MPT_ID)?;
            ensure!(Output::try_from_slice(&info.receipt.journal.bytes)? == expected, "the receipt commits another output");
            json!({
                "kind": kind, "mode": mode, "conditions": conditions, "input_bytes": bytes.len(),
                "user_cycles": info.stats.user_cycles, "total_cycles": info.stats.total_cycles,
                "segments": info.stats.segments, "seconds": seconds,
                "receipt_bytes": bincode::serialize(&info.receipt)?.len(),
            })
        }
        _ => bail!("mode is execute or prove, not {mode}"),
    };
    println!("{report}");
    Ok(())
}
