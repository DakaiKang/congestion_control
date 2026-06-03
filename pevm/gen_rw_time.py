"""Convert rw_gas_<N>.json + block_<N>.json → rw_time_<N>.json.

Output schema (per tx):
  { txHash, from, reads, writes, executionTime }
where:
  from          = block_<N>.json transaction's `from` field (real caller)
  executionTime = (gasUsed / 100) * T_SLOAD_NS  (ns; chosen so the V2 loop
                  count `target = executionTime / T_SLOAD_NS` reproduces V1's
                  `target = gasUsed / 100`. NOT a real wall-clock measurement.)

Matches rw_gas entries to block txs by `txHash` (positional match is verified
as a fallback). Skips blocks whose rw_gas / block files are missing or whose
tx counts disagree.

Usage:
  python3 gen_rw_time.py [--rw-gas DIR] [--blocks DIR] [--out DIR] [--limit N]

Defaults:
  --rw-gas /home/ubuntu/eth-block-data/rw_gas
  --blocks /home/ubuntu/eth-block-data/blocks_rw
  --out    /home/ubuntu/eth-block-data/rw_time
"""
import argparse
import json
import re
import sys
from pathlib import Path

# Must match T_SLOAD_NS in tests/tx_simulator/mod.rs.
# Recalibrate via `test_calibrate_sload_ns` if the host machine changes.
T_SLOAD_NS = 482


def parse_args():
    p = argparse.ArgumentParser()
    p.add_argument("--rw-gas", default="/home/ubuntu/eth-block-data/rw_gas")
    p.add_argument("--blocks", default="/home/ubuntu/eth-block-data/blocks_rw")
    p.add_argument("--out",    default="/home/ubuntu/eth-block-data/rw_time")
    p.add_argument("--limit",  type=int, default=None,
                   help="process at most N rw_gas files (for smoke tests)")
    p.add_argument("--scale",  type=float, default=1.0,
                   help="multiply executionTime by this factor "
                        "(scales the V2 per-tx loop count)")
    return p.parse_args()


def block_num_from(stem: str) -> int:
    m = re.match(r"^rw_gas_(\d+)$", stem)
    if not m:
        raise ValueError(f"unexpected rw_gas filename stem: {stem}")
    return int(m.group(1))


def main():
    args = parse_args()
    rw_gas_dir = Path(args.rw_gas)
    blocks_dir = Path(args.blocks)
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)

    rw_gas_files = sorted(rw_gas_dir.glob("rw_gas_*.json"))
    if args.limit is not None:
        rw_gas_files = rw_gas_files[: args.limit]

    print(f"rw_gas dir : {rw_gas_dir}  ({len(rw_gas_files)} files)")
    print(f"blocks dir : {blocks_dir}")
    print(f"out dir    : {out_dir}")

    written = 0
    skipped_missing_block = 0
    skipped_count_mismatch = 0
    hash_mismatches = 0

    for i, rw_gas_path in enumerate(rw_gas_files):
        block_num = block_num_from(rw_gas_path.stem)
        block_path = blocks_dir / f"block_{block_num}.json"
        if not block_path.exists():
            skipped_missing_block += 1
            continue

        with rw_gas_path.open() as f:
            rw_entries = json.load(f)
        with block_path.open() as f:
            block = json.load(f)

        block_txs = block.get("transactions") or []
        # Some snapshots store hashes in a parallel array (no "hash" field
        # inside each tx); handle both shapes.
        block_hashes = block.get("hashes")

        if len(rw_entries) != len(block_txs):
            print(f"  block {block_num}: count mismatch "
                  f"(rw_gas={len(rw_entries)}, block_txs={len(block_txs)}) — skipped")
            skipped_count_mismatch += 1
            continue

        out_entries = []
        for idx, (rw, tx) in enumerate(zip(rw_entries, block_txs)):
            tx_hash = rw.get("txHash") or rw.get("hash")
            # Verify positional match via hash when both are present.
            block_hash = tx.get("hash")
            if block_hash is None and block_hashes and idx < len(block_hashes):
                block_hash = block_hashes[idx]
            if tx_hash and block_hash and tx_hash.lower() != block_hash.lower():
                hash_mismatches += 1

            from_addr = tx.get("from")
            if from_addr is None:
                # As a last-ditch, leave from blank and let downstream detect.
                from_addr = ""

            gas_used = int(rw.get("gasUsed", 0))
            # V1 heuristic: target = gasUsed / 100. Scale by T_SLOAD_NS so the
            # V2 loader's `target = executionTime / T_SLOAD_NS` recovers it.
            execution_time = int((gas_used // 100) * T_SLOAD_NS * args.scale)

            out_entries.append({
                "txHash":        tx_hash,
                "from":          from_addr,
                "reads":         rw.get("reads",  []),
                "writes":        rw.get("writes", []),
                "executionTime": execution_time,
            })

        out_path = out_dir / f"rw_time_{block_num}.json"
        with out_path.open("w") as f:
            json.dump(out_entries, f, separators=(",", ":"))
        written += 1

        if (i + 1) % 500 == 0:
            print(f"  ... {i + 1}/{len(rw_gas_files)} processed")

    print()
    print(f"wrote {written} rw_time_*.json files to {out_dir}")
    if skipped_missing_block:
        print(f"skipped {skipped_missing_block} (no matching block_<N>.json)")
    if skipped_count_mismatch:
        print(f"skipped {skipped_count_mismatch} (rw_gas / block tx count mismatch)")
    if hash_mismatches:
        print(f"warning: {hash_mismatches} txHash/block-hash positional mismatches "
              f"(rw_gas order may not equal block tx order — verify a sample)")


if __name__ == "__main__":
    sys.exit(main())
