// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

// ScheduleFetcher is responsible for fetching scheduled tasks from the PevmAPI.
// The fetched tasks can then be proposed as a Mysticeti Vertex.

use std::{cmp::min, sync::Arc, time::Duration};
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

    pub fn start(pevm_api: PevmAPI) {
        runtime::Handle::current().spawn(
            Self {
            }
            .run(pevm_api),
        );
    }

    // A thread that continuously fetches scheduled tasks from the PevmAPI.
    pub async fn run(self, mut pevm_api: PevmAPI) {
        tracing::info!("Starting ScheduleFetcher to fetch scheduled task");
        loop {
            match pevm_api.scheduled_transactions() {
                Ok(task) => {
                    tracing::info!("Fetched scheduled task {}", task);
                    // [TODO] Here you would typically process the tasks, e.g., propose them as a Mysticeti Vertex.
                }
                Err(e) => {
                    tracing::error!("Error fetching scheduled task: {}", e);
                }
            }
            // Sleep for 100ms before fetching again.
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}