use ethers::types::{
    transaction::eip2718::TypedTransaction,
    transaction::eip1559::Eip1559TransactionRequest,
    transaction::eip2930::AccessList,
    TransactionRequest, NameOrAddress, Address, U256, Bytes, Signature
};
use ethers::signers::{LocalWallet, Signer};
use rlp::Encodable;
use std::str::FromStr;
use rand;

/// Generate and sign a transaction, either EIP-1559 or Legacy
pub async fn build_and_sign_tx(
    wallet: &LocalWallet,
    use_legacy: bool,
    to_addr: &str,
    value: U256,
    gas_limit: u64,
    gas_price_or_priority_fee: Option<u64>,
    max_fee_per_gas: Option<u64>,
    nonce: u64,
) -> Result<String, Box<dyn std::error::Error>> {
    let to = Address::from_str(to_addr)?;

    let typed_tx: TypedTransaction = if use_legacy {
        // Legacy 交易
        TransactionRequest {
            from: None,
            to: Some(NameOrAddress::Address(to)),
            gas: Some(gas_limit.into()),
            gas_price: None,
            value: Some(value),
            data: Some(Vec::new().into()),
            nonce: Some(nonce.into()),
            ..Default::default()
        }
        .into()
    } else {
        // EIP-1559 交易
        Eip1559TransactionRequest {
            from: None,
            to: Some(NameOrAddress::Address(to)),
            gas: Some(gas_limit.into()),
            value: Some(value),
            data: Some(Vec::new().into()),
            nonce: Some(nonce.into()),
            chain_id: Some(1u64.into()),
            max_priority_fee_per_gas: gas_price_or_priority_fee.map(U256::from),
            max_fee_per_gas: max_fee_per_gas.map(U256::from),
            access_list: AccessList::default(),
        }
        .into()
    };

    let raw_bytes_us = typed_tx.rlp();
    let raw_tx_hex_us = format!("0x{}", hex::encode(raw_bytes_us));
    println!("unsigned raw tx: {}", raw_tx_hex_us);

    let sig: Signature = wallet.sign_transaction(&typed_tx).await?;
    let signed_tx = typed_tx.clone().rlp_signed(&sig);
    let raw_tx_hex = format!("0x{}", hex::encode(signed_tx));
    Ok(raw_tx_hex)
}

pub async fn build_tx_unsigned(
    use_legacy: bool,
    to_addr: &str,
    value: U256,
    gas_limit: u64,
    gas_price_or_priority_fee: Option<u64>,
    max_fee_per_gas: Option<u64>,
    nonce: u64,
) -> Result<String, Box<dyn std::error::Error>> {
    let to = Address::from_str(to_addr)?;

    let typed_tx: TypedTransaction = if use_legacy {
        // Legacy
        TransactionRequest {
            from: None,
            to: Some(NameOrAddress::Address(to)),
            gas: Some(gas_limit.into()),
            gas_price: gas_price_or_priority_fee.map(U256::from),
            value: Some(value),
            data: Some(Vec::new().into()),
            nonce: Some(nonce.into()),
            ..Default::default()
        }
        .into()
    } else {
        // EIP-1559
        Eip1559TransactionRequest {
            from: None,
            to: Some(NameOrAddress::Address(to)),
            gas: Some(gas_limit.into()),
            value: Some(value),
            data: Some(Vec::new().into()),
            nonce: Some(nonce.into()),
            chain_id: Some(1u64.into()),
            max_priority_fee_per_gas: gas_price_or_priority_fee.map(U256::from),
            max_fee_per_gas: max_fee_per_gas.map(U256::from),
            access_list: AccessList::default(),
        }
        .into()
    };

    let raw_bytes = typed_tx.rlp();
    let raw_tx_hex = format!("0x{}", hex::encode(raw_bytes));

    Ok(raw_tx_hex)
}

pub async fn test() -> Result<(), Box<dyn std::error::Error>> {
    // A wallet with random private key
    let wallet = LocalWallet::new(&mut rand::thread_rng()).with_chain_id(1u64);

    // Test: Generate and sign EIP-1559 transaction
    let raw_1559 = build_tx_unsigned(
        false, // false = EIP-1559
        "0xd8da6bf26964af9d7eed9e03e53415d37aa96045",
        U256::from(100000000000000000u128), // 0.1 ETH
        21000,
        Some(1_500_000_000u64), // MaxPriorityFeePerGas / GasPrice
        Some(2_000_000_000u64), // MaxFeePerGas / GasPrice
        0, // Nonce
    )
    .await?;
    println!("unsigned EIP-1559 raw tx: {}", raw_1559);

    // Test: Generate an unsigned EIP-1559 transaction
    let raw_1559_signed = build_and_sign_tx(
        &wallet,
        false, // false = EIP-1559
        "0xd8da6bf26964af9d7eed9e03e53415d37aa96045",
        U256::from(100000000000000000u128), // 0.1 ETH
        21000,
        Some(1_500_000_000u64), // MaxPriorityFeePerGas / GasPrice
        Some(2_000_000_000u64), // MaxFeePerGas / GasPrice
        0, // Nonce
    )
    .await?;
    println!("");
    println!("sigend EIP-1559 raw tx: {}", raw_1559_signed);
    println!("");

    // Test: Generate and sign Legacy transaction
    let raw_legacy = build_and_sign_tx(
        &wallet,
        true, // true = Legacy
        "0xd8da6bf26964af9d7eed9e03e53415d37aa96045",
        U256::from(200000000000000000u128), // 0.2 ETH
        21000,
        Some(20_000_000_000u64), // GasPrice
        None,                    // MaxFeePerGas (Ignored in legacy)
        1, // Nonce
    )
    .await?;
    println!("Legacy raw tx: {}", raw_legacy);

    Ok(())
}
