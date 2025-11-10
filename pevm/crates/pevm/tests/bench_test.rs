
#[path = "../benches/gigagas.rs"]
pub mod gigagas;

use criterion::{Criterion, BenchmarkId, black_box};

#[test]
pub fn test_bench() {
    println!("Running test_bench");
    let mut criterion = Criterion::default().configure_from_args();
    gigagas::bench_solana(&mut criterion);
}