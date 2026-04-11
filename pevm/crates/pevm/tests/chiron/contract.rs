#[path = "../common/mod.rs"]
pub mod common;

use common::storage::{StorageBuilder};
use pevm::{EvmAccount};
use revm::primitives::{
    fixed_bytes, hex::FromHex, Bytecode, Bytes, B256, U256,
};

/// `Chiron` contract bytecode
const CHIRON: &str = include_str!("./assets/Chiron.hex");

#[derive(Debug, Default)]
pub struct Chiron {
}

impl Chiron {
    /// Build the EVM account for PEVM
    pub fn build() -> EvmAccount {
        let hex = CHIRON.trim();
        let bytecode = Bytecode::new_raw(Bytes::from_hex(hex).unwrap());

        EvmAccount {
            balance: U256::ZERO,
            nonce: 1u64,
            code_hash: Some(bytecode.hash_slow()),
            code: Some(bytecode.into()),
            storage: StorageBuilder::new().build(),
        }
    }

    /// Function selectors (first 4 bytes of keccak256 hash)
    pub fn exchange_selector() -> [u8; 4] {
        *fixed_bytes!("53556559") // exchange(uint256)
    }

    pub fn exchangetwo_selector() -> [u8; 4] {
        *fixed_bytes!("291da724") // exchangetwo(uint256,uint256)
    }

    pub fn loop_exchange_selector() -> [u8; 4] {
        *fixed_bytes!("179521a6") // loop_exchange(uint256,uint256[])
    }

    //     "53556559": "exchange(uint256)",
    //     "291da724": "exchangeTwo(uint256,uint256)",
    //     "179521a6": "loopExchange(uint256,uint256[])",

    /// Encode `exchange(uint256)` calldata
    pub fn exchange(resource: U256) -> Bytes {
        Bytes::from([&Self::exchange_selector()[..], &B256::from(resource)[..]].concat())
    }

    /// Encode `exchangetwo(uint256,uint256)` calldata
    pub fn exchange_two(resource1: U256, resource2: U256) -> Bytes {
        Bytes::from(
            [
                &Self::exchangetwo_selector()[..],
                &B256::from(resource1)[..],
                &B256::from(resource2)[..],
            ]
                .concat(),
        )
    }

    /// Encode `loop_exchange(uint256[],uint256)` calldata

    // But you'll also need to invert the parameter serialization!
    pub fn loop_exchange(loop_count: U256, resources: &[usize]) -> Bytes {
        let mut head = vec![];
        let mut tail = vec![];

        // word0: loop_count (static)
        head.extend_from_slice(&loop_count.to_be_bytes::<32>());

        // word1: offset to resources array (static pointer)
        let offset = U256::from(32 * 2); // 64 bytes = 2 words
        head.extend_from_slice(&offset.to_be_bytes::<32>());

        // dynamic section: resources array
        tail.extend_from_slice(&U256::from(resources.len()).to_be_bytes::<32>());
        for r in resources {
            tail.extend_from_slice(&U256::from(*r).to_be_bytes::<32>());
        }

        Bytes::from([&Self::loop_exchange_selector()[..], &head[..], &tail[..]].concat())
    }
}
