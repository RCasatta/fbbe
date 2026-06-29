# Address Index Schema

The optional address index is a RocksDB database opened by `Database::new` in
`src/threads/index_addresses.rs`. It is enabled when `--addr-index-path` is
provided.

The index is not a full transaction database. It stores compact lookup keys and
block heights, then loads the corresponding blocks from Bitcoin Core REST when a
page needs exact transaction data.

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

`script_hash` is `FxHasher64(script_pubkey_bytes)`. During indexing, each block
stores at most one funding key per distinct script hash, even if that script
appears in multiple outputs in the same block.

Address page lookup computes the queried address script hash, iterates this
column family in reverse height order, loads the matching blocks, and scans the
full blocks for outputs whose script exactly matches the queried script. The
current UI returns at most 10 matching funding heights.

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

## Derived Behavior

The current schema supports:

- address pages listing recent confirmed funding outputs for an address;
- address pages showing whether those displayed outputs were later spent;
- transaction pages showing confirmed spent/unspent status for outputs;
- links from a spent output to the spending transaction;
- fee lookup caching.

The schema does not store full scripts, full outpoints, full transaction ids for
funding appearances, or spending transaction ids. Those are recovered by loading
and scanning blocks from Bitcoin Core.
