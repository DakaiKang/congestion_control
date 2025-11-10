// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

contract Benchmark {
    // Equivalent to TestTables.resource_table: mapping<u64 => u64>
    mapping(uint256 => uint256) public resourceTable;

    /// Equivalent to loop_exchange
    function loopExchange(uint256 loopCount, uint256[] calldata resources) external {
        uint256 length = resources.length;

        // First loop: add/update all in resources
        for (uint256 i = 0; i < length; i++) {
            resourceTable[resources[i]] += 1;
        }

        // Second loop: loopCount iterations, cycling through resources
        for (uint256 i = 0; i < loopCount; i++) {
            uint256 j = i % length;
            resourceTable[resources[j]] += 1;
        }
    }

    /// Equivalent to exchange
    function exchange(uint256 resource) external {
        for (uint256 i = 0; i < 8; i++) {
            resourceTable[resource] += 1;
        }
    }

    /// Equivalent to exchangetwo
    function exchangeTwo(uint256 resource1, uint256 resource2) external {
        for (uint256 i = 0; i < 8; i++) {
            resourceTable[resource1] += 1;
            resourceTable[resource2] += 1;
        }
    }
}
