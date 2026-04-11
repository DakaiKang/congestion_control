#[path = "../common/mod.rs"]
pub mod common;

use common::storage::StorageBuilder;
use pevm::EvmAccount;
use revm::primitives::{fixed_bytes, hex::FromHex, Bytecode, Bytes, B256, U256};

/// `TxSimulator` contract bytecode
const TX_SIMULATOR_HEX: &str = include_str!("./assets/TxSimulator.hex");

/// Wrapper for the TxSimulator contract.
///
/// TxSimulator replays a transaction's read/write set against a shared
/// `mapping(bytes32 => uint256)` until a target access count is reached.
/// Target = gasUsed / 100, approximating the original workload.
/// Conflicts arise naturally when two transactions share a storage key.
#[derive(Debug, Default)]
pub struct TxSimulator;

impl TxSimulator {
    /// Build the EVM account for PEVM, embedding the compiled bytecode.
    pub fn build() -> EvmAccount {
        let hex = TX_SIMULATOR_HEX.trim();
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

    /// ABI-encode a call to `execute(bytes32[] reads, bytes32[] writes, uint256 gasUsed)`.
    ///
    /// Layout (after 4-byte selector):
    /// - word 0: offset to `reads`  (= 96 = 3 * 32, head occupies 3 words)
    /// - word 1: offset to `writes` (= 96 + 32 + reads.len() * 32)
    /// - word 2: `gasUsed` (static)
    /// - reads array:  length word + elements
    /// - writes array: length word + elements
    pub fn encode_execute(reads: &[B256], writes: &[B256], gas_used: u64) -> Bytes {
        let mut data = Vec::new();

        // 4-byte selector
        data.extend_from_slice(&Self::execute_selector());

        // Head: offset to reads (96 bytes = 3 * 32)
        data.extend_from_slice(&U256::from(96u64).to_be_bytes::<32>());

        // Head: offset to writes (after head + reads array)
        let offset_writes = 96u64 + 32 + reads.len() as u64 * 32;
        data.extend_from_slice(&U256::from(offset_writes).to_be_bytes::<32>());

        // Head: gasUsed (static uint256)
        data.extend_from_slice(&U256::from(gas_used).to_be_bytes::<32>());

        // reads array: length + elements
        data.extend_from_slice(&U256::from(reads.len()).to_be_bytes::<32>());
        for r in reads {
            data.extend_from_slice(r.as_slice());
        }

        // writes array: length + elements
        data.extend_from_slice(&U256::from(writes.len()).to_be_bytes::<32>());
        for w in writes {
            data.extend_from_slice(w.as_slice());
        }

        Bytes::from(data)
    }
}
