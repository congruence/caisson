//! Reads one `Input` (Borsh, length-prefixed), proves its fact under its root, checks its
//! conditions, and commits the `Output`. A refusal panics, so it can't be proven.

use borsh::BorshDeserialize;
use mpt_core::{Input, check};
use risc0_zkvm::guest::env;

fn main() {
    let start = env::cycle_count();

    // Borsh over a raw slice: risc0's serde codec widens every byte to a word, which would inflate
    // both the input and the cycles spent reading proof nodes.
    let len: u32 = env::read();
    let mut bytes = vec![0u8; len as usize];
    env::read_slice(&mut bytes);
    let input = Input::try_from_slice(&bytes).expect("an Input");
    let read = env::cycle_count();

    let output = check(&input).unwrap_or_else(|e| panic!("refused: {e:?}"));
    let checked = env::cycle_count();

    env::commit_slice(&borsh::to_vec(&output).expect("an Output encodes"));

    // Read by the host in execute mode; not part of the journal.
    eprintln!("cycles: read {} check {}", read - start, checked - read);
}
