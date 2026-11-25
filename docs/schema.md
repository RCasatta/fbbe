# Address Index Schema

The optional address index is a RocksDB database opened by `Database::new` in
`src/threads/index_addresses.rs`. It is enabled when `--addr-index-path` is
provided.

The index is not a full transaction database. It stores compact lookup keys and
block heights, then loads the corresponding blocks from Bitcoin Core REST when a
page needs exact transaction data. The one exception is the confirmed balance of
each script, kept as a running total in `BALANCE_CF`.

Indexing the balances needs the outputs spent by each block, from the
`/rest/spenttxouts/<block_hash>.bin` endpoint of Bitcoin Core 30 or later.

## Script Hash

Scripts are keyed by `script_hash`: SipHash-2-4 of the `script_pubkey` bytes,
keyed with 16 random bytes generated when the database is created and stored in
`META_CF`, as an 8-byte big-endian `u64`. Without the key nobody can craft a
script whose hash collides with a given address, which could otherwise alter its
balance.

## Column Families

### `BLOCK_HASH_CF`

Tracks blocks that have already been indexed.

Key:

```text
block_hash: 32 bytes
```

Value:

```text
empty
```

The startup indexer reads this column family into memory and skips any block
hash already present.

### `FUNDING_CF`

Finds blocks where an address script appears in transaction outputs.

Key:

```text
script_hash: 8 bytes, big-endian
height:      4 bytes, big-endian
```

Value:

```text
empty
```

During indexing, each block
stores at most one funding key per distinct script hash, even if that script
appears in multiple outputs in the same block.

Address page lookup computes the queried address script hash, iterates this
column family in reverse height order, loads the latest matching block, and
scans the full block for outputs whose script exactly matches the queried
script.

### `SPENDING_CF`

Finds the block height where an outpoint is spent.

Key:

```text
outpoint_key: 8 bytes, big-endian
height:       4 bytes, big-endian
```

Value:

```text
empty
```

`outpoint_key` is a compact key derived from the spent outpoint:

```text
u64::from_be_bytes(outpoint.txid[..8]) + outpoint.vout
```

Transaction pages use this column family to mark confirmed outputs as spent.
Address pages use it to find the spending block for each displayed funding
outpoint. After a spending height is found, the code loads that block and scans
its inputs to recover the exact spending transaction id and input index.

This key is compact, but it is not collision-free. A collision can point the
lookup at an unrelated block height; the later full-block scan may then fail to
find the exact outpoint.

### `FEE_CF`

Caches transaction fees.

Key:

```text
txid_prefix: 12 bytes
```

Value:

```text
fee_sats: 8 bytes, big-endian u64
```

This is separate from the address index lookups, but it lives in the same
RocksDB database.

### `BALANCE_CF`

The confirmed balance of each script, as a running total.

Key:

```text
script_hash: 8 bytes, big-endian
```

Value:

```text
balance_sats: zigzag LEB128 varint of an i64
```

For each block the indexer computes, per script, the value of the outputs paying
it minus the value of the outputs it spent, and writes the non-zero changes as
RocksDB merge operands. The merge operator (`merge_sum`) sums them, so indexing
never reads before writing. Operands and the stored value share the signed
encoding because changes are often negative; a correct balance never is, and a
negative one is reported as an index needing a reindex.

A missing key means a balance of 0. Scripts that were used and are now empty keep
a key with value 0; `FUNDING_CF` tells whether a script was ever used.

Only blocks buried more than `REORG_DEPTH` (6) blocks reach this column family,
see [Recent Blocks and Reorgs](#recent-blocks-and-reorgs).

### `META_CF`

Database metadata, under single-byte keys:

| Key | Value |
| --- | --- |
| `V` | format version, 4 bytes, big-endian `u32`, currently 1 |
| `K` | the 16-byte script hash key |
| `T` | balance tip: height, 4 bytes big-endian, then block hash, 32 bytes |

The format version and the key are written together when the database is
created. A database without `META_CF`, created by a release from before this
format existed, is refused: it must be rebuilt in an empty `--addr-index-path`.

The balance tip is the last block counted in `BALANCE_CF`. It is written in the
same batch as that block's merge operands, so each block is counted exactly
once, even if the process is killed.

## Recent Blocks and Reorgs

The last `REORG_DEPTH` blocks are kept in memory, as the balance changes of each
block, and are not yet in `BALANCE_CF`. A block is accepted only if it follows the
last one, by height and previous block hash. When more than `REORG_DEPTH` blocks
are held, the oldest is written to `BALANCE_CF` with the new balance tip.

A balance read is the value in `BALANCE_CF` plus the changes in the recent
blocks, both read under the same lock.

On a new tip, recent blocks no longer in the active chain are dropped and the
missing ones are fetched and pushed again. `BALANCE_CF` never needs undoing. A
reorg deeper than `REORG_DEPTH`, replacing the balance tip itself, is detected
and makes balances unavailable until a reindex.

After a restart the recent blocks are empty: the indexer pushes again the blocks
after the balance tip.

The other column families are add-only and are not undone on reorgs either:
pages always check them against the block loaded from Bitcoin Core, so stale
entries are harmless.

## Derived Behavior

The current schema supports:

- address pages listing recent confirmed funding outputs for an address;
- address pages showing whether those displayed outputs were later spent;
- address pages showing the confirmed balance, with one point lookup;
- transaction pages showing confirmed spent/unspent status for outputs;
- links from a spent output to the spending transaction;
- fee lookup caching.

The schema does not store full scripts, full outpoints, full transaction ids for
funding appearances, spending transaction ids, or the history of balances. Those
are recovered by loading and scanning blocks from Bitcoin Core, apart from the
balance history, which is not available.
