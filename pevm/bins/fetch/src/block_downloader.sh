#!/bin/bash

RPC_URL="https://ethereum.publicnode.com"
# RPC_URL="https://eth-mainnet.g.alchemy.com/v2/LuubbU5Y_KqTYngcB_lyd"
START_BLOCK=18581726
COUNT=100

for ((i=0; i<COUNT; i++)); do
  BLOCK=$((START_BLOCK + i))
  echo "Fetching block $BLOCK"
  cargo run --bin pevm-fetch $RPC_URL $BLOCK
done
