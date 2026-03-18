#[path = "../common/mod.rs"]
pub mod common;

use common::storage::StorageBuilder;
use pevm::EvmAccount;
use revm::primitives::{fixed_bytes, hex::FromHex, Bytecode, Bytes, B256, U256};

/// `TxSimulatorV2` contract bytecode
const TX_SIMULATOR_V2_HEX: &str = include_str!("./assets/TxSimulatorV2.hex");

/// Wrapper for the TxSimulatorV2 contract.
///
/// TxSimulatorV2 replays a transaction's read/write set against a shared
/// `mapping(bytes32 => uint256)` until a `target` access count is reached.
/// Unlike TxSimulator, `target` is passed directly by the caller rather than
/// derived from `gasUsed / 100`, so it can be calibrated against real
/// wall-clock execution times.
#[derive(Debug, Default)]
pub struct TxSimulatorV2;

impl TxSimulatorV2 {
    /// Build the EVM account for PEVM, embedding the compiled bytecode.
    pub fn build() -> EvmAccount {
        let hex = TX_SIMULATOR_V2_HEX.trim();
        let bytecode = Bytecode::new_raw(Bytes::from_hex(hex).unwrap());

        EvmAccount {
            balance: U256::ZERO,
            nonce: 1u64,
            code_hash: Some(bytecode.hash_slow()),
            code: Some(bytecode.into()),
            storage: StorageBuilder::new().build(),
        }
    }

    /// 4-byte selector for `execute(bytes32[],bytes32[],uint256)`.
    pub fn execute_selector() -> [u8; 4] {
        *fixed_bytes!("2a92932d")
    }

    /// ABI-encode a call to `execute(bytes32[] reads, bytes32[] writes, uint256 target)`.
    ///
    /// `target` is the number of storage accesses to perform — the caller
    /// should set this to `execution_time_ns / t_sload_ns`.
    ///
    /// Layout (after 4-byte selector):
    /// - word 0: offset to `reads`  (= 96 = 3 * 32)
    /// - word 1: offset to `writes` (= 96 + 32 + reads.len() * 32)
    /// - word 2: `target` (static uint256)
    /// - reads array:  length word + elements
    /// - writes array: length word + elements
    pub fn encode_execute(reads: &[B256], writes: &[B256], target: u64) -> Bytes {
        let mut data = Vec::new();

        data.extend_from_slice(&Self::execute_selector());

        // Head: offset to reads (96 bytes = 3 * 32)
        data.extend_from_slice(&U256::from(96u64).to_be_bytes::<32>());

        // Head: offset to writes
        let offset_writes = 96u64 + 32 + reads.len() as u64 * 32;
        data.extend_from_slice(&U256::from(offset_writes).to_be_bytes::<32>());

        // Head: target (static uint256)
        data.extend_from_slice(&U256::from(target).to_be_bytes::<32>());

        // reads array
        data.extend_from_slice(&U256::from(reads.len()).to_be_bytes::<32>());
        for r in reads {
            data.extend_from_slice(r.as_slice());
        }

        // writes array
        data.extend_from_slice(&U256::from(writes.len()).to_be_bytes::<32>());
        for w in writes {
            data.extend_from_slice(w.as_slice());
        }

        Bytes::from(data)
    }
}
