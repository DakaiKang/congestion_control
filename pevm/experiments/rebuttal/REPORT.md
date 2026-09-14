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
| Vegeta (speculate + batch replay + serial tail) | 104.9 | 22,766 | 6.84 | 9.49 | 12.32 | 1.31× |
| Omakase | 51.6 | 46,253 | 3.36 | 4.15 | 5.94 | 2.67× |

**Where the time goes — full pipeline including the preparatory phases**

| Phase | total s | share | ms/block | cost as a fraction of sequential time |
|---|---:|---:|---:|---:|
| Pre-execute (proposer, once per block) | 139.6 | 60% | 9.31 | 1.01× |
| Build conflict graph + intra-block reorder | 14.1 | 6% | 0.94 | 0.10× |
| Integrate (greedy, per committed round) | 25.9 | 11% | 1.73 | 0.19× |
| Execute (Omakase, parallel) | 51.6 | 22% | 3.44 | 0.37× |
| All phases on one node (single proposer, P = 1) | 231.2 | 100% | 15.42 | 1.68× (speedup 0.60×) |
| Validator-side: integrate + execute | 77.5 |  | 5.17 | 0.56× (speedup 1.78×) |
| **Every stage on one node, 1/P of proposer stages, P = 20** | **85.2** |  | **5.68** | **0.62×** (speedup 1.62×) |
| **Every stage on one node, 1/P of proposer stages, P = 50** | **80.6** |  | **5.37** | **0.59×** (speedup 1.71×) |
| **Every stage on one node, 1/P of proposer stages, P = 100** | **79.1** |  | **5.27** | **0.57×** (speedup 1.74×) |
| Sequential execution (reference) | 137.7 |  | 9.18 | 1.00× |

Vegeta's schedule construction (Rule-1 reorder + DAG rebuild) on the same pre-pass: 24.02 s total, 1.601 ms/block.

**Every stage on one node with P proposers** — round size = P (one block per proposer per round); ms/block

| P | blocks | seq. | proposer stages (pre-exec + graph) | integrate | execute | validator + proposer/P | exec only | integ + exec | every stage on one node, 1/P | Block-STM |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 20 | 15,000 | 9.37 | 10.39 | 1.63 | 4.01 | 6.16 | 2.33× | 1.66× | **1.52×** | 1.84× |
| 50 | 15,000 | 9.33 | 10.40 | 1.72 | 3.74 | 5.66 | 2.50× | 1.71× | **1.65×** | 1.86× |
| 100 | 15,000 | 9.18 | 10.25 | 1.73 | 3.44 | 5.27 | 2.67× | 1.78× | **1.74×** | 1.87× |

Integration cost per block grows with the round (more candidate merges) while execution improves (more inter-block parallelism); the proposer stages are paid once per block by its proposer, so a validator pays 1/P of them. Round size 100 is the main campaign; the other sizes are the full-dataset round-size runs (`run_roundsize_full.sh`).


**Metadata shipped in a proposal**

| Payload | MB total | KB/block | vs calldata |
|---|---:|---:|---:|
| Transaction calldata (what a block already carries) | 1729.2 | 115.3 | 100% |
| Omakase per-block conflict graph (access-set hashes + WAW edges) | 211.3 | 14.1 | 12.2% |
| Vegeta schedule (same encoding) | 237.0 | 15.8 | 13.7% |

**Worst case across rounds (execution phase, in-memory)**

| Engine | mean speedup | worst round | p10 | rounds slower than sequential | p99/p50 latency |
|---|---:|---:|---:|---:|---:|
| Block-STM | 1.94× | 1.17× | 1.63× | 0 | 2.35 |
| Block-STM, concatenated | 2.35× | 0.75× | 1.81× | 8 | 3.71 |
| Omakase | 2.66× | 1.90× | 2.25× | 0 | 1.77 |

**Speedup by contention level** (rounds split by Block-STM's re-execution rate)

| rounds | Block-STM re-exec/tx | Block-STM | Concat | Omakase |
|---|---:|---:|---:|---:|
| low contention (bottom 20% Block-STM re-exec/tx) | 0.42 | 1.87× | 2.35× | 2.73× |
| high contention (top 20%) | 0.69 | 1.80× | 2.09× | 2.49× |

**Concatenated round: whole-window sequential fallback** — 8 of 150 rounds (concat re-executions = 0 in the diagnostics run; confirmed with `PEVM_TRACE_FALLBACK=1`: a transaction reads an account self-destructed earlier in the same optimistic window, pevm abandons the window and re-executes all of it sequentially). Per-block Block-STM and Omakase's <=10-block groups never fell back on these rounds because the self-destruct and the read land in different windows.

| chunk | round | txs | seq ms/block | Block-STM | Block-STM, concatenated | Graph OCC, concatenated | Graph-aware OCC | Omakase |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| real_16774645.csv | 41 | 15,896 | 6.53 | 1.99× | 0.78× | 0.87× | 1.74× | 2.36× |
| real_16774645.csv | 17 | 14,382 | 6.68 | 2.12× | 0.81× | 0.84× | 1.86× | 2.39× |
| real_16774645.csv | 14 | 14,110 | 7.69 | 1.96× | 0.86× | 0.89× | 1.93× | 2.67× |
| real_18581726.csv | 30 | 15,299 | 9.97 | 1.70× | 0.78× | 0.91× | 1.56× | 2.96× |
| real_19557289.csv | 4 | 17,481 | 10.17 | 1.87× | 0.93× | 0.96× | 1.69× | 2.21× |
| real_18581726.csv | 31 | 15,405 | 10.43 | 1.68× | 0.75× | 0.71× | 1.54× | 3.23× |
| real_19557289.csv | 34 | 17,917 | 10.51 | 1.93× | 0.77× | 0.82× | 1.81× | 2.77× |
| real_19557289.csv | 26 | 17,812 | 10.64 | 1.95× | 0.90× | 0.92× | 1.85× | 3.17× |

**Per-round statistics with and without the fallback rounds** (execution phase, speedup over sequential)

| rounds | engine | mean | worst | p10 | rounds < 1× | p99/p50 |
|---|---:|---:|---:|---:|---:|---:|
| all rounds | Block-STM | 1.94× | 1.17× | 1.63× | 0 | 2.35 |
| all rounds | Block-STM, concatenated | 2.35× | 0.75× | 1.81× | 8 | 3.71 |
| all rounds | Graph OCC, concatenated | 2.12× | 0.71× | 1.72× | 11 | 3.57 |
| all rounds | Graph-aware OCC | 1.80× | 1.16× | 1.54× | 0 | 2.17 |
| all rounds | Omakase | 2.66× | 1.90× | 2.25× | 0 | 1.77 |
| excluding fallback rounds | Block-STM | 1.94× | 1.17× | 1.62× | 0 | 2.36 |
| excluding fallback rounds | Block-STM, concatenated | 2.44× | 1.25× | 1.90× | 0 | 2.56 |
| excluding fallback rounds | Graph OCC, concatenated | 2.19× | 0.76× | 1.93× | 3 | 3.18 |
| excluding fallback rounds | Graph-aware OCC | 1.81× | 1.16× | 1.54× | 0 | 2.18 |
| excluding fallback rounds | Omakase | 2.66× | 1.90× | 2.26× | 0 | 1.73 |

**Worst genuine (non-fallback) rounds for the concatenated round**, with every engine's speedup and re-executions per tx

| chunk | round | txs | seq ms/block | Block-STM | Block-STM, concatenated | Graph OCC, concatenated | Graph-aware OCC | Omakase | re-exec/tx concat | Block-STM | Omakase |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| real_18581726.csv | 39 | 16,163 | 24.18 | 1.17× | 1.25× | 1.29× | 1.16× | 4.03× | 0.81 | 0.46 | 0.43 |
| real_18581726.csv | 11 | 14,306 | 14.49 | 1.37× | 1.52× | 1.47× | 1.33× | 2.29× | 0.90 | 0.54 | 0.47 |
| real_18581726.csv | 21 | 14,683 | 14.82 | 1.36× | 1.58× | 1.69× | 1.40× | 3.37× | 1.19 | 0.72 | 0.49 |
| real_18581726.csv | 43 | 15,510 | 10.76 | 1.53× | 1.67× | 1.98× | 1.45× | 2.89× | 1.16 | 0.55 | 0.55 |
| real_18581726.csv | 45 | 15,203 | 9.46 | 1.61× | 1.71× | 1.62× | 1.51× | 2.73× | 1.07 | 0.64 | 0.57 |
| real_18581726.csv | 18 | 15,544 | 11.81 | 1.51× | 1.74× | 1.72× | 1.45× | 3.26× | 0.99 | 0.57 | 0.53 |
| real_18581726.csv | 26 | 14,274 | 11.33 | 1.59× | 1.79× | 1.77× | 1.51× | 3.10× | 0.95 | 0.56 | 0.43 |
| real_18581726.csv | 3 | 13,339 | 9.30 | 1.53× | 1.81× | 2.23× | 1.45× | 2.96× | 1.18 | 0.67 | 0.57 |
| real_18581726.csv | 15 | 15,072 | 11.20 | 1.54× | 1.83× | 1.90× | 1.48× | 1.90× | 0.91 | 0.56 | 0.49 |
| real_18581726.csv | 19 | 15,573 | 9.84 | 1.62× | 1.85× | 1.73× | 1.53× | 2.10× | 1.09 | 0.70 | 0.47 |

**Aborts and re-executions** — 2,387,073 txs (diagnostics build)

| Engine | re-executions / tx | validation aborts / tx | cascade aborts / tx | re-exec writing a new location / tx | cascade share of aborts |
|---|---:|---:|---:|---:|---:|
| Block-STM | 0.544 | 0.183 | 0.1351 | 0.0073 | 74% |
| Block-STM, concatenated | 0.827 | 0.197 | 0.1832 | 0.0075 | 93% |
| Graph OCC, concatenated | 0.650 | 0.361 | 0.2634 | 0.0027 | 73% |
| Graph-aware OCC | 0.346 | 0.154 | 0.1035 | 0.0004 | 67% |
| Vegeta | 0.010 | 0.000 | 0.0000 | 0.0000 | 0% |
| Omakase | 0.420 | 0.207 | 0.1415 | 0.0008 | 68% |

*cascade abort* = validation failure caused by the *re-execution* of a lower-indexed transaction (the invalidating write carries incarnation > 0, is an ESTIMATE left by an aborted incarnation, or is a version the reader saw that a later incarnation no longer writes); an abort caused by a lower-indexed writer's first execution is an ordinary optimistic abort. *re-exec writing a new location* = re-executions whose write set gained a location the previous incarnation had not written (post-blocking first executions excluded).

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
| Vegeta (speculate + batch replay + serial tail) | 144.2 | 16,558 | 9.13 | 11.63 | 20.04 | 1.33× |
| Omakase | 81.9 | 29,163 | 5.09 | 6.42 | 14.99 | 2.34× |

**Where the time goes — full pipeline including the preparatory phases**

| Phase | total s | share | ms/block | cost as a fraction of sequential time |
|---|---:|---:|---:|---:|
| Pre-execute (proposer, once per block) | 197.4 | 65% | 13.16 | 1.03× |
| Build conflict graph + intra-block reorder | 11.5 | 4% | 0.77 | 0.06× |
| Integrate (greedy, per committed round) | 12.4 | 4% | 0.83 | 0.06× |
| Execute (Omakase, parallel) | 81.9 | 27% | 5.46 | 0.43× |
| All phases on one node (single proposer, P = 1) | 303.1 | 100% | 20.21 | 1.58× (speedup 0.63×) |
| Validator-side: integrate + execute | 94.3 |  | 6.28 | 0.49× (speedup 2.03×) |
| **Every stage on one node, 1/P of proposer stages, P = 20** | **104.7** |  | **6.98** | **0.55×** (speedup 1.83×) |
| **Every stage on one node, 1/P of proposer stages, P = 50** | **98.4** |  | **6.56** | **0.51×** (speedup 1.95×) |
| **Every stage on one node, 1/P of proposer stages, P = 100** | **96.3** |  | **6.42** | **0.50×** (speedup 1.99×) |
| Sequential execution (reference) | 191.6 |  | 12.77 | 1.00× |

Vegeta's schedule construction (Rule-1 reorder + DAG rebuild) on the same pre-pass: 31.88 s total, 2.125 ms/block.

**Every stage on one node with P proposers** — round size = P (one block per proposer per round); ms/block

| P | blocks | seq. | proposer stages (pre-exec + graph) | integrate | execute | validator + proposer/P | exec only | integ + exec | every stage on one node, 1/P | Block-STM |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 20 | 15,000 | 13.09 | 13.96 | 0.77 | 5.60 | 7.07 | 2.34× | 2.06× | **1.85×** | 2.01× |
| 50 | 15,000 | 13.07 | 13.91 | 0.82 | 5.57 | 6.66 | 2.35× | 2.05× | **1.96×** | 2.00× |
| 100 | 15,000 | 12.77 | 13.93 | 0.83 | 5.46 | 6.42 | 2.34× | 2.03× | **1.99×** | 1.96× |

Integration cost per block grows with the round (more candidate merges) while execution improves (more inter-block parallelism); the proposer stages are paid once per block by its proposer, so a validator pays 1/P of them. Round size 100 is the main campaign; the other sizes are the full-dataset round-size runs (`run_roundsize_full.sh`).


**Metadata shipped in a proposal**

| Payload | MB total | KB/block | vs calldata |
|---|---:|---:|---:|
| Transaction calldata (what a block already carries) | 1587.0 | 105.8 | 100% |
| Omakase per-block conflict graph (access-set hashes + WAW edges) | 192.2 | 12.8 | 12.1% |
| Vegeta schedule (same encoding) | 231.6 | 15.4 | 14.6% |

**Worst case across rounds (execution phase, in-memory)**

| Engine | mean speedup | worst round | p10 | rounds slower than sequential | p99/p50 latency |
|---|---:|---:|---:|---:|---:|
| Block-STM | 2.04× | 1.24× | 1.72× | 0 | 2.82 |
| Block-STM, concatenated | 2.85× | 1.59× | 2.25× | 0 | 2.99 |
| Omakase | 2.43× | 1.43× | 2.05× | 0 | 2.94 |

**Speedup by contention level** (rounds split by Block-STM's re-execution rate)

| rounds | Block-STM re-exec/tx | Block-STM | Concat | Omakase |
|---|---:|---:|---:|---:|
| low contention (bottom 20% Block-STM re-exec/tx) | 0.49 | 2.20× | 3.40× | 2.64× |
| high contention (top 20%) | 0.92 | 1.56× | 2.23× | 1.85× |

**Aborts and re-executions** — 1,226,448 txs (diagnostics build)

| Engine | re-executions / tx | validation aborts / tx | cascade aborts / tx | re-exec writing a new location / tx | cascade share of aborts |
|---|---:|---:|---:|---:|---:|
| Block-STM | 0.683 | 0.205 | 0.1463 | 0.0000 | 71% |
| Block-STM, concatenated | 1.067 | 0.207 | 0.1805 | 0.0000 | 87% |
| Graph OCC, concatenated | 0.000 | 0.000 | 0.0000 | 0.0000 | 0% |
| Graph-aware OCC | 0.000 | 0.000 | 0.0000 | 0.0000 | 0% |
| Vegeta | 0.000 | 0.000 | 0.0000 | 0.0000 | 0% |
| Omakase | 0.000 | 0.000 | 0.0000 | 0.0000 | 0% |

*cascade abort* = validation failure caused by the *re-execution* of a lower-indexed transaction (the invalidating write carries incarnation > 0, is an ESTIMATE left by an aborted incarnation, or is a version the reader saw that a later incarnation no longer writes); an abort caused by a lower-indexed writer's first execution is an ordinary optimistic abort. *re-exec writing a new location* = re-executions whose write set gained a location the previous incarnation had not written (post-blocking first executions excluded).

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
| 6 | 10% | 0.100 | 0.109 | 0.000 | 0.000 | 0.000 | 0.000 |
| 8 | 10% | 0.100 | 0.110 | 0.000 | 0.000 | 0.000 | 0.000 |
| 2 | 20% | 0.453 | 0.471 | 0.000 | 0.000 | 0.000 | 0.000 |
| 4 | 20% | 0.452 | 0.594 | 0.000 | 0.000 | 0.000 | 0.000 |
| 6 | 20% | 0.453 | 0.710 | 0.000 | 0.000 | 0.000 | 0.000 |
| 8 | 20% | 0.452 | 0.794 | 0.000 | 0.000 | 0.000 | 0.000 |
| 2 | 30% | 0.943 | 1.056 | 0.000 | 0.000 | 0.000 | 0.000 |
| 4 | 30% | 0.944 | 1.320 | 0.000 | 0.000 | 0.000 | 0.000 |
| 6 | 30% | 0.944 | 1.552 | 0.000 | 0.000 | 0.000 | 0.000 |
| 8 | 30% | 0.941 | 1.710 | 0.000 | 0.000 | 0.000 | 0.000 |
| 2 | 40% | 1.536 | 1.840 | 0.000 | 0.000 | 0.000 | 0.000 |
| 4 | 40% | 1.536 | 2.257 | 0.000 | 0.000 | 0.000 | 0.000 |
| 6 | 40% | 1.527 | 2.625 | 0.000 | 0.000 | 0.000 | 0.000 |
| 8 | 40% | 1.530 | 2.852 | 0.000 | 0.000 | 0.000 | 0.000 |

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


## Emulated state-access latency — real Ethereum (20 Cancun batches, t=8)

| read latency µs | seq ms/block | Block-STM × | Concat × | Omakase exec × | Omakase exec+integrate × | integrate ms/block | integrate share of validator time | exec saved vs Block-STM ms/block | re-exec/tx Block-STM | Omakase |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 | 10.6 | 1.95 | 2.37 | 2.74 | 1.86 | 1.85 | 32% | 1.57 | 0.62 | 0.42 |
| 1 | 12.4 | 2.05 | 2.50 | 2.78 | 1.95 | 1.90 | 30% | 1.58 | 0.61 | 0.42 |
| 2 | 13.8 | 2.10 | 2.58 | 2.78 | 2.03 | 1.84 | 27% | 1.61 | 0.61 | 0.41 |
| 5 | 18.5 | 2.25 | 2.76 | 2.83 | 2.20 | 1.89 | 22% | 1.69 | 0.60 | 0.40 |
| 10 | 26.0 | 2.35 | 2.86 | 2.82 | 2.35 | 1.85 | 17% | 1.86 | 0.59 | 0.40 |
| 20 | 41.1 | 2.43 | 2.93 | 2.81 | 2.49 | 1.88 | 11% | 2.33 | 0.58 | 0.41 |
| 50 | 85.5 | 2.43 | 2.96 | 2.74 | 2.59 | 1.83 | 6% | 3.90 | 0.57 | 0.43 |

Every account/slot read pays the latency in every engine; graph construction and integration never touch state and stay constant.


**Read latency × worker count** (real Ethereum, 20 batches; speedup over sequential at the same latency)

| read latency µs | workers | Block-STM | Concat | Omakase exec | Omakase exec+integ |
|---|---:|---:|---:|---:|---:|
| 20 | 8 | 2.43 | 2.93 | 2.81 | 2.49 |
| 20 | 16 | 2.19 | 2.83 | 2.69 | 2.37 |
| 20 | 32 | 2.08 | 2.58 | 2.51 | 2.30 |
| 50 | 8 | 2.43 | 2.96 | 2.74 | 2.59 |
| 50 | 16 | 2.39 | 2.96 | 2.72 | 2.55 |
| 50 | 32 | 2.28 | 2.87 | 2.54 | 2.43 |


## Live 4-validator Mysticeti deployment — minround1 (ERC20 8x4x8, steady state, per validator)

| execution mode | load step | committed tx/s | executor busy | implied executor capacity tx/s | blocks/round | rounds |
|---|---:|---:|---:|---:|---:|---:|
| sequential | 0 | 39,949 | 42% | 94,610 | 1.3 | 20,390 |
| parallel | 0 | 39,986 | 46% | 86,878 | 1.3 | 21,160 |
| concatenated | 0 | 39,961 | 43% | 92,289 | 1.2 | 22,710 |
| integrated | 0 | 39,943 | 84% | 47,551 | 3.9 | 3,957 |
| sequential | 1 | 39,907 | 42% | 94,399 | 1.2 | 21,898 |
| parallel | 1 | 39,962 | 47% | 84,980 | 1.2 | 22,847 |
| concatenated | 1 | 39,931 | 44% | 90,907 | 1.2 | 22,678 |
| integrated | 1 | 39,913 | 84% | 47,291 | 4.0 | 3,554 |

The generator caps committed throughput at ~40k tx/s per validator in every mode, so the executor is never the bottleneck here; *executor busy* is the share of wall-clock the executor spends on its round (all stages, pre-execution and integration included for Omakase), and *implied capacity* = tx/s ÷ busy. Load step 0/1 = successive PEVM_LOAD settings.


## Live 4-validator Mysticeti deployment — minround1000 (ERC20 8x4x8, steady state, per validator)

| execution mode | load step | committed tx/s | executor busy | implied executor capacity tx/s | blocks/round | rounds |
|---|---:|---:|---:|---:|---:|---:|
| sequential | 0 | 39,405 | 39% | 100,780 | 9.8 | 2,638 |
| parallel | 0 | 39,968 | 44% | 90,069 | 9.8 | 2,681 |
| concatenated | 0 | 40,012 | 32% | 123,685 | 9.8 | 2,738 |
| integrated | 0 | 39,970 | 80% | 50,245 | 5.3 | 1,930 |

The generator caps committed throughput at ~40k tx/s per validator in every mode, so the executor is never the bottleneck here; *executor busy* is the share of wall-clock the executor spends on its round (all stages, pre-execution and integration included for Omakase), and *implied capacity* = tx/s ÷ busy. Load step 0/1 = successive PEVM_LOAD settings.


## Live Omakase executor cost per phase — phases_minround1000 (us per committed tx, steady state)

| validator | rounds | blocks/round | total | pre-execute + graph | integrate | execute | own 1/4 of proposer stages |
|---|---:|---:|---:|---:|---:|---:|---:|
| integrated_v0.log | 1733 | 4.7 | 20.37 | 10.20 | 2.72 | 4.26 | 12.72 |
| integrated_v1.log | 1729 | 4.7 | 20.45 | 10.28 | 2.70 | 4.28 | 12.74 |
| integrated_v2.log | 1726 | 4.7 | 20.56 | 10.40 | 2.70 | 4.00 | 12.77 |
| integrated_v3.log | 1731 | 4.7 | 20.65 | 10.40 | 2.77 | 4.26 | 12.86 |
| **mean** |  |  | **20.51** | **10.32** | **2.72** | **4.20** | **12.77** |

The prototype re-derives every block's graph at every validator (it does not ship graphs with proposals), so the pre-execute + graph column is counted P = 4 times; the last column charges each validator only its own quarter of it. The remainder (total minus the three phases) is hex decoding and state bookkeeping.
