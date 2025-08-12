// Cargo.toml 需添加：
// [dependencies]
// rlp = "0.5"
// ethereum-types = "0.14"
// hex = "0.4"

// pub mod adapter;
pub use super::adapter::{TxTo, TxKind, TxUnsigned, Signature, AccessListItem, from_revm_txenv};
pub use revm::primitives::{TxEnv, TransactTo, Bytes};

pub use ethereum_types::{H160, H256, U256, U64};
use revm::primitives::U256 as R256;
use rlp::RlpStream;

/// ------- 辅助编码 --------

fn rlp_opt_address(s: &mut RlpStream, to: &TxTo) {
    match to {
        TxTo::Call(addr) => s.append(addr),
        TxTo::Create     => s.append_empty_data(),
    };
}

fn rlp_access_list(s: &mut RlpStream, al: &[AccessListItem]) {
    s.begin_list(al.len());
    for item in al {
        s.begin_list(2);
        s.append(&item.address.as_slice());
        s.begin_list(item.storage_keys.len());
        for k in &item.storage_keys {
            s.append(&k.as_slice());
        }
    }
}

/// ------- 1) 生成“用于签名”的序列化载荷 -------
/// 返回：需要做 keccak256 的字节（对 legacy 与 1559 不同）
pub fn serialize_for_signing(tx: &TxUnsigned) -> Vec<u8> {
    match tx.kind {
        TxKind::Legacy => {
            // EIP-155 签名摘要是对以下 RLP 的 keccak256：
            // [nonce, gasPrice, gasLimit, to, value, data, chainId, 0, 0]
            let gas_price = tx.gas_price.expect("legacy must set gas_price");
            let mut s = RlpStream::new_list(9);
            s.append(&tx.nonce);
            s.append(&gas_price);
            s.append(&tx.gas_limit);
            rlp_opt_address(&mut s, &tx.to);
            s.append(&tx.value);
            s.append(&tx.data);
            s.append(&tx.chain_id);
            s.append(&0u8);
            s.append(&0u8);
            s.out().to_vec()
        }
        TxKind::Eip1559 => {
            // EIP-1559 的签名摘要是：
            // keccak256( 0x02 || rlp([chainId, nonce, maxPriorityFeePerGas, maxFeePerGas,
            //                          gasLimit, to, value, data, accessList]) )
            let max_p = tx.max_priority_fee_per_gas.expect("1559 must set max_priority_fee_per_gas");
            let max_f = tx.max_fee_per_gas.expect("1559 must set max_fee_per_gas");
            let mut inner = RlpStream::new_list(9);
            inner.append(&tx.chain_id);
            inner.append(&tx.nonce);
            inner.append(&max_p);
            inner.append(&max_f);
            inner.append(&tx.gas_limit);
            rlp_opt_address(&mut inner, &tx.to);
            inner.append(&tx.value);
            inner.append(&tx.data);
            rlp_access_list(&mut inner, &tx.access_list);
            let mut out = Vec::with_capacity(1 + inner.as_raw().len());
            out.push(0x02);
            out.extend_from_slice(inner.as_raw());
            out
        }
    }
}

/// ------- 2) 生成“已签名”的完整交易字节流 -------
/// 注意：EIP-1559 需前缀 0x02；legacy 无类型前缀
pub fn serialize_signed(tx: &TxUnsigned, sig: &Signature) -> Vec<u8> {
    match tx.kind {
        TxKind::Legacy => {
            // Legacy 交易 RLP：
            // [nonce, gasPrice, gasLimit, to, value, data, v, r, s]
            // 其中 v 已含 EIP-155：v = (recovery_id + 35 + chain_id*2)
            let gas_price = tx.gas_price.expect("legacy must set gas_price");
            let mut s = RlpStream::new_list(9);
            s.append(&tx.nonce);
            s.append(&gas_price);
            s.append(&tx.gas_limit);
            rlp_opt_address(&mut s, &tx.to);
            s.append(&tx.value);
            s.append(&tx.data);
            s.append(&sig.v);
            s.append(&sig.r);
            s.append(&sig.s);
            s.out().to_vec()
        }
        TxKind::Eip1559 => {
            // EIP-1559 封包：
            // 0x02 || rlp([chainId, nonce, maxPriorityFeePerGas, maxFeePerGas,
            //             gasLimit, to, value, data, accessList, yParity, r, s])
            let max_p = tx.max_priority_fee_per_gas.expect("1559 must set max_priority_fee_per_gas");
            let max_f = tx.max_fee_per_gas.expect("1559 must set max_fee_per_gas");
            let mut inner = RlpStream::new_list(12);
            inner.append(&tx.chain_id);
            inner.append(&tx.nonce);
            inner.append(&max_p);
            inner.append(&max_f);
            inner.append(&tx.gas_limit);
            rlp_opt_address(&mut inner, &tx.to);
            inner.append(&tx.value);
            inner.append(&tx.data);
            rlp_access_list(&mut inner, &tx.access_list);
            // EIP-1559 用 yParity(0/1) 代替 legacy v 扩展值
            inner.append(&sig.v); // 这里应是 0 或 1
            inner.append(&sig.r);
            inner.append(&sig.s);

            let mut out = Vec::with_capacity(1 + inner.as_raw().len());
            out.push(0x02);
            out.extend_from_slice(inner.as_raw());
            out
        }
    }
}


pub fn try_one_serialization() {
    // 一个简单的 EIP-1559 转账示例（未签名载荷）
    let tx = TxUnsigned {
        kind: TxKind::Eip1559,
        chain_id: U64::from(1),
        nonce: 0u64.into(),
        gas_price: None,
        max_priority_fee_per_gas: Some(U256::from(1_000_000_000u64)), // 1 gwei
        max_fee_per_gas: Some(U256::from(30_000_000_000u64)),         // 30 gwei
        gas_limit: U64::from(21_000),
        to: TxTo::Call("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".parse().unwrap_or(H160::zero())),
        value: U256::from(1_000_000_000_000_000u64), // 0.001 ETH
        data: vec![],
        access_list: vec![],
    };

    let signing_payload = serialize_for_signing(&tx);
    println!("1559 signing payload (hex): 0x{}", hex::encode(signing_payload));

    // 假设你已使用 secp256k1 对上述摘要签名并拿到 (yParity, r, s)
    let sig = Signature {
        v: 0,                                // yParity: 0 或 1
        r: U256::from_dec_str("1").unwrap(), // 示例占位
        s: U256::from_dec_str("2").unwrap(),
    };

    let raw = serialize_signed(&tx, &sig);
    println!("raw tx (hex): 0x{}", hex::encode(raw));
}


pub fn try_one_adapt() {
    let tx = TxEnv {
        chain_id: Some(1),
        nonce: Some(0),
        gas_price: R256::from(1000),
        gas_priority_fee: Some(R256::from(10)),
        gas_limit: 21_000,
        transact_to: TransactTo::Call("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".parse().unwrap()),
        value: R256::from(1_000_000_000_000_000u64),
        data: Bytes::new(),
        access_list: vec![],
        // 如果你的 TxEnv 有额外字段，这里需要加上
        ..Default::default()
    };

    let unsigned = from_revm_txenv(&tx).expect("Failed to adapt TxEnv");

    let signing_payload = serialize_for_signing(&unsigned);
    println!("Adapted signing payload (hex): 0x{}", hex::encode(signing_payload));

    // 假设你已使用 secp256k1 对上述摘要签名并拿到 (yParity, r, s)
    let sig = Signature {
        v: 0,                                // yParity: 0 或 1
        r: U256::from_dec_str("1").unwrap(), // 示例占位
        s: U256::from_dec_str("2").unwrap(),
    };

    let raw = serialize_signed(&unsigned, &sig);
    println!("raw tx (hex): 0x{}", hex::encode(raw));
}