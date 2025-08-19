use pevm::serialization::serializer;
use pevm::serialization::deserializer;
use pevm::serialization::adapter;
pub use ethers::types::{Address, H256, U256};
pub use ethers::signers::{LocalWallet, Signer};
pub use hex::FromHex;
pub use rlp::Rlp;
pub use std::env;

// #[test]
// fn test_serializer() {
//     serializer::test();
// }

#[tokio::test]
async fn test_serializer() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = LocalWallet::new(&mut rand::thread_rng()).with_chain_id(1u64);
    let pubkey = wallet.signer().verifying_key(); // 返回 k256::PublicKey
    let pubkey_uncompressed = pubkey.to_encoded_point(false);
    println!("0x{}", hex::encode(pubkey_uncompressed.as_bytes()));
    println!("Address: {:?}", wallet.address());

    let raw_1559_signed = serializer::build_and_sign_tx(
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


    let deserialized_tx = deserializer::decode_hex(&raw_1559_signed);

    println!("{:#?}", deserialized_tx);

    let tx_env = adapter::adapt_transaction(deserialized_tx);

    println!("{:#?}", tx_env);

    Ok(())
}