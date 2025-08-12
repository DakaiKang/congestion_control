// -------------------- 你的序列化结构（沿用我之前提供的定义） --------------------
// 建议把这部分放在一个 module 里复用
pub use ethereum_types::{H160, H256, U256, U64};
pub use rlp::RlpStream;
pub use revm::primitives::AccessListItem;
pub use revm::primitives::{TxEnv, TransactTo, Bytes};

#[derive(Clone, Debug)]
pub enum TxTo { Call(H160), Create }

// #[derive(Clone, Debug)]
// pub struct AccessListItem {
//     pub address: H160,
//     pub storage_keys: Vec<H256>,
// }

#[derive(Clone, Debug)]
pub enum TxKind { Legacy, Eip1559 }

#[derive(Clone, Debug)]
pub struct TxUnsigned {
    pub kind: TxKind,
    pub chain_id: U64,
    pub nonce: U64,
    pub gas_price: Option<U256>,                // legacy 用
    pub max_priority_fee_per_gas: Option<U256>, // 1559 用
    pub max_fee_per_gas: Option<U256>,          // 1559 用
    pub gas_limit: U64,
    pub to: TxTo,
    pub value: U256,
    pub data: Vec<u8>,
    pub access_list: Vec<AccessListItem>,       // legacy 通常为空
}

#[derive(Clone, Debug)]
pub struct Signature { pub v: u64, pub r: U256, pub s: U256 }

// （serialize_for_signing / serialize_signed 省略，直接复用我之前给的实现）
// ------------------------------------------------------------------------------


// -------------------- 这里开始：从 revm::primitives::TxEnv 适配 --------------------
    use super::*;
    use revm::primitives::{Address, U256 as R256, TransactTo as RTransactTo};

    /// 简单的错误类型（缺失 chain_id / nonce 等）
    #[derive(Debug, thiserror::Error)]
    pub enum AdaptError {
        #[error("missing chain_id in TxEnv")]
        MissingChainId,
        #[error("unsupported access list item shape")]
        AccessListShape,
    }

    // revm::U256 -> ethereum_types::U256
    #[inline]
    fn u256_from_revm(x: R256) -> U256 {
        let bytes_be: [u8; 32] = x.to_be_bytes(); // big-endian
        U256::from(bytes_be)
    }

    #[inline]
    fn h160_from_revm(a: Address) -> H160 {
        let arr: [u8; 20] = a.into();
        H160::from(arr)
    }

    #[inline]
    fn h256_from_revm_u256(k: R256) -> H256 {
        let bytes_be: [u8; 32] = k.to_be_bytes(); // big-endian
        H256::from(bytes_be)
    }

    // // map revm::primitives::TransactTo -> TxTo
    // fn map_to(to: &RTransactTo) -> TxTo {
    //     match to {
    //         RTransactTo::Call(a) => TxTo::Call(h160_from_revm(*a)),
    //         RTransactTo::Create => TxTo::Create,
    //     }
    // }

    pub fn from_revm_txenv(tx: &revm::primitives::TxEnv) -> Result<TxUnsigned, AdaptError> {
        
        let chain_id_u64: U64 = match tx.chain_id {
            Some(id) => U64::from(id),
            None => return Err(AdaptError::MissingChainId),
        };

        let nonce_u64: U64 = match tx.nonce {
            Some(n) => U64::from(n),
            None => U64::zero(),
        };

        let gas_limit_u64: U64 = U64::from(tx.gas_limit);

        // If gas_priority_fee exists, we assume it's EIP-1559; otherwise, it's Legacy.
        let kind = if let Some(_tip) = tx.gas_priority_fee {
            TxKind::Eip1559
        } else {
            TxKind::Legacy
        };

        let to = match tx.transact_to {
            RTransactTo::Call(a) => TxTo::Call(h160_from_revm(a)),
            RTransactTo::Create => TxTo::Create,
        };

        let value = u256_from_revm(tx.value);
        let data = tx.data.to_vec();
        let access_list = tx.access_list.clone();

        let (gas_price, max_priority_fee_per_gas, max_fee_per_gas) = match kind {
            TxKind::Legacy => (
                Some(u256_from_revm(tx.gas_price)),
                None,
                None,
            ),
            TxKind::Eip1559 => {
                // 约定：revm 中 gas_price 作为 max_fee_per_gas；gas_priority_fee 作为 max_priority_fee_per_gas
                let mp = u256_from_revm(tx.gas_priority_fee.expect("checked above"));
                let mf = u256_from_revm(tx.gas_price);
                (None, Some(mp), Some(mf))
            }
        };

        Ok(TxUnsigned {
            kind,
            chain_id: chain_id_u64,
            nonce: nonce_u64,
            gas_price,
            max_priority_fee_per_gas,
            max_fee_per_gas,
            gas_limit: gas_limit_u64,
            to,
            value,
            data,
            access_list,
        })
    }

// TxEnv fields and types:
//   [X]chain_id:  type = core::option::Option<u64>, value = Some(1)
//   [X]nonce:     type = core::option::Option<u64>, value = Some(0)
//   gas_price: type = ruint::Uint<256, 4>, value = 1000
//   [X]gas_priority_fee: type = core::option::Option<ruint::Uint<256, 4>>, value = Some(10)
//   [X]gas_limit: type = u64, value = 21000
//   [X]transact_to: type = alloy_primitives::common::TxKind, value = Call(0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee)
//   [X]value:     type = ruint::Uint<256, 4>, value = 1000000000000000
//   [X]data:      type = alloy_primitives::bytes_::Bytes, value = 0x
//   [X]access_list: type = alloc::vec::Vec<alloy_eip2930::AccessListItem>, value = []
