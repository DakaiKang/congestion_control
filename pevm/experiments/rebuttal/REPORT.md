# Rebuttal experiments — generated tables

Machine: c4.8xlarge (18 cores / 36 threads, 58 GiB), t = 8 workers, τ_CV = 0.5, τ_hot = 1.5, InMemoryStorage.


## Real Ethereum (15 000 mainnet blocks)

**Absolute throughput and per-block latency** — 150 batches, 15,000 blocks, 2,387,073 txs

| Engine | total s | tx/s (aggregate) | ms/block p50 | p90 | p99 | speedup |
|---|---:|---:|---:|---:|---:|---:|
| Sequential | 137.7 | 17,335 | 8.82 | 12.45 | 15.11 | 1.00× |
| Block-STM | 73.5 | 32,494 | 4.57 | 7.13 | 10.75 | 1.87× |
| Block-STM, concatenated | 64.5 | 37,027 | 3.73 | 6.29 | 13.82 | 2.14× |
| Graph-aware OCC, concatenated | 70.2 | 33,983 | 4.10 | 6.73 | 14.63 | 1.96× |
| Graph-aware OCC | 78.2 | 30,536 | 4.96 | 7.28 | 10.77 | 1.76× |
| Vegeta (schedule on graph OCC) | 83.4 | 28,611 | 5.29 | 7.70 | 11.43 | 1.65× |
| Omakase | 51.6 | 46,253 | 3.36 | 4.15 | 5.94 | 2.67× |

**Where the time goes — full pipeline including the preparatory phases**

| Phase | total s | share | ms/block | cost as a fraction of sequential time |
|---|---:|---:|---:|---:|
| Pre-execute (proposer, once per block) | 139.6 | 60% | 9.31 | 1.01× |
| Build conflict graph + intra-block reorder | 14.1 | 6% | 0.94 | 0.10× |
| Integrate (greedy, per committed round) | 25.9 | 11% | 1.73 | 0.19× |
| Execute (Omakase, parallel) | 51.6 | 22% | 3.44 | 0.37× |
| **All phases on one node** | **231.2** | 100% | **15.42** | **1.68×** (speedup 0.60×) |
| Validator-side: integrate + execute | 77.5 |  | 5.17 | 0.56× (speedup 1.78×) |
| Sequential execution (reference) | 137.7 |  | 9.18 | 1.00× |

Vegeta's schedule construction (Rule-1 reorder + DAG rebuild) on the same pre-pass: 9.53 s total, 0.635 ms/block.

**Metadata shipped in a proposal**

| Payload | MB total | KB/block | vs calldata |
|---|---:|---:|---:|
| Transaction calldata (what a block already carries) | 1729.2 | 115.3 | 100% |
| Omakase per-block conflict graph (access-set hashes + WAW edges) | 211.3 | 14.1 | 12.2% |
| Vegeta schedule (same encoding) | 211.3 | 14.1 | 12.2% |

**Aborts and re-executions** — 2,387,073 txs (diagnostics build)

| Engine | re-executions / tx | validation aborts / tx | cascade aborts / tx | re-exec writing a new location / tx | cascade share of aborts |
|---|---:|---:|---:|---:|---:|
| Block-STM | 0.547 | 0.182 | 0.0292 | 0.1624 | 16% |
| Block-STM, concatenated | 0.829 | 0.196 | 0.0048 | 0.1955 | 2% |
| Graph OCC, concatenated | 0.650 | 0.360 | 0.0096 | 0.0407 | 3% |
| Graph-aware OCC | 0.346 | 0.154 | 0.0149 | 0.0703 | 10% |
| Vegeta | 0.339 | 0.195 | 0.0284 | 0.0244 | 15% |
| Omakase | 0.419 | 0.205 | 0.0069 | 0.0540 | 3% |

*cascade abort* = validation failure because a lower-indexed writer appeared after the read; *re-exec writing a new location* = the event that invalidates downstream readers, i.e. the trigger of a cascade.

![](plot_real_speedup.png) ![](plot_real_phases.png)


## Synthetic V2 (paper §7.2.2)

**Absolute throughput and per-block latency** — 150 batches, 15,000 blocks, 2,387,073 txs

| Engine | total s | tx/s (aggregate) | ms/block p50 | p90 | p99 | speedup |
|---|---:|---:|---:|---:|---:|---:|
| Sequential | 191.6 | 12,457 | 12.16 | 15.80 | 21.95 | 1.00× |
| Block-STM | 97.9 | 24,371 | 6.04 | 8.11 | 17.01 | 1.96× |
| Block-STM, concatenated | 70.3 | 33,962 | 4.43 | 5.45 | 13.24 | 2.73× |
| Graph-aware OCC, concatenated | 64.1 | 37,217 | 4.04 | 4.94 | 12.62 | 2.99× |
| Graph-aware OCC | 91.3 | 26,143 | 5.63 | 7.67 | 16.30 | 2.10× |
| Vegeta (schedule on graph OCC) | 92.2 | 25,883 | 5.62 | 7.77 | 16.29 | 2.08× |
| Omakase | 81.9 | 29,163 | 5.09 | 6.42 | 14.99 | 2.34× |

**Where the time goes — full pipeline including the preparatory phases**

| Phase | total s | share | ms/block | cost as a fraction of sequential time |
|---|---:|---:|---:|---:|
| Pre-execute (proposer, once per block) | 197.4 | 65% | 13.16 | 1.03× |
| Build conflict graph + intra-block reorder | 11.5 | 4% | 0.77 | 0.06× |
| Integrate (greedy, per committed round) | 12.4 | 4% | 0.83 | 0.06× |
| Execute (Omakase, parallel) | 81.9 | 27% | 5.46 | 0.43× |
| **All phases on one node** | **303.1** | 100% | **20.21** | **1.58×** (speedup 0.63×) |
| Validator-side: integrate + execute | 94.3 |  | 6.28 | 0.49× (speedup 2.03×) |
| Sequential execution (reference) | 191.6 |  | 12.77 | 1.00× |

Vegeta's schedule construction (Rule-1 reorder + DAG rebuild) on the same pre-pass: 6.44 s total, 0.429 ms/block.

**Metadata shipped in a proposal**

| Payload | MB total | KB/block | vs calldata |
|---|---:|---:|---:|
| Transaction calldata (what a block already carries) | 1587.0 | 105.8 | 100% |
| Omakase per-block conflict graph (access-set hashes + WAW edges) | 192.2 | 12.8 | 12.1% |
| Vegeta schedule (same encoding) | 192.1 | 12.8 | 12.1% |

**Aborts and re-executions** — 2,387,073 txs (diagnostics build)

| Engine | re-executions / tx | validation aborts / tx | cascade aborts / tx | re-exec writing a new location / tx | cascade share of aborts |
|---|---:|---:|---:|---:|---:|
| Block-STM | 0.730 | 0.214 | 0.0418 | 0.2033 | 19% |
| Block-STM, concatenated | 1.170 | 0.218 | 0.0045 | 0.2343 | 2% |
| Graph OCC, concatenated | 0.000 | 0.000 | 0.0000 | 0.0000 | 0% |
| Graph-aware OCC | 0.000 | 0.000 | 0.0000 | 0.0000 | 0% |
| Vegeta | 0.000 | 0.000 | 0.0000 | 0.0000 | 0% |
| Omakase | 0.000 | 0.000 | 0.0000 | 0.0000 | 0% |

*cascade abort* = validation failure because a lower-indexed writer appeared after the read; *re-exec writing a new location* = the event that invalidates downstream readers, i.e. the trigger of a cascade.

![](plot_v2_speedup.png) ![](plot_v2_phases.png)


## Artificial workload (tunable conflict)

**Artificial workload — speedup over sequential (t=8, H=5)**

| ρ_inter K | ρ_intra M | Block-STM | Concat | Concat+graph | Graph OCC | Vegeta | Omakase |
|---|---:|---:|---:|---:|---:|---:|---:|
| 2 | 10% | 5.08 | 5.89 | 6.22 | 5.27 | 5.24 | 5.28 |
| 4 | 10% | 5.05 | 5.88 | 6.22 | 5.25 | 5.19 | 5.25 |
| 6 | 10% | 5.06 | 5.89 | 6.23 | 5.25 | 5.20 | 5.25 |
| 8 | 10% | 4.95 | 5.74 | 6.21 | 5.24 | 5.20 | 5.25 |
| 2 | 20% | 3.67 | 5.20 | 5.96 | 3.69 | 3.78 | 3.70 |
| 4 | 20% | 3.65 | 5.05 | 5.94 | 3.69 | 3.77 | 3.69 |
| 6 | 20% | 3.66 | 4.94 | 5.96 | 3.68 | 3.76 | 3.69 |
| 8 | 20% | 3.67 | 4.80 | 5.68 | 3.69 | 3.77 | 3.69 |
| 2 | 30% | 2.67 | 4.47 | 5.75 | 2.67 | 2.70 | 4.31 |
| 4 | 30% | 2.67 | 4.19 | 4.94 | 2.66 | 2.70 | 4.11 |
| 6 | 30% | 2.64 | 3.91 | 4.32 | 2.66 | 2.70 | 3.80 |
| 8 | 30% | 2.67 | 3.74 | 3.98 | 2.67 | 2.71 | 3.54 |
| 2 | 40% | 2.06 | 3.79 | 4.76 | 2.09 | 2.10 | 3.50 |
| 4 | 40% | 2.08 | 3.40 | 3.67 | 2.08 | 2.09 | 3.24 |
| 6 | 40% | 2.09 | 3.09 | 3.22 | 2.07 | 2.08 | 2.99 |
| 8 | 40% | 2.08 | 2.88 | 3.00 | 2.09 | 2.10 | 2.80 |

**Artificial workload — re-executions per transaction**

| K | M | Block-STM | Concat | Concat+graph | Graph OCC | Vegeta | Omakase |
|---|---:|---:|---:|---:|---:|---:|---:|
| 2 | 10% | 0.100 | 0.101 | 0.000 | 0.000 | 0.000 | 0.000 |
| 4 | 10% | 0.101 | 0.106 | 0.000 | 0.000 | 0.000 | 0.000 |
| 6 | 10% | 0.100 | 0.108 | 0.000 | 0.000 | 0.000 | 0.000 |
| 8 | 10% | 0.101 | 0.110 | 0.000 | 0.000 | 0.000 | 0.000 |
| 2 | 20% | 0.452 | 0.471 | 0.000 | 0.000 | 0.000 | 0.000 |
| 4 | 20% | 0.453 | 0.596 | 0.000 | 0.000 | 0.000 | 0.000 |
| 6 | 20% | 0.453 | 0.714 | 0.000 | 0.000 | 0.000 | 0.000 |
| 8 | 20% | 0.453 | 0.797 | 0.000 | 0.000 | 0.000 | 0.000 |
| 2 | 30% | 0.943 | 1.057 | 0.000 | 0.000 | 0.000 | 0.000 |
| 4 | 30% | 0.943 | 1.318 | 0.000 | 0.000 | 0.000 | 0.000 |
| 6 | 30% | 0.947 | 1.559 | 0.000 | 0.000 | 0.000 | 0.000 |
| 8 | 30% | 0.943 | 1.717 | 0.000 | 0.000 | 0.000 | 0.000 |
| 2 | 40% | 1.529 | 1.832 | 0.000 | 0.000 | 0.000 | 0.000 |
| 4 | 40% | 1.531 | 2.243 | 0.000 | 0.000 | 0.000 | 0.000 |
| 6 | 40% | 1.536 | 2.631 | 0.000 | 0.000 | 0.000 | 0.000 |
| 8 | 40% | 1.530 | 2.830 | 0.000 | 0.000 | 0.000 | 0.000 |

![](plot_artificial_grid.png)


## Thread-count sweep — artificial

| worker threads | Block-STM | Concat | Concat+graph | Graph OCC | Vegeta | Omakase | groups/batch | re-exec/tx: Concat | Omakase |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 4 | 2.02 | 2.33 | 3.25 | 2.09 | 2.12 | 2.10 | 50.0 | 1.572 | 0.000 |
| 8 | 2.08 | 3.42 | 3.73 | 2.09 | 2.11 | 3.24 | 25.1 | 2.245 | 0.000 |
| 16 | 1.28 | 2.52 | 3.29 | 1.90 | 1.95 | 3.11 | 14.3 | 3.486 | 0.000 |
| 24 | 1.21 | 2.38 | 3.27 | 1.74 | 1.83 | 2.91 | 14.5 | 3.553 | 0.000 |
| 32 | 1.23 | 2.39 | 3.27 | 1.58 | 1.80 | 2.77 | 14.5 | 3.413 | 0.000 |


## Thread-count sweep — real

| worker threads | Block-STM | Concat | Concat+graph | Graph OCC | Vegeta | Omakase | groups/batch | re-exec/tx: Concat | Omakase |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 4 | 1.78 | 2.12 | 2.01 | 1.79 | 1.66 | 2.44 | 41.4 | 0.322 | 0.269 |
| 8 | 1.96 | 2.37 | 2.14 | 1.84 | 1.75 | 2.68 | 21.1 | 0.864 | 0.420 |
| 16 | 1.32 | 1.85 | 1.93 | 1.71 | 1.66 | 2.39 | 35.6 | 1.261 | 0.401 |
| 24 | 1.24 | 1.80 | 1.98 | 1.65 | 1.61 | 2.23 | 58.5 | 1.426 | 0.323 |
| 32 | 1.22 | 1.59 | 1.97 | 1.56 | 1.53 | 2.07 | 73.5 | 1.507 | 0.262 |


## Round-size sweep — artificial

| blocks per round | Block-STM | Concat | Concat+graph | Graph OCC | Vegeta | Omakase | groups/batch | re-exec/tx: Concat | Omakase |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 5 | 2.08 | 2.51 | 2.59 | 2.10 | 2.13 | 2.50 | 3.8 | 2.542 | 0.000 |
| 10 | 2.11 | 3.17 | 3.38 | 2.09 | 2.12 | 3.00 | 5.9 | 2.250 | 0.000 |
| 20 | 2.08 | 3.14 | 3.37 | 2.09 | 2.12 | 3.01 | 11.5 | 2.332 | 0.000 |
| 50 | 2.08 | 3.42 | 3.73 | 2.09 | 2.10 | 3.23 | 25.1 | 2.252 | 0.000 |
| 100 | 2.05 | 3.34 | 3.68 | 2.08 | 2.09 | 3.29 | 48.8 | 2.280 | 0.000 |


## Round-size sweep — real

| blocks per round | Block-STM | Concat | Concat+graph | Graph OCC | Vegeta | Omakase | groups/batch | re-exec/tx: Concat | Omakase |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 5 | 1.89 | 2.36 | 2.14 | 1.81 | 1.71 | 2.18 | 2.4 | 0.916 | 0.418 |
| 10 | 1.89 | 2.47 | 2.23 | 1.80 | 1.71 | 2.28 | 3.7 | 0.963 | 0.420 |
| 20 | 1.91 | 2.54 | 2.26 | 1.81 | 1.72 | 2.38 | 5.4 | 0.962 | 0.426 |
| 50 | 1.94 | 2.43 | 2.12 | 1.83 | 1.73 | 2.59 | 10.6 | 0.916 | 0.426 |
| 100 | 1.95 | 2.37 | 2.13 | 1.83 | 1.74 | 2.68 | 21.1 | 0.867 | 0.419 |


## Merge-cap sweep (Omakase only varies) — artificial

| merge cap (blocks/group) | Block-STM | Concat | Concat+graph | Graph OCC | Vegeta | Omakase | groups/batch | re-exec/tx: Concat | Omakase |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 10 | 2.08 | 3.43 | 3.73 | 2.09 | 2.11 | 3.24 | 25.1 | 2.247 | 0.000 |
| 25 | 2.06 | 3.40 | 3.73 | 2.09 | 2.10 | 3.24 | 25.1 | 2.257 | 0.000 |
| 50 | 2.07 | 3.41 | 3.73 | 2.09 | 2.10 | 3.24 | 25.1 | 2.255 | 0.000 |
| 1000 | 2.08 | 3.41 | 3.74 | 2.09 | 2.10 | 3.24 | 25.1 | 2.267 | 0.000 |


## Merge-cap sweep (Omakase only varies) — real

| merge cap (blocks/group) | Block-STM | Concat | Concat+graph | Graph OCC | Vegeta | Omakase | groups/batch | re-exec/tx: Concat | Omakase |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 10 | 1.96 | 2.37 | 2.14 | 1.85 | 1.75 | 2.69 | 21.1 | 0.859 | 0.419 |
| 25 | 1.95 | 2.35 | 2.14 | 1.84 | 1.74 | 2.67 | 20.9 | 0.869 | 0.417 |
| 50 | 1.95 | 2.36 | 2.14 | 1.84 | 1.75 | 2.67 | 20.9 | 0.863 | 0.413 |
| 1000 | 1.96 | 2.36 | 2.04 | 1.84 | 1.75 | 2.68 | 20.9 | 0.866 | 0.414 |


## Aggressive integration (merge unless hot-key conflict) — artificial

| merge cap, tau_cv=0.01 | Block-STM | Concat | Concat+graph | Graph OCC | Vegeta | Omakase | groups/batch | re-exec/tx: Concat | Omakase |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 10 | 2.09 | 3.44 | 3.73 | 2.09 | 2.11 | 3.45 | 13.3 | 2.263 | 0.000 |
| 50 | 2.07 | 3.40 | 3.73 | 2.08 | 2.10 | 3.49 | 4.8 | 2.265 | 0.000 |
| 1000 | 2.05 | 3.40 | 3.75 | 2.08 | 2.10 | 3.49 | 4.8 | 2.267 | 0.000 |


## Aggressive integration (merge unless hot-key conflict) — real

| merge cap, tau_cv=0.01 | Block-STM | Concat | Concat+graph | Graph OCC | Vegeta | Omakase | groups/batch | re-exec/tx: Concat | Omakase |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 10 | 1.95 | 2.36 | 2.14 | 1.84 | 1.75 | 2.73 | 13.2 | 0.865 | 0.413 |
| 50 | 1.94 | 2.35 | 2.12 | 1.83 | 1.74 | 2.36 | 6.7 | 0.868 | 0.367 |
| 1000 | 1.96 | 2.37 | 2.15 | 1.84 | 1.75 | 1.68 | 4.8 | 0.865 | 0.250 |


## Per-transaction cost sweep — artificial

| simulated work per tx (TARGET) | Block-STM | Concat | Concat+graph | Graph OCC | Vegeta | Omakase | groups/batch | re-exec/tx: Concat | Omakase |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 100 | 2.08 | 3.41 | 3.74 | 2.09 | 2.10 | 3.26 | 25.0 | 2.255 | 0.000 |
| 500 | 2.35 | 4.08 | 4.27 | 2.38 | 2.39 | 3.82 | 25.2 | 3.462 | 0.000 |
| 2000 | 2.41 | 4.25 | 4.40 | 2.46 | 2.45 | 3.92 | 25.3 | 4.160 | 0.000 |


## Pipelined integration — real

| blocks/round | rounds | txs | seq s | Block-STM s | Omakase serial s | integration s | Omakase pipelined s | integration hidden |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 100 | 10 | 170746 | 9.87 | 4.96 (1.99×) | 6.24 (1.58×) | 2.04 | 4.45 (2.22×) | 88% |
| 20 | 20 | 70070 | 3.94 | 2.05 (1.93×) | 2.88 (1.37×) | 0.82 | 2.14 (1.84×) | 90% |
