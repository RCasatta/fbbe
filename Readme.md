

# Fast Bitcoin Block Explorer

A light Bitcoin block explorer without address indexing, using only a bitcoin core instance.

Runs live on mainnet, testnet and signet @ http://fbbe.info

Locally can run on regtest.

## Running locally

Supposing to have [rust installed](https://www.rust-lang.org/tools/install) and a synced [bitcoin core](https://bitcoincore.org/en/download/) on mainnet with `txindex=1` and `rest=1` do:

```
git clone https://github.com/RCasatta/fbbe
cd fbbe
cargo run --release
```

## Nix

A flake is provided to build the fbbe binary.

Whenever `Cargo.toml` or `Cargo.lock` are updated the `cargo2nix` tool must be launched to keep `Cargo.nix` in sync. See https://github.com/cargo2nix/cargo2nix

```
nix run github:cargo2nix/cargo2nix   # needs nix-command and flakes experimental feature enabled 
```

## CSS

After changing `custom.css` use a minifier:

```
yui-compressor custom.css > custom.min.css
```

## Mainnet test cases

* Block with most tx 00000000000000001080e6de32add416cd6cda29f35ec9bce694fea4b964c7be

* max inputs per tx 52539a56b1eb890504b775171923430f0355eb836a57134ba598170a2f8980c1

* max outputs per tx dd9f6bbf80ab36b722ca95d93268667a3ea6938288e0d4cf0e7d2e28a7a91ab3
