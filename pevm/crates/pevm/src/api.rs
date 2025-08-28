// Provide Pevm API for transaction scheduling and execution, used by Mysticeti.
use std::fmt;
use std::error::Error;
// #[cfg(feature = "with-tokio")]
use tokio::time::{sleep, Duration};
// #[cfg(feature = "with-tokio")]
use tokio::sync::Mutex;

use std::{
    collections::{VecDeque},
};
use crate::{
    Pevm,
};

use ethers::types::{
    Address, 
};

use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

use super::serialization::deserializer;

#[derive(Debug, Clone, Default)]
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
}

impl PevmAPI {
    pub fn new() -> Self {
        Self {
            pevm: Pevm::default(),
            txns_queue: Mutex::new(VecDeque::new()),
            scheduled_txns: Mutex::new(VecDeque::new()),
        }
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

        tracing::info!("Scheduled transactions: {:?}", scheduled_queue);

        scheduled_queue
            .pop_front()
            .ok_or(APIError::NoScheduledTransactions)
    }

}


