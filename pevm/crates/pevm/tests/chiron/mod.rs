use pevm::{Bytecodes, EvmAccount};
use revm::primitives::{Address, U256, B256, TransactTo, TxEnv, AccessListItem};
use rand::{thread_rng, Rng};
use std::collections::HashMap;
use alloy_primitives::{U128};
use contract::Chiron;
use sol::{COST_DISTR, LEN_DISTR, RES_DISTR};
use rand::distributions::{Distribution, WeightedIndex};

const GAS_LIMIT: u64 = 100_000_000_000;
// 6803268
// 6697512
#[path = "../data/solana_distribution.rs"]
pub mod sol;

#[path = "../common/mod.rs"]
pub mod common;

pub mod contract;




fn generate_addresses(length: usize) -> Vec<Address> {
    (0..length).map(|_| Address::new(rand::random())).collect()
}

/// Generate only `exchange` workload
pub fn generate_exchange(num_tx: usize) -> (HashMap<Address, EvmAccount>, Bytecodes, Vec<TxEnv>) {
    let accounts: Vec<Address> = generate_addresses(num_tx);
    let chiron_address = Address::new(rand::random());

    let chiron_account = Chiron::build();
    let mut state = HashMap::from([(chiron_address, chiron_account)]);
    let mut txs = Vec::new();

    for account in &accounts {
        state.insert(
            *account,
            EvmAccount {
                balance: U256::from(U128::MAX),
                ..EvmAccount::default()
            },
        );
    }

    let mut rng = thread_rng();
    let mut sender_map = HashMap::new();

    for _ in 0..num_tx {
        // todo take based on rng distribution.
        let idx = rng.gen_range(0..accounts.len());
        let person = accounts[idx];

        let nonce = sender_map.get(&person).unwrap_or(&0);
        let calldata = Chiron::exchange(U256::from(idx));

        txs.push(TxEnv {
            caller: person,
            gas_limit: GAS_LIMIT,
            gas_price: U256::from(1),
            transact_to: TransactTo::Call(chiron_address),
            data: calldata,
            nonce: Some(*nonce),
            access_list: vec![AccessListItem {
                address: person,
                storage_keys: vec![B256::from(U256::from(idx))],
            }],
            ..TxEnv::default()
        });

        sender_map.insert(person, nonce + 1);
    }

    let mut bytecodes = Bytecodes::default();
    for account in state.values_mut() {
        if let Some(code) = account.code.take() {
            bytecodes.insert(account.code_hash.unwrap(), code);
        }
    }

    (state, bytecodes, txs)
}

/// Generate only `exchangetwo` workload
pub fn generate_exchange_two(num_tx: usize) -> (HashMap<Address, EvmAccount>, Bytecodes, Vec<TxEnv>) {
    let accounts: Vec<Address> = generate_addresses(num_tx);
    let chiron_address = Address::new(rand::random());

    let chiron_account = Chiron::build();
    let mut state = HashMap::from([(chiron_address, chiron_account)]);
    let mut txs = Vec::new();

    for account in &accounts {
        state.insert(
            *account,
            EvmAccount {
                balance: U256::from(1_000_000_000_000_000_000u128),
                ..EvmAccount::default()
            },
        );
    }

    let mut rng = thread_rng();
    let mut sender_map = HashMap::new();

    for _ in 0..num_tx {
        // todo take based on rng distribution.
        let sender_idx = rng.gen_range(0..accounts.len());
        let person = accounts[sender_idx];
        let nonce = sender_map.get(&person).unwrap_or(&0);

        let receiver_idx = rng.gen_range(0..accounts.len()) ;
        let calldata = Chiron::exchange_two(U256::from(sender_idx), U256::from(receiver_idx));

        txs.push(TxEnv {
            caller: person,
            gas_limit: GAS_LIMIT,
            gas_price: U256::from(1),
            transact_to: TransactTo::Call(chiron_address),
            data: calldata,
            nonce: Some(*nonce),
            access_list: vec![AccessListItem {
                address: person,
                storage_keys: vec![B256::from(U256::from(sender_idx)), B256::from(U256::from(receiver_idx))],
            }],
            ..TxEnv::default()
        });

        sender_map.insert(person, nonce + 1);
    }

    let mut bytecodes = Bytecodes::default();
    for account in state.values_mut() {
        if let Some(code) = account.code.take() {
            bytecodes.insert(account.code_hash.unwrap(), code);
        }
    }

    (state, bytecodes, txs)
}

/// Generate only `loop_exchange` workload
pub fn generate_loop_exchange(num_tx: usize) -> (HashMap<Address, EvmAccount>, Bytecodes, Vec<TxEnv>) {
    let accounts: Vec<Address> = generate_addresses(num_tx);
    let chiron_address = Address::new(rand::random());

    let chiron_account = Chiron::build();
    let mut state = HashMap::from([(chiron_address, chiron_account)]);
    let mut txs = Vec::new();

    for account in &accounts {
        state.insert(
            *account,
            EvmAccount {
                balance: U256::from(U128::MAX),
                ..EvmAccount::default()
            },
        );
    }

    let mut rng = thread_rng();
    let mut sender_map = HashMap::new();
    let res_distribution: WeightedIndex<f64> = WeightedIndex::new(&RES_DISTR).unwrap();

    for x in 0..num_tx {
        let person = accounts[rng.gen_range(0..accounts.len())];
        let nonce = sender_map.get(&person).unwrap_or(&0);

        let cost_sample = COST_DISTR[rand::thread_rng().gen_range(0..COST_DISTR.len())];
        let write_len_sample = LEN_DISTR[rand::thread_rng().gen_range(0..LEN_DISTR.len())] as usize;
        let mut writes= Vec::new();
        for _ in 0..write_len_sample {
            writes.push(res_distribution.sample(&mut rng)) ;
        }

        //println!("writes {:?}", &writes);

        let cost = U256::from(cost_sample.round() as u64);
        let calldata = Chiron::loop_exchange(cost, &writes);

        let mut write_keys:Vec<AccessListItem> = Vec::new();
        for write in writes {
            let slot = U256::from(write);

            write_keys.push(AccessListItem {
                address: chiron_address,
                storage_keys: vec!(B256::from(slot)),
            });
        }

        write_keys.push(AccessListItem {
            address: person.clone(),
            storage_keys: vec!(B256::ZERO),
        });

        txs.push(TxEnv {
            caller: person,
            gas_limit: GAS_LIMIT * cost_sample as u64,
            gas_price: U256::from(1),
            transact_to: TransactTo::Call(chiron_address),
            data: calldata,
            nonce: Some(*nonce),
            access_list: write_keys,
            ..TxEnv::default()
        });

        sender_map.insert(person, nonce + 1);
    }

    let mut bytecodes = Bytecodes::default();
    for account in state.values_mut() {
        if let Some(code) = account.code.take() {
            bytecodes.insert(account.code_hash.unwrap(), code);
        }
    }

    (state, bytecodes, txs)
}
