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

    println!("chiron address: {}", chiron_address);
    println!("accounts number: {}", accounts.len());

    // Initiate num_tx accounts. Each account has a balance U128:MAX
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
    // Maintain the nonce of each account
    let mut sender_map = HashMap::new();
    let res_distribution: WeightedIndex<f64> = WeightedIndex::new(&RES_DISTR).unwrap();

    for x in 0..num_tx {
        // For each transaction, randomly select one account as the sender
        let person = accounts[rng.gen_range(0..accounts.len())];
        let nonce = sender_map.get(&person).unwrap_or(&0);

        let cost_sample = COST_DISTR[rand::thread_rng().gen_range(0..COST_DISTR.len())];
        let write_len_sample = LEN_DISTR[rand::thread_rng().gen_range(0..LEN_DISTR.len())] as usize;
        let mut writes= Vec::new();
        for _ in 0..write_len_sample {
            writes.push(res_distribution.sample(&mut rng)) ;
        }
        // println!("cost_smaple: {:?}", cost_sample);
        // println!("write_len_sample: {:?}", write_len_sample);
        // println!("writes {:?}", &writes);

        let cost = U256::from(cost_sample.round() as u64);
        let calldata = Chiron::loop_exchange(cost, &writes);

        let mut write_keys:Vec<AccessListItem> = Vec::new();
        // for write in writes {
        //     let slot = U256::from(write);

        //     write_keys.push(AccessListItem {
        //         address: chiron_address,
        //         storage_keys: vec!(B256::from(slot)),
        //     });
        // }

        // write_keys.push(AccessListItem {
        //     address: person.clone(),
        //     storage_keys: vec!(B256::ZERO),
        // });

        println!("gas limit is {:#?}", GAS_LIMIT * cost_sample as u64);

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

/// Generate `batch_num` batches of `loop_exchange` workload, each batch containing `num_tx` transactions
pub fn generate_loop_exchange_batches(
    num_tx: usize,
    batch_num: usize,
) -> (HashMap<Address, EvmAccount>, Bytecodes, Vec<Vec<TxEnv>>) {
    let accounts: Vec<Address> = generate_addresses(num_tx);
    let chiron_address = Address::new(rand::random());

    let chiron_account = Chiron::build();
    let mut state = HashMap::from([(chiron_address, chiron_account)]);
    let mut all_batches = Vec::new();

    println!("chiron address: {}", chiron_address);
    println!("accounts number: {}", accounts.len());
    println!("batch number: {}", batch_num);

    // Initiate num_tx accounts. Each account has a balance U128:MAX
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
    // Maintain the nonce of each account across all batches
    let mut sender_map = HashMap::new();
    let res_distribution: WeightedIndex<f64> = WeightedIndex::new(&RES_DISTR).unwrap();
    // let sender_num = 4;
    // Generate batch_num batches
    for batch_idx in 0..batch_num {
        let mut txs = Vec::new();

        for x in 0..num_tx {
            // For each transaction, randomly select one account as the sender
            // let person = accounts[rng.gen_range(0..accounts.len())];
            let range_length = accounts.len() / batch_num;
            let person_id = batch_idx * range_length + (x % range_length);
            let person = accounts[person_id];
            let nonce = sender_map.get(&person).unwrap_or(&0);

            let cost_sample = COST_DISTR[rand::thread_rng().gen_range(0..COST_DISTR.len())];
            // let write_len_sample = LEN_DISTR[rand::thread_rng().gen_range(0..LEN_DISTR.len())] as usize;
            let write_len_sample = rand::thread_rng().gen_range(1..3) as usize;
            let mut writes = Vec::new();
            // for _ in 0..write_len_sample {
            //     writes.push(res_distribution.sample(&mut rng));
            // }
            // let write_len_sample = 1 as usize;
            // for _ in 0..write_len_sample-1 {
            //     writes.push(res_distribution.sample(&mut rng)) ;
            // }
            writes.push(person_id % 2 + 100 * batch_idx);
            println!("writes: {:?}", writes);
            // println!("cost_sample: {:?}", cost_sample);
            // println!("write_len_sample: {:?}", write_len_sample);
            // println!("writes {:?}", &writes);

            let cost = U256::from(cost_sample.round() as u64);
            let calldata = Chiron::loop_exchange(cost, &writes);

            let write_keys: Vec<AccessListItem> = Vec::new();

            // println!("gas limit is {:#?}", GAS_LIMIT * cost_sample as u64);

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

        all_batches.push(txs);
    }

    let mut bytecodes = Bytecodes::default();
    for account in state.values_mut() {
        if let Some(code) = account.code.take() {
            bytecodes.insert(account.code_hash.unwrap(), code);
        }
    }

    (state, bytecodes, all_batches)
}


/// Generate only `loop_exchange` workload
pub fn generate_loop_exchange_with_cost(num_tx: usize) -> (HashMap<Address, EvmAccount>, Bytecodes, Vec<TxEnv>, Vec<u64>) {
    let accounts: Vec<Address> = generate_addresses(num_tx);
    let chiron_address = Address::new(rand::random());

    let chiron_account = Chiron::build();
    let mut state = HashMap::from([(chiron_address, chiron_account)]);
    let mut txs = Vec::new();
    let mut costs = Vec::new();

    println!("chiron address: {}", chiron_address);
    println!("accounts number: {}", accounts.len());

    // Initiate num_tx accounts. Each account has a balance U128:MAX
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
    // Maintain the nonce of each account
    let mut sender_map = HashMap::new();
    let res_distribution: WeightedIndex<f64> = WeightedIndex::new(&RES_DISTR).unwrap();

    for x in 0..num_tx {
        // For each transaction, randomly select one account as the sender
        let person = accounts[rng.gen_range(0..accounts.len())];
        let nonce = sender_map.get(&person).unwrap_or(&0);

        let cost_sample = COST_DISTR[rand::thread_rng().gen_range(0..COST_DISTR.len())];
        // let write_len_sample = LEN_DISTR[rand::thread_rng().gen_range(0..LEN_DISTR.len())] as usize;
        let write_len_sample = rand::thread_rng().gen_range(1..3) as usize;
        let mut writes= Vec::new();
        for _ in 0..write_len_sample {
            writes.push(res_distribution.sample(&mut rng)) ;
        }
        // println!("cost_smaple: {:?}", cost_sample);
        // println!("write_len_sample: {:?}", write_len_sample);
        // println!("person: {}", person);
        // println!("writes {:?}", &writes);


        let cost = U256::from(cost_sample.round() as u64);
        costs.push(cost_sample.round() as u64);
        let calldata = Chiron::loop_exchange(cost, &writes);

        let mut write_keys:Vec<AccessListItem> = Vec::new();

        // println!("gas limit is {:#?}", GAS_LIMIT * cost_sample as u64);

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

    (state, bytecodes, txs, costs)
}


/// Generate only `loop_exchange` workload
pub fn generate_single_sender_loop_exchange(num_tx: usize) -> (HashMap<Address, EvmAccount>, Bytecodes, Vec<TxEnv>, Vec<u64>) {
    let accounts: Vec<Address> = generate_addresses(num_tx);
    let chiron_address = Address::new(rand::random());

    let chiron_account = Chiron::build();
    let mut state = HashMap::from([(chiron_address, chiron_account)]);
    let mut txs = Vec::new();
    let mut costs = Vec::new();

    println!("chiron address: {}", chiron_address);
    println!("accounts number: {}", accounts.len());

    // Initiate num_tx accounts. Each account has a balance U128:MAX
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
    // Maintain the nonce of each account
    let mut sender_map = HashMap::new();
    let res_distribution: WeightedIndex<f64> = WeightedIndex::new(&RES_DISTR).unwrap();

    for x in 0..num_tx {
        // For each transaction, randomly select one account as the sender
        let person = accounts[0];
        let nonce = sender_map.get(&person).unwrap_or(&0);

        let cost_sample = COST_DISTR[rand::thread_rng().gen_range(0..COST_DISTR.len())];
        // let write_len_sample = LEN_DISTR[rand::thread_rng().gen_range(0..LEN_DISTR.len())] as usize;
        let write_len_sample = rand::thread_rng().gen_range(1..3) as usize;
        let mut writes= Vec::new();
        for _ in 0..write_len_sample {
            writes.push(res_distribution.sample(&mut rng)) ;
        }
        // println!("cost_smaple: {:?}", cost_sample);
        // println!("write_len_sample: {:?}", write_len_sample);
        // println!("person: {}", person);
        // println!("writes {:?}", &writes);


        let cost = U256::from(cost_sample.round() as u64);
        costs.push(cost_sample.round() as u64);
        let calldata = Chiron::loop_exchange(cost, &writes);

        let mut write_keys:Vec<AccessListItem> = Vec::new();

        // println!("gas limit is {:#?}", GAS_LIMIT * cost_sample as u64);

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

    (state, bytecodes, txs, costs)
}


/// Generate only `loop_exchange` workload
pub fn generate_dense_multiple_sender_loop_exchange(num_tx: usize, sender_num: usize) -> (HashMap<Address, EvmAccount>, Bytecodes, Vec<TxEnv>, Vec<u64>) {
    let accounts: Vec<Address> = generate_addresses(num_tx);
    let chiron_address = Address::new(rand::random());

    let chiron_account = Chiron::build();
    let mut state = HashMap::from([(chiron_address, chiron_account)]);
    let mut txs = Vec::new();
    let mut costs = Vec::new();

    println!("chiron address: {}", chiron_address);
    println!("accounts number: {}", accounts.len());

    // Initiate num_tx accounts. Each account has a balance U128:MAX
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
    // Maintain the nonce of each account
    let mut sender_map = HashMap::new();
    let res_distribution: WeightedIndex<f64> = WeightedIndex::new(&RES_DISTR).unwrap();

    for x in 0..num_tx {
        // For each transaction, randomly select one account as the sender
        let person = accounts[x];
        let nonce = sender_map.get(&person).unwrap_or(&0);

        let cost_sample = COST_DISTR[rand::thread_rng().gen_range(0..COST_DISTR.len())];
        // let write_len_sample = LEN_DISTR[rand::thread_rng().gen_range(0..LEN_DISTR.len())] as usize;
        let write_len_sample = rand::thread_rng().gen_range(1..3) as usize;
        let mut writes= Vec::new();
        for _ in 0..write_len_sample-1 {
            writes.push(res_distribution.sample(&mut rng)) ;
        }
        writes.push(1000 * (x/sender_num));
        // println!("cost_smaple: {:?}", cost_sample);
        // println!("write_len_sample: {:?}", write_len_sample);
        // println!("person: {}", person);
        // println!("writes {:?}", &writes);

        let cost = U256::from(cost_sample.round() as u64);
        costs.push(cost_sample.round() as u64);
        let calldata = Chiron::loop_exchange(cost, &writes);

        let mut write_keys:Vec<AccessListItem> = Vec::new();

        // println!("gas limit is {:#?}", GAS_LIMIT * cost_sample as u64);

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

    (state, bytecodes, txs, costs)
}


/// Generate only `loop_exchange` workload
pub fn generate_loop_exchange_with_cost_two_batch(num_tx: usize) -> (HashMap<Address, EvmAccount>, Bytecodes, Vec<TxEnv>, Vec<u64>, Vec<TxEnv>, Vec<u64>) {
    let accounts: Vec<Address> = generate_addresses(num_tx * 2);
    let chiron_address = Address::new(rand::random());
    let chiron_account = Chiron::build();
    let chiron_address2 = Address::new(rand::random());
    let chiron_account2 = Chiron::build();
    let mut state = HashMap::from([(chiron_address, chiron_account), (chiron_address2, chiron_account2)]);

    let mut txs = Vec::new();
    let mut costs = Vec::new();
    let mut txs2 = Vec::new();
    let mut costs2 = Vec::new();

    println!("chiron address: {}", chiron_address);
    println!("accounts number: {}", accounts.len());

    // Initiate num_tx accounts. Each account has a balance U128:MAX
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
    // Maintain the nonce of each account
    let mut sender_map = HashMap::new();
    let res_distribution: WeightedIndex<f64> = WeightedIndex::new(&RES_DISTR).unwrap();

    for x in 0..num_tx {
        // For each transaction, randomly select one account as the sender
        let person_idx = (rng.gen_range(0..num_tx*num_tx) as f64).sqrt() as usize ;

        let person = accounts[person_idx];
        let nonce = sender_map.get(&person).unwrap_or(&0);

        let cost_sample = COST_DISTR[rand::thread_rng().gen_range(0..COST_DISTR.len())];
        // let write_len_sample = LEN_DISTR[rand::thread_rng().gen_range(0..LEN_DISTR.len())] as usize;
        let write_len_sample = rand::thread_rng().gen_range(1..3) as usize;
        let mut writes= Vec::new();
        for _ in 0..write_len_sample {
            writes.push(res_distribution.sample(&mut rng)) ;
        }
        
        // println!("cost_smaple: {:?}", cost_sample);
        // println!("write_len_sample: {:?}", write_len_sample);
        // println!("person: {}", person);
        // println!("writes {:?}", &writes);


        let cost = U256::from(cost_sample.round() as u64);
        costs.push(cost_sample.round() as u64);
        let calldata = Chiron::loop_exchange(cost, &writes);

        let mut write_keys:Vec<AccessListItem> = Vec::new();

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

    for x in 0..num_tx {
        // For each transaction, randomly select one account as the sender
        let person_idx = (rng.gen_range(0..num_tx*num_tx) as f64).sqrt() as usize ;

        let person = accounts[person_idx+num_tx];
        let nonce = sender_map.get(&person).unwrap_or(&0);

        let cost_sample = COST_DISTR[rand::thread_rng().gen_range(0..COST_DISTR.len())];
        // let write_len_sample = LEN_DISTR[rand::thread_rng().gen_range(0..LEN_DISTR.len())] as usize;
        let write_len_sample = rand::thread_rng().gen_range(1..3) as usize;
        let mut writes= Vec::new();
        for _ in 0..write_len_sample {
            writes.push(res_distribution.sample(&mut rng)) ;
        }
        // println!("cost_smaple: {:?}", cost_sample);
        // println!("write_len_sample: {:?}", write_len_sample);
        // println!("person: {}", person);
        // println!("writes {:?}", &writes);

        let cost = U256::from(cost_sample.round() as u64);
        costs2.push(cost_sample.round() as u64);
        let calldata = Chiron::loop_exchange(cost, &writes);

        let mut write_keys:Vec<AccessListItem> = Vec::new();

        txs2.push(TxEnv {
            caller: person,
            gas_limit: GAS_LIMIT * cost_sample as u64,
            gas_price: U256::from(1),
            transact_to: TransactTo::Call(chiron_address2),
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

    (state, bytecodes, txs, costs, txs2, costs2)
}


// =================================
// new workloads
use rand_distr::{Poisson, Zipf};

/// Simplified workload configuration
#[derive(Clone, Debug)]
pub struct BlockGenerationConfig {
    /// Address space size per block
    pub address_space_per_block: usize,  // H
    
    /// Probability of accessing common region
    pub common_access_ratio: f64,  // r_a ∈ [0,1]
    
    /// Zipfian skewness parameter
    pub zipf_theta: f64,  // θ
    
    /// Average number of resources accessed per transaction
    pub avg_accesses_per_tx: f64,  // λ
}

impl Default for BlockGenerationConfig {
    fn default() -> Self {
        Self {
            address_space_per_block: 100,
            common_access_ratio: 0.3,
            zipf_theta: 1.2,
            avg_accesses_per_tx: 2.5,
        }
    }
}

/// Generate n blocks, each containing num_tx transactions
/// All transaction callers are unique
/// All blocks share the same chiron account and state
/// All accesses are writes
/// Uses Poisson and Zipfian distributions to control conflict patterns
pub fn generate_n_blocks_with_controlled_conflicts(
    num_blocks: usize,
    num_tx_per_block: usize,
    config: BlockGenerationConfig,
) -> (HashMap<Address, EvmAccount>, Bytecodes, Vec<Vec<TxEnv>>) {
    
    let total_accounts = num_blocks * num_tx_per_block;
    
    // Generate all required unique accounts
    let all_accounts: Vec<Address> = generate_addresses(total_accounts);
    let chiron_address = Address::new(rand::random());
    
    println!("chiron address: {}", chiron_address);
    println!("total accounts: {}", total_accounts);
    println!("num_blocks: {}, num_tx_per_block: {}", num_blocks, num_tx_per_block);
    println!("config: H={}, r_a={:.2}, θ={:.2}, λ={:.2}", 
             config.address_space_per_block, 
             config.common_access_ratio,
             config.zipf_theta,
             config.avg_accesses_per_tx);
    
    // Initialize shared state with chiron contract
    let chiron_account = Chiron::build();
    let mut state = HashMap::from([(chiron_address, chiron_account)]);
    
    // Initialize all accounts in the shared state
    for account in &all_accounts {
        state.insert(
            *account,
            EvmAccount {
                balance: U256::from(U128::MAX),
                ..EvmAccount::default()
            },
        );
    }
    
    // Create distributions
    let poisson = Poisson::new(config.avg_accesses_per_tx)
        .expect("Invalid Poisson parameter");
    let zipf_common = Zipf::new(config.address_space_per_block as u64, config.zipf_theta)
        .expect("Invalid Zipf parameters for common region");
    
    let mut all_blocks_txs = Vec::new();
    let mut rng = thread_rng();
    
    // Generate transactions for each block
    for block_id in 0..num_blocks {
        let mut txs = Vec::new();
        
        // Calculate account range for this block
        let start_idx = block_id * num_tx_per_block;
        let end_idx = start_idx + num_tx_per_block;
        let block_accounts = &all_accounts[start_idx..end_idx];
        
        // Create Zipfian distribution for this block
        let zipf_block = Zipf::new(config.address_space_per_block as u64, config.zipf_theta)
            .expect("Invalid Zipf parameters for block region");
        
        // println!("Block {}: accounts [{}, {})", block_id, start_idx, end_idx);
        
        // Generate transactions for this block
        for &caller in block_accounts.iter() {
            // 1. Sample number of accesses (using Poisson distribution)
            let num_accesses = sample_num_accesses(&poisson, &mut rng);
            
            // 2. Generate accessed resources (all writes)
            let writes = generate_accesses(
                block_id,
                num_accesses,
                &config,
                &zipf_common,
                &zipf_block,
                &mut rng,
            );

            // 3. Generate calldata
            let cost = U256::from(0);  // cost parameter is unused in this context
            let calldata = Chiron::loop_exchange(cost, &writes);
            
            // 4. Sample gas limit
            let cost_sample = COST_DISTR[rng.gen_range(0..COST_DISTR.len())];
            
            txs.push(TxEnv {
                caller,
                gas_limit: GAS_LIMIT * cost_sample as u64,
                gas_price: U256::from(1),
                transact_to: TransactTo::Call(chiron_address),
                data: calldata,
                nonce: Some(0),
                access_list: Vec::new(),
                ..TxEnv::default()
            });
        }
        
        all_blocks_txs.push(txs);
    }
    
    // Build bytecodes from the shared state
    let mut bytecodes = Bytecodes::default();
    for account in state.values_mut() {
        if let Some(code) = account.code.take() {
            bytecodes.insert(account.code_hash.unwrap(), code);
        }
    }
    
    (state, bytecodes, all_blocks_txs)
}

/// Sample number of accesses (using Poisson distribution)
fn sample_num_accesses(poisson: &Poisson<f64>, rng: &mut impl Rng) -> usize {
    let sample = poisson.sample(rng);
    // Clamp to [1, 10] range
    (sample as usize).clamp(1, 10)
}

/// Generate resources accessed by a transaction
/// All accesses are writes
/// 
/// Address space layout:
/// - Common region: [0, H)
/// - Block i region: [(i+1)×H, (i+2)×H)
fn generate_accesses(
    block_id: usize,
    num_accesses: usize,
    config: &BlockGenerationConfig,
    zipf_common: &Zipf<f64>,
    zipf_block: &Zipf<f64>,
    rng: &mut impl Rng,
) -> Vec<usize> {
    let mut accesses = Vec::new();
    
    for _ in 0..num_accesses {
        let resource_id = if rng.gen::<f64>() < config.common_access_ratio {
            // Access common region [0, H)
            select_from_common_region(config.address_space_per_block, zipf_common, rng)
        } else {
            // Access block's own region [(block_id+1)×H, (block_id+2)×H)
            select_from_block_region(
                block_id,
                config.address_space_per_block,
                zipf_block,
                rng,
            )
        };
        
        accesses.push(resource_id);
    }
    
    accesses
}

/// Select resource from common region
fn select_from_common_region(
    address_space_per_block: usize,
    zipf_common: &Zipf<f64>,
    rng: &mut impl Rng,
) -> usize {
    let offset = (zipf_common.sample(rng) as usize).saturating_sub(1);
    // Return address in [0, H)
    offset.min(address_space_per_block - 1)
}

/// Select resource from block's own region
fn select_from_block_region(
    block_id: usize,
    address_space_per_block: usize,
    zipf_block: &Zipf<f64>,
    rng: &mut impl Rng,
) -> usize {
    let start = (block_id + 1) * address_space_per_block;
    let offset = (zipf_block.sample(rng) as usize).saturating_sub(1);
    // Return address in [(block_id+1)×H, (block_id+2)×H)
    start + offset.min(address_space_per_block - 1)
}