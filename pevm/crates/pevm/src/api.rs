// Provide Pevm API for transaction scheduling and execution, used by Mysticeti.
use std::{fmt, fs};
use std::error::Error;
// #[cfg(feature = "with-tokio")]
use tokio::time::{sleep, Duration};
// #[cfg(feature = "with-tokio")]
use tokio::sync::Mutex;
use tokio::sync::mpsc;

use std::{
    collections::{VecDeque},
    num::NonZeroUsize,
    thread,
};
use crate::{
    Pevm,
    erc20::contract::ERC20Token,
};

use revm::primitives::{alloy_primitives::U160, BlockEnv, SpecId, TxEnv, U256, TransactTo};

use std::sync::Arc;

use ethers::types::{
    Address, 
};

use alloy_primitives::Address as AlloyAddress;

use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter};
use std::path::Path;

use serde::{Serialize, Deserialize};

use super::serialization::{deserializer, serializer};
use crate::{Bytecodes, ChainState, EvmAccount, InMemoryStorage, chain::PevmEthereum};
use super::erc20;

use serde_json;

fn save(storage: &InMemoryStorage, path: &str) -> anyhow::Result<()> {
    let file = File::create(path)?;
    let writer = BufWriter::new(file);
    serde_json::to_writer(writer, storage)?;
    Ok(())
}

fn load(path: &str) -> anyhow::Result<InMemoryStorage> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let storage = serde_json::from_reader(reader)?;
    Ok(storage)
}

pub fn load_in_memory_storage(workload_type: &WorkloadType) -> InMemoryStorage {
    match workload_type {
        WorkloadType::ERC20(num_clusters, num_families_per_cluster, num_people_per_family) => {
            let path = format!("/home/ubuntu/congestion_control/pevm/crates/pevm/storage_{}_{}_{}.json", num_clusters, num_families_per_cluster, num_people_per_family);
            println!("in memory storage file path: {}", path);
            load(&path).unwrap()
        }
    }
}

pub fn load_account_addresses(workload_type: &WorkloadType) -> Vec<(AlloyAddress, Vec<Vec<AlloyAddress>>)> {
    match workload_type {
        WorkloadType::ERC20(num_clusters, num_families_per_cluster, num_people_per_family) => {
            let path = format!("/home/ubuntu/congestion_control/pevm/crates/pevm/account_addresses_{}_{}_{}.bin", num_clusters, num_families_per_cluster, num_people_per_family);
            println!("account addresses file path: {}", path);
            load_addresses(&path).unwrap()
        }
    }
}

type Addresses = Vec<(AlloyAddress, Vec<Vec<AlloyAddress>>)>;

fn save_addresses(path: &str, data: &Addresses) -> anyhow::Result<()> {
    let encoded = bincode::serialize(data)?;        // binary encoding
    fs::write(path, encoded)?;
    Ok(())
}

fn load_addresses(path: &str) -> anyhow::Result<Addresses> {
    let bytes = fs::read(path)?;
    let decoded: Addresses = bincode::deserialize(&bytes)?;
    Ok(decoded)
}


#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TransactionWithHint {
    pub raw_hex: String,
    pub caller: Address,
    pub hint: String, // [TODO] The type 'String' is a placeholder for now
}

/// A list of Error types that can be returned by the Pevm API.
#[derive(Debug, Clone)]
pub enum APIError {
    NoScheduledTransactions,
    NoWorkloadFile,
}

impl fmt::Display for APIError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            APIError::NoScheduledTransactions => write!(f, "No scheduled transactions available"),
            APIError::NoWorkloadFile => write!(f, "No workload file available"),
        }
    }
}

impl Error for APIError {}

#[derive(Debug)]
pub struct PevmAPI {
    pub pevm: Pevm,
    pub txns_queue: Mutex<VecDeque<TransactionWithHint>>,
    pub scheduled_txns: Mutex<VecDeque<TransactionWithHint>>,
    pub addresses: Vec<(AlloyAddress, Vec<Vec<AlloyAddress>>)>, 
    pub workload_type: WorkloadType,
    pub in_memory_storage: InMemoryStorage,
}

impl PevmAPI {
    pub fn new(workload_type: WorkloadType) -> Self {
        let addresses = load_account_addresses(&workload_type);
        let in_memory_storage = load_in_memory_storage(&workload_type);
        Self {
            pevm: Pevm::default(),
            txns_queue: Mutex::new(VecDeque::new()),
            scheduled_txns: Mutex::new(VecDeque::new()),
            workload_type,
            addresses,
            in_memory_storage,
        }
    }

    pub async fn num_pending_txns(&self) -> usize {
        let queue = self.txns_queue.lock().await;
        let scheduled = self.scheduled_txns.lock().await;
        queue.len() + scheduled.len()
    }

    pub async fn add_transactions(&mut self, transactions: Vec<TransactionWithHint>) {
        tracing::info!("Waiting queue lock");
        let mut queue = self.txns_queue.lock().await;
        for txn in transactions {
            tracing::info!("Adding transaction: {:?}", txn);
            queue.push_back(txn);
        }
    }
    

    pub async fn fetch_one_scheduled_txn(&mut self) -> Result<TransactionWithHint, APIError> {
        let mut scheduled_queue = self.scheduled_txns.lock().await;
        if scheduled_queue.is_empty() {
            tracing::info!("No scheduled transactions");
            return Err(APIError::NoScheduledTransactions);
        }

        scheduled_queue
            .pop_front()
            .ok_or(APIError::NoScheduledTransactions)
    }

    pub fn get_erc20_state_and_bytecode(num_clusters: usize, num_families_per_cluster: usize, num_people_per_family: usize) -> (InMemoryStorage, Vec<(AlloyAddress, Vec<Vec<AlloyAddress>>)>) {
        let mut addresses = Vec::new();
        let mut final_state = ChainState::default();
        let mut final_bytecodes = Bytecodes::default();
        final_state.insert(AlloyAddress::ZERO, EvmAccount::default()); // Beneficiary
        for _ in 0..num_clusters {
            let (state, bytecodes, gld_address, families) = erc20::generate_state_and_byte_code(num_families_per_cluster, num_people_per_family);
            final_state.extend(state);
            final_bytecodes.extend(bytecodes);
            addresses.push((gld_address, families));
        }
        let in_memory_storage = InMemoryStorage::new(final_state, Arc::new(final_bytecodes), Default::default());
        (in_memory_storage, addresses)
    }

}


#[derive(Debug, Clone, Default)]
pub enum ExecutionMode {
    #[default] Sequential,
    Parallel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkloadType {
    ERC20(usize, usize, usize), // NUM_CLUSTERS, NUM_FAMILY_PER_CLUSTER, NUM_PEOPLE_PER_FAMILY
}

impl Default for WorkloadType {
    fn default() -> Self {
        WorkloadType::ERC20(1, 1, 1) // or your preferred defaults
    }
}

pub struct PevmExecutor {
    pub execution_mode: ExecutionMode,
    pub storage: InMemoryStorage,
    pub chain: PevmEthereum,
}

impl PevmExecutor {
    pub fn new(execution_mode: ExecutionMode, workload_type: WorkloadType) -> Self {
        let storage = load_in_memory_storage(&workload_type);
        Self {
            execution_mode,
            storage,
            chain: PevmEthereum::mainnet(),
        }
    }

    fn load_from_json(path: &str) -> InMemoryStorage {
        load(path).expect("Failed to load InMemoryStorage from JSON")
    }

    pub fn execute(&self, txs: Vec<(String, Address)>) {
        let txs = deserializer::decode_batch_hex(txs);
        match self.execution_mode {
            ExecutionMode::Sequential => {
                tracing::info!("Executed transactions sequentially");
                crate::execute_revm_sequential(
                    &self.chain,
                    &self.storage,
                    SpecId::LATEST,
                    BlockEnv::default(),
                    txs.clone(),
                );
            }
            ExecutionMode::Parallel => {
                let concurrency_level = thread::available_parallelism().unwrap_or(NonZeroUsize::MIN);
                tracing::info!("Executed transactions in parallel with {} threads", concurrency_level);
                Pevm::default().execute_revm_parallel(
                    &self.chain,
                    &self.storage,
                    SpecId::LATEST,
                    BlockEnv::default(),
                    txs,
                    concurrency_level,
                );
            }
        }
    }
}

pub struct PevmTransactionGenerator {
    pub workload_type: WorkloadType,
    pub clusters: Vec::<(AlloyAddress, Vec<Vec<AlloyAddress>>)>,
    pub nonce: u64,
    pub replica_id: u64,
    pub replica_num: u64,
    pub pevm_txn_sender: mpsc::Sender<Vec<(String, Address)>>,
    pub insufficient_txn_signal_receiver: mpsc::Receiver<usize>,
}

impl PevmTransactionGenerator {
    pub fn new(workload_type: WorkloadType, replica_id: u64, replica_num: u64, pevm_txn_sender: mpsc::Sender<Vec<(String, Address)>>, insufficient_txn_signal_receiver: mpsc::Receiver<usize>) -> Self {
        let clusters = load_account_addresses(&workload_type);
        Self {
            workload_type,
            clusters,
            nonce: 0,
            replica_id,
            replica_num,
            pevm_txn_sender,
            insufficient_txn_signal_receiver,
        }
    }

    pub async fn run(&mut self) {
        const MAX_PENDING_TRANSACTION_NUM:usize = 1000;
        let mut new_transactions = Vec::new();
        tracing::info!("Start Running PEVM");
        loop {
            let batch = self.generate_transactions();
            new_transactions.extend(batch);
            if new_transactions.len() >= MAX_PENDING_TRANSACTION_NUM + 500 {
                let initial_batch_to_schedule = new_transactions.drain(..500).collect();
                tracing::info!("Sending 500");
                self.pevm_txn_sender.send(initial_batch_to_schedule).await;
                tracing::info!("Sent 500");
                break;
            }
        }

        loop{
            let txn_needed = self.insufficient_txn_signal_receiver.recv().await.unwrap();
            tracing::info!("txn_needed = {}", txn_needed);
            let batch_to_schedule: Vec<(String, Address)> = new_transactions.drain(..txn_needed).collect();
            self.pevm_txn_sender.send(batch_to_schedule).await;
            loop{
                let batch = self.generate_transactions();
                new_transactions.extend(batch);
                if new_transactions.len() >= MAX_PENDING_TRANSACTION_NUM {
                    break;
                }
            }
        }
    }

    pub fn generate_transactions(&mut self) -> Vec<(String, Address)> {
        match self.workload_type {
            WorkloadType::ERC20(_, _, _) => {
                self.generate_erc20_transactions()
            },
        }
    }

    pub fn generate_erc20_transactions(&mut self) -> Vec<(String, Address)> {
        const GAS_LIMIT: u64 = 35_000;
        let mut transactions = Vec::new();
        let mut rng = rand::thread_rng();

        for (gld_address, families) in &self.clusters {
            for family in families {
                let mut counter = 0;
                for member in family {
                    if counter % self.replica_num != self.replica_id {
                        counter += 1;
                        continue; // Each member sends transaction once every 4 iterations
                    }
                    let recipient = family[(rand::random::<usize>()) % (family.len())];
                    let calldata = ERC20Token::transfer(recipient, U256::from(rand::random::<u8>()));
                    transactions.push(TxEnv {
                        caller: *member,
                        gas_limit: GAS_LIMIT,
                        gas_price: U256::from(0xb2d05e07u64),
                        transact_to: TransactTo::Call(*gld_address),
                        data: calldata,
                        nonce: Some(self.nonce as u64),
                        chain_id: Some(1),
                        ..TxEnv::default()
                    });
                    counter += 1;
                }
            }
        }

        let hex_codes = serializer::encode_batch_to_hex(transactions);
        
        self.nonce += 1;

        hex_codes
    }
}


pub struct PevmScheduler {
    scheduled_txns: Arc<Mutex<Vec<TransactionWithHint>>>,
    pevm_txn_receiver: Mutex<mpsc::Receiver<Vec<(String, Address)>>>,
}

impl PevmScheduler {
    pub fn new(
        pevm_txn_receiver: mpsc::Receiver<Vec<(String, Address)>>,
    ) -> Self {
        Self {
            scheduled_txns: Arc::new(Mutex::new(Vec::new())),
            pevm_txn_receiver: Mutex::new(pevm_txn_receiver),
        }
    }

    pub async fn run(self: Arc<Self>) {
        tracing::info!("starting running PevmScheduler");
        // Take the receiver exactly once
        let mut rx = self.pevm_txn_receiver
            .lock().await;

        tracing::info!("Got rx");
        while let Some(batch) = rx.recv().await {
            tracing::info!("scheduling {} txns", batch.len());
            self.schedule(batch).await;
        }
        tracing::info!("receiver closed, exiting scheduler");
    }

    pub async fn schedule(&self, batch: Vec<(String, Address)>) {
        let batch: Vec<TransactionWithHint> = batch
            .into_iter()
            .map(|(raw_hex, caller)| TransactionWithHint {
                raw_hex,
                caller,
                hint: String::new(), // or some default value
            }).collect();
        let mut lock = self.scheduled_txns.lock().await;
        lock.extend(batch);
    }

    pub async fn fetch_batch(&self, n: usize) -> Vec<TransactionWithHint> {
        let mut lock = self.scheduled_txns.lock().await;
        let len = lock.len();
        let batch: Vec<TransactionWithHint> = lock.drain(..n.min(len)).collect();
        batch
    }
}

#[test]
pub fn store_in_memory_storage() {
    let (in_memory_storage, _account_addresses) = PevmAPI::get_erc20_state_and_bytecode(1, 2, 3);
    // Save
    save(&in_memory_storage, "storage.json");

    // Load
    let restored = load("storage.json");
    println!("Restored: {:?}", restored);
}

#[test]

pub fn store_account_address() {
    let (in_memory_storage, account_addresses) = PevmAPI::get_erc20_state_and_bytecode(1, 2, 3);
    // Save
    save_addresses("account_addresses.bin", &account_addresses);
    println!("Saved account addresses: {:?}", account_addresses);

    // Load
    let restored = load_addresses("account_addresses.bin");
    println!("Restored: {:?}", restored);
    
}

#[test]
pub fn store_and_load_both() {
    let (in_memory_storage, account_addresses) = PevmAPI::get_erc20_state_and_bytecode(1, 4, 4);
    // Save
    save_addresses("account_addresses.bin", &account_addresses);
    println!("Saved account addresses: {:?}", account_addresses);

    save(&in_memory_storage, "storage.json");
    // println!("Saved in-memory storage: {:?}", in_memory_storage);

     // Load
    let restored_addresses = load_addresses("account_addresses.bin");
    println!("Restored: {:?}", restored_addresses);
    let restored = load("storage.json");
    // println!("Restored: {:?}", restored);

    assert_eq!(restored.unwrap(), in_memory_storage);
    assert_eq!(restored_addresses.unwrap(), account_addresses);
}


#[test]

pub fn test_load_both() {
    let workload_type = WorkloadType::ERC20(1, 2, 3);

    let restored_addresses = load_addresses("account_addresses.bin");
    println!("Restored: {:?}", restored_addresses);
    let restored_storage = load("storage.json").unwrap();
    // println!("Restored: {:?}", restored);

    let mut generator = PevmTransactionGenerator::new(workload_type, restored_addresses.unwrap());

    let transactions = generator.generate_transactions();

    let tx_envs = deserializer::decode_batch_hex(transactions);

    let chain = PevmEthereum::mainnet();

    println!("tx_envs: {:?}", tx_envs);

    let concurrency_level = thread::available_parallelism().unwrap_or(NonZeroUsize::MIN);
        tracing::info!("Executed transactions in parallel with {} threads", concurrency_level);
        let results = Pevm::default().execute_revm_parallel(
            &chain,
            &restored_storage,
            SpecId::LATEST,
            BlockEnv::default(),
            tx_envs,
            concurrency_level,
        );

    // println!("Results: {:?}", results);

}

#[test]
pub fn test_load_in_memory_storage(){
    let workload_type = WorkloadType::ERC20(1, 2, 3);
    let storage = load_in_memory_storage(&workload_type);
    let addresses = load_account_addresses(&workload_type);
    println!("{:?}", addresses);
}