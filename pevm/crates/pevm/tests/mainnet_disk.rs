/// Quick performance test for Ethereum Mainnet blocks
use pevm::{chain::PevmEthereum, Pevm};
use std::thread;
use std::num::NonZeroUsize;
pub mod common;

#[test]
/// Performance test for specific Ethereum Mainnet block
pub fn test_mainnet_block_performance() {
    use std::time::Instant;
    
    let chain = PevmEthereum::mainnet();
    let concurrency_level = thread::available_parallelism()
        .unwrap_or(NonZeroUsize::MIN)
        .min(NonZeroUsize::new(8).unwrap());
    
    let mut pevm = Pevm::default();
    let target_block = 5283152;  // ← 指定区块号

    println!("\n=== Mainnet Block Performance ===");
    println!("Concurrency: {}\n", concurrency_level);

    common::for_each_block_from_disk(|block, storage| {
        // Only run for block 5283152
        if block.header.number != target_block {
            return;  // ← 跳过其他区块
        }
        
        println!("Block {} ({} txs, {} gas)", 
                 block.header.number, 
                 block.transactions.len(), 
                 block.header.gas_used);
        
        // Sequential
        let start = Instant::now();
        pevm.execute(&chain, &storage, &block, concurrency_level, true).unwrap();
        let seq_time = start.elapsed();
        
        // Parallel
        let start = Instant::now();
        pevm.execute(&chain, &storage, &block, concurrency_level, false).unwrap();
        let par_time = start.elapsed();
        
        let tx_count = block.transactions.len() as f64;
        println!("  Sequential: {:.2}s ({:.2} txs/s)", 
                 seq_time.as_secs_f64(), 
                 tx_count / seq_time.as_secs_f64());
        println!("  Parallel:   {:.2}s ({:.2} txs/s)", 
                 par_time.as_secs_f64(), 
                 tx_count / par_time.as_secs_f64());
        println!("  Speedup:    {:.2}x\n", 
                 seq_time.as_secs_f64() / par_time.as_secs_f64());
    });
}