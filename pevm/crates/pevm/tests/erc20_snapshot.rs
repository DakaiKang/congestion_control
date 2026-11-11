//! Produce the ERC20 state snapshot the Mysticeti prototype loads at start-up.
//! `ERC20_CLUSTERS=… ERC20_FAMILIES=… ERC20_PEOPLE=… cargo test --release --test erc20_snapshot -- --nocapture`
#[test]
fn write_erc20_snapshot() {
    let e = |k: &str, d: usize| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    let (c, f, p) = (e("ERC20_CLUSTERS", 1), e("ERC20_FAMILIES", 1), e("ERC20_PEOPLE", 4));
    pevm::api::write_erc20_snapshot(c, f, p).expect("snapshot");
    println!("wrote storage_{c}_{f}_{p}.json and account_addresses_{c}_{f}_{p}.bin");
}
