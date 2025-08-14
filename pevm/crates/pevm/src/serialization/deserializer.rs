use ethers::types::{
    transaction::eip2718::TypedTransaction,
    transaction::eip1559::Eip1559TransactionRequest,
    transaction::eip2930::AccessList,
    TransactionRequest, NameOrAddress, Address, U256, Bytes, Signature, H256
};
use ethers::core::k256::{
    ecdsa::{Signature as K256Signature, SigningKey, VerifyingKey},
    elliptic_curve::sec1::ToEncodedPoint,
};
use ethers::utils::keccak256;

pub use hex::FromHex;
pub use rlp::{Rlp, RlpStream};
pub use std::env;

fn u256_from_bytes(b: &[u8]) -> U256 {
    if b.is_empty() {
        U256::from(0)
    } else {
        U256::from_big_endian(b)
    }
}

fn parse_to_address(b: &[u8]) -> Option<Address> {
    if b.is_empty() {
        None
    } else {
        // RLP Address is 20 bytes
        if b.len() != 20 {
            // The last 20 bytes are the address
            if b.len() > 20 {
                let mut a = [0u8; 20];
                a.copy_from_slice(&b[b.len() - 20..]);
                Some(Address::from(a))
            } else {
                None
            }
        } else {
            let mut a = [0u8; 20];
            a.copy_from_slice(b);
            Some(Address::from(a))
        }
    }
}


fn hexify(b: &[u8]) -> String {
    format!("0x{}", hex::encode(b))
}

/// Parse the access list from RLP
fn parse_access_list(rlp: &rlp::Rlp, idx: usize) -> Vec<(ethers::types::Address, Vec<ethers::types::H256>)> {
    let mut out = Vec::new();

    // accessList is a list of tuples: [address, [storageKeys...]]
    let access_list_rlp = rlp.at(idx).expect("accessList rlp");

    for item in access_list_rlp.iter() {
        // item = [ address, [storageKeys...] ]
        let addr_bytes = item.at(0).unwrap().data().unwrap_or_default();
        let addr = {
            use ethers::types::Address;
            if addr_bytes.len() == 20 {
                let mut a = [0u8; 20];
                a.copy_from_slice(addr_bytes);
                Address::from(a)
            } else {
                // The last 20 bytes are the address
                let mut a = [0u8; 20];
                let start = addr_bytes.len().saturating_sub(20);
                a.copy_from_slice(&addr_bytes[start..]);
                Address::from(a)
            }
        };

        // List of storageKeys 
        let mut keys = Vec::new();
        let storage_rlp = item.at(1).unwrap();
        for k in storage_rlp.iter() {
            let kb = k.data().unwrap_or_default();
            let mut arr = [0u8; 32];
            if kb.len() <= 32 {
                arr[32 - kb.len()..].copy_from_slice(kb);
                keys.push(ethers::types::H256::from(arr));
            }
        }

        out.push((addr, keys));
    }

    out
}

fn recover_address(
    r_sig: U256,
    s_sig: U256,
    v: U256,
    unsigned: Vec<u8>,
) -> Result<Address, Box<dyn std::error::Error>> {
    let sig = Signature {
        r: r_sig,
        s: s_sig,
        v: v.as_u64(),
    };

    let sighash = ethers::utils::keccak256(&unsigned);
    let recovered_addr = sig.recover(sighash).expect("recover failed");
    println!("Recovered from: {:?}", recovered_addr);
    Ok(recovered_addr)
}


fn decode_legacy(bytes: &[u8], no_signature: Vec<u8>) {
    let r = Rlp::new(bytes);

    let nonce = u256_from_bytes(r.at(0).unwrap().data().unwrap_or_default());
    let gas_price = u256_from_bytes(r.at(1).unwrap().data().unwrap_or_default());
    let gas_limit = u256_from_bytes(r.at(2).unwrap().data().unwrap_or_default());
    let to_b = r.at(3).unwrap().data().unwrap_or_default();
    let to = parse_to_address(to_b);
    let value = u256_from_bytes(r.at(4).unwrap().data().unwrap_or_default());
    let data = r.at(5).unwrap().data().unwrap_or_default();

    println!("# Legacy (type 0)");
    println!("nonce                : {}", nonce);
    println!("gasPrice (wei)       : {}", gas_price);
    println!("gasLimit             : {}", gas_limit);
    println!("to                   : {}", to.map(|a| format!("{a:?}")).unwrap_or_else(|| "<create>".into()));
    println!("value (wei)          : {}", value);
    println!("data                 : {}", hexify(data));

    if r.item_count().unwrap_or(0) >= 9 {
        let v = u256_from_bytes(r.at(6).unwrap().data().unwrap_or_default());
        let r_sig = u256_from_bytes(r.at(7).unwrap().data().unwrap_or_default());
        let s_sig = u256_from_bytes(r.at(8).unwrap().data().unwrap_or_default());
        println!("v                    : {}", v);
        println!("r                    : 0x{:064x}", r_sig);
        println!("s                    : 0x{:064x}", s_sig);

        recover_address(r_sig, s_sig, v, no_signature).expect("Failed to recover address");
    }
}

fn decode_eip2930(inner: &[u8], no_signature: Vec<u8>) {
    // RLP: [chainId, nonce, gasPrice, gasLimit, to, value, data, accessList, v?, r?, s?]
    let r = Rlp::new(inner);
    let chain_id = u256_from_bytes(r.at(0).unwrap().data().unwrap_or_default());
    let nonce = u256_from_bytes(r.at(1).unwrap().data().unwrap_or_default());
    let gas_price = u256_from_bytes(r.at(2).unwrap().data().unwrap_or_default());
    let gas_limit = u256_from_bytes(r.at(3).unwrap().data().unwrap_or_default());
    let to = parse_to_address(r.at(4).unwrap().data().unwrap_or_default());
    let value = u256_from_bytes(r.at(5).unwrap().data().unwrap_or_default());
    let data = r.at(6).unwrap().data().unwrap_or_default();
    let access_list = parse_access_list(&r, 7);
    

    println!("# EIP-2930 (type 0x01)");
    println!("chainId              : {}", chain_id);
    println!("nonce                : {}", nonce);
    println!("gasPrice (wei)       : {}", gas_price);
    println!("gasLimit             : {}", gas_limit);
    println!("to                   : {}", to.map(|a| format!("{a:?}")).unwrap_or_else(|| "<create>".into()));
    println!("value (wei)          : {}", value);
    println!("data                 : {}", hexify(data));
    println!("accessList           : [{} items]", access_list.len());

    if r.item_count().unwrap_or(0) >= 10 {
        let v = u256_from_bytes(r.at(8).unwrap().data().unwrap_or_default());
        let r_sig = u256_from_bytes(r.at(9).unwrap().data().unwrap_or_default());
        let s_sig = u256_from_bytes(r.at(10).unwrap().data().unwrap_or_default());
        println!("v                    : {}", v);
        println!("r                    : 0x{:064x}", r_sig);
        println!("s                    : 0x{:064x}", s_sig);

        let sig = Signature {
            r: r_sig,
            s: s_sig,
            v: v.as_u64(),
        };

        recover_address(r_sig, s_sig, v, no_signature).expect("Failed to recover address");
    }
}

fn u256_to_h256(val: U256) -> H256 {
    let mut bytes = [0u8; 32];
    val.to_big_endian(&mut bytes);
    H256::from(bytes)
}

fn decode_eip1559(inner: &[u8], no_signature: Vec<u8>) {
    // RLP: [chainId, nonce, maxPriorityFeePerGas, maxFeePerGas, gasLimit, to, value, data, accessList, v?, r?, s?]
    let r = Rlp::new(inner);
    let chain_id = u256_from_bytes(r.at(0).unwrap().data().unwrap_or_default());
    let nonce = u256_from_bytes(r.at(1).unwrap().data().unwrap_or_default());
    let max_priority = u256_from_bytes(r.at(2).unwrap().data().unwrap_or_default());
    let max_fee = u256_from_bytes(r.at(3).unwrap().data().unwrap_or_default());
    let gas_limit = u256_from_bytes(r.at(4).unwrap().data().unwrap_or_default());
    let to = parse_to_address(r.at(5).unwrap().data().unwrap_or_default());
    let value = u256_from_bytes(r.at(6).unwrap().data().unwrap_or_default());
    let data = r.at(7).unwrap().data().unwrap_or_default();
    let access_list = parse_access_list(&r, 8);

    println!("# EIP-1559 (type 0x02)");
    println!("chainId              : {}", chain_id);
    println!("nonce                : {}", nonce);
    println!("maxPriorityFeePerGas : {}", max_priority);
    println!("maxFeePerGas         : {}", max_fee);
    println!("gasLimit             : {}", gas_limit);
    println!("to                   : {}", to.map(|a| format!("{a:?}")).unwrap_or_else(|| "<create>".into()));
    println!("value (wei)          : {}", value);
    println!("data                 : {}", hexify(data));
    println!("accessList           : [{} items]", access_list.len());
    
    // Recover sender address if available
    if r.item_count().unwrap_or(0) >= 11 {
        let v = u256_from_bytes(r.at(9).unwrap().data().unwrap_or_default());
        let r_sig = u256_from_bytes(r.at(10).unwrap().data().unwrap_or_default());
        let s_sig = u256_from_bytes(r.at(11).unwrap().data().unwrap_or_default());
        println!("v                    : {}", v);
        println!("r                    : 0x{:064x}", r_sig);
        println!("s                    : 0x{:064x}", s_sig);
        
        recover_address(r_sig, s_sig, v, no_signature).expect("Failed to recover address");
    }

}

fn strip_signature(raw_bytes: &[u8]) -> Vec<u8> {
    // Check if the first byte indicates a typed transaction (EIP-2718)
    let (tx_type, rlp_bytes) = if !raw_bytes.is_empty() && (raw_bytes[0] == 0x02 || raw_bytes[0] == 0x01) {
        (Some(raw_bytes[0]), &raw_bytes[1..])
    } else {
        (None, raw_bytes)
    };

    // Decode RLP
    let rlp = Rlp::new(rlp_bytes);
    let total_items = rlp.item_count().expect("invalid RLP");

    if total_items < 3 {
        panic!("Not enough fields to strip v/r/s");
    }

    // Remove v, r, s from the RLP
    let mut stream = RlpStream::new_list(total_items - 3);
    for i in 0..(total_items - 3) {
        stream.append_raw(rlp.at(i).unwrap().as_raw(), 1);
    }

    // Add the tx type if it exists
    let mut out = Vec::new();
    if let Some(t) = tx_type {
        out.push(t);
    }
    out.extend(stream.out().to_vec());
    out
}


pub fn decode_hex(raw_hex: &str) {
    // Remove "0x" prefix if present
    let h = raw_hex.trim_start_matches("0x");
    let bytes = Vec::from_hex(h).expect("invalid hex");

    if bytes.is_empty() {
        eprintln!("empty bytes");
        return;
    }

    let no_signature = strip_signature(bytes.as_slice());
    println!("raw bytes without signature: {}", hexify(&no_signature));

    match bytes[0] {
        0x01 => {
            // EIP-2930
            if bytes.len() < 2 {
                eprintln!("malformed 2930");
                return;
            }
            decode_eip2930(&bytes[1..], no_signature);
        }
        0x02 => {
            // EIP-1559
            if bytes.len() < 2 {
                eprintln!("malformed 1559");
                return;
            }
            decode_eip1559(&bytes[1..], no_signature);
        }
        b if b >= 0xc0 => {
            // Legacy
            decode_legacy(&bytes, no_signature);
        }
        _ => {
            decode_legacy(&bytes, no_signature);
        }
    }
}
