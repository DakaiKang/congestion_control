// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @notice Simulates Ethereum transaction execution by replaying each tx's
///         read/write set until a target access count is reached.
///
/// Unlike TxSimulator (which derives target from gasUsed / 100), this version
/// accepts `target` directly.  The caller pre-computes:
///
///     target = executionTime_ns / t_sload_ns
///
/// where `t_sload_ns` is the measured wall-clock cost of one hot SLOAD in the
/// local EVM environment.  This makes the synthetic workload's execution-time
/// distribution match the real transaction execution-time distribution rather
/// than the gas distribution.
///
/// Conflicts emerge naturally: two txs conflict if they share a key in their
/// write sets, or one reads what the other writes.
contract TxSimulatorV2 {

    /// Shared state — conflicts emerge when two txs touch the same key.
    mapping(bytes32 => uint256) public state;

    /// @param reads   hashed storage keys this tx reads
    /// @param writes  hashed storage keys this tx writes
    /// @param target  number of storage accesses to perform
    function execute(
        bytes32[] calldata reads,
        bytes32[] calldata writes,
        uint256 target
    ) external {
        uint256 count = 0;

        while (count < target) {
            for (uint256 i = 0; i < reads.length && count < target; i++) {
                state[reads[i]];
                count++;
            }
            for (uint256 i = 0; i < writes.length && count < target; i++) {
                state[writes[i]] = 1;
                count++;
            }
        }
    }
}
