// Provide Pevm API for transaction scheduling and execution, used by Mysticeti.


use std::fmt;
use std::error::Error;
#[cfg(feature = "with-tokio")]
use tokio::time::{sleep, Duration};

use std::{
    collections::{VecDeque},
};
use crate::{
    Pevm,
};



/// A list of Error types that can be returned by the Pevm API.
#[derive(Debug)]
pub enum APIError {
    NoScheduledTransactions,
}

impl fmt::Display for APIError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            APIError::NoScheduledTransactions => write!(f, "No scheduled transactions available"),
        }
    }
}

impl Error for APIError {}

#[derive(Debug)]
pub struct PevmAPI {
    pevm: Pevm,
    txns_queue: VecDeque<i32>,
    scheduled_txns: VecDeque<i32>,
}

impl PevmAPI {
    pub fn new() -> Self {
        Self {
            pevm: Pevm::default(),
            txns_queue: VecDeque::new(),
            scheduled_txns: VecDeque::new(),
        }
    }
    pub fn add_transactions(&mut self, transactions: Vec<i32>) {
        for txn in &transactions {
            tracing::info!("Adding transaction: {}", txn);
            self.txns_queue.push_back(*txn);
        }
    }

    pub async fn schedule(&mut self) {
        loop {
            let Some(transaction) = self.txns_queue.pop_front() else {
                tracing::info!("No more transactions to schedule");
                #[cfg(feature = "with-tokio")]
                sleep(Duration::from_millis(1000)).await;
                continue;
            };
            tracing::info!("Scheduling transaction: {:?}", transaction);
            // Here we would schedule the transaction for execution
            // For now, we just log it
            self.scheduled_txns.push_back(transaction);
        }
    }

    pub fn scheduled_transactions(&mut self) -> Result<i32, APIError> {
        if self.scheduled_txns.is_empty() {
            tracing::info!("No scheduled transactions");
            return Err(APIError::NoScheduledTransactions);
        }

        tracing::info!("Scheduled transactions: {:?}", self.scheduled_txns);

        self.scheduled_txns
            .pop_front()
            .ok_or(APIError::NoScheduledTransactions)
    }
}

