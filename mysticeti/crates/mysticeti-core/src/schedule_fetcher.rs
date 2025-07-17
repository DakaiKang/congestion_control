// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

// ScheduleFetcher is responsible for fetching scheduled tasks from the PevmAPI.
// The fetched tasks can then be proposed as a Mysticeti Vertex.

use std::{cmp::min, sync::{Arc}};
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

use crate::{
    runtime::{self, timestamp_utc},
};
use pevm::api::{PevmAPI, APIError};

pub struct ScheduleFetcher {
    // This struct can hold any necessary state for fetching scheduled tasks.
}

impl ScheduleFetcher {
    pub fn new() -> Self {
        Self {}
    }

    pub fn start(pevm_api: Arc<Mutex<PevmAPI>>) {
        tracing::info!("Starting ScheduleFetcher");
        tokio::spawn(async move {
            Self{}.run(pevm_api).await;
        });
    }

    pub async fn run(self, pevm_api: Arc<Mutex<PevmAPI>>) {
        loop {
            sleep(Duration::from_millis(100)).await;
            let mut guard = pevm_api.lock().await;
            match guard.scheduled_transactions() {
                Ok(task) => {
                    tracing::info!("Fetched scheduled task {}", task);
                    // [TODO] propose task to Mysticeti Vertex
                }
                Err(e) => {
                    tracing::error!("Error fetching scheduled task: {}", e);
                }
            }
            drop(guard);
        }
    }
}