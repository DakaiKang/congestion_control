
pub use ethers::types::{
    transaction::eip2718::TypedTransaction,
    transaction::eip1559::Eip1559TransactionRequest,
    transaction::eip2930::{AccessList, AccessListItem},
    TransactionRequest, NameOrAddress, Address, U256, Bytes, Signature, H256, U64
};

use alloy_primitives::{TxKind, B256};
use alloy_primitives::Bytes as AlloyBytes;
use alloy_primitives::Address as AlloyAddress;
use alloy_rpc_types_eth::AccessListItem as AlloyAccessListItem;

pub use ethers::core::k256::{
    ecdsa::{Signature as K256Signature, SigningKey, VerifyingKey},
    elliptic_curve::sec1::ToEncodedPoint,
};

pub use ethers::utils::keccak256;

pub use hex::FromHex;
pub use rlp::{Rlp, RlpStream};
pub use std::env;

use revm::{
    primitives::{AuthorizationList, BlockEnv, SpecId, TxEnv, ruint::Uint},
    Handler,
};

use super::deserializer::DecodedTransaction;

fn get_address_slice(address: &Address) -> [u8; 20] {
    address.as_bytes().try_into().expect("slice is always 20 bytes")
}

fn option_u256_to_uint_or_zero(opt: Option<U256>) -> Uint<256, 4> {
    match opt {
        Some(v) => {
            let mut bytes = [0u8; 32];
            v.to_big_endian(&mut bytes);
            Uint::<256, 4>::from_be_bytes(bytes)
        }
        None => Uint::<256, 4>::from(0u64),
    }
}


fn to_tx_kind(to: Option<NameOrAddress>) -> TxKind {
    match to {
        Some(NameOrAddress::Address(addr)) => {
            TxKind::Call(AlloyAddress::from_slice(addr.as_bytes()))
        }
        Some(NameOrAddress::Name(_name)) => {
            // ENS resolution required; placeholder error here
            unimplemented!("Need to resolve ENS name to an address")
        }
        None => TxKind::Create,
    }
}

fn ethers_to_alloy_access_list(list: AccessList) -> Vec<AlloyAccessListItem> {
    list.0.into_iter()
        .map(|item| AlloyAccessListItem {
            address: AlloyAddress::from_slice(item.address.as_bytes()), // H160 → alloy::Address
            storage_keys: item
                .storage_keys
                .into_iter()
                .map(|h: H256| B256::from(h.0))
                .collect(),
        })
        .collect()
}

fn legacy_transaction_2_tx_env(tx: TransactionRequest, caller_addr: Address) -> TxEnv {
    TxEnv {
        caller: AlloyAddress::new(get_address_slice(&caller_addr)),
        gas_limit: tx.gas.map(|v| v.as_u64()).unwrap_or(0),  
        gas_price: option_u256_to_uint_or_zero(tx.gas_price),
        transact_to: to_tx_kind(tx.to),
        value: option_u256_to_uint_or_zero(tx.value),
        data: tx.data.map(|b| AlloyBytes::from(b.to_vec())).unwrap_or_default(), 
        nonce: tx.nonce.map(|v| v.as_u64()),
        ..Default::default()
    }
}

fn eip1559_transaction_2_tx_env(tx: Eip1559TransactionRequest, caller_addr: Address) -> TxEnv {
    let mut base_fee = U256::default();
    let max_priority_fee_per_gas = tx.max_priority_fee_per_gas.unwrap_or_default();
    let max_fee_per_gas = tx.max_fee_per_gas.unwrap_or_default();
    let gas_price = if max_fee_per_gas > base_fee + max_priority_fee_per_gas {
        Some(max_fee_per_gas)
    } else {
        Some(max_priority_fee_per_gas)
    };

    TxEnv {
        chain_id: Some(tx.chain_id.unwrap_or_default().as_u64()),
        nonce: tx.nonce.map(|v| v.as_u64()),
        caller: AlloyAddress::new(get_address_slice(&caller_addr)),
        gas_price: option_u256_to_uint_or_zero(gas_price),
        gas_limit: tx.gas.map(|v| v.as_u64()).unwrap_or(0),  
        transact_to: to_tx_kind(tx.to),
        value: option_u256_to_uint_or_zero(tx.value),
        data: tx.data.map(|b| AlloyBytes::from(b.to_vec())).unwrap_or_default(), 
        access_list: ethers_to_alloy_access_list(tx.access_list),
        ..Default::default()
    }
}


pub fn adapt_transaction(decoded_tx: DecodedTransaction) -> TxEnv {
    match decoded_tx {
        DecodedTransaction::Legacy(tx, caller) => legacy_transaction_2_tx_env(tx, caller),
        DecodedTransaction::Eip1559(tx, caller) => eip1559_transaction_2_tx_env(tx, caller),
        _ => {
            panic!("Unsupported transaction type for adaptation: {:?}", decoded_tx);
        }
    }
}
