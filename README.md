# IPFee

This runs together with Solana validator software to track IP addresses and the fees for each address. It is a combination of https://github.com/bji/txingest-receiver and https://github.com/rpcpool/tpu-traffic-classifier to put IPs into different categories based on the fees paid, to prioritize high fee payers and throttle low fee payer IPs.

Variables to define:
REFRESH_IP_SETS_FREQUENCY = 5000 # Defaults to every 5000 txs observed

1. Creates a hashmap of <ip_addr, (tx_count, avg_fee)>
2. Every REFRESH_IP_SETS_FREQUENCY txs observed, update ipsets
3. Of the top 80% of tx count senders, those that are in the bottom 10% of fee senders will be throttled.

## Setup

These instructions are for _jito validators running their own relayer_.

### Step 1. Jito Solana with ipfee

```
git clone https://github.com/nasmithan/solana-jito.git
cd solana-jito
git checkout ip-fees-1.17.31
git submodule update --init --recursive
cargo build --release
```

Add `--ipfee-host 127.0.0.1:15555` to your startup command and then restart your validator.

### Step 2. Jito Relayer with ipfee

Setup your own relayer with these instructions if you don't already have one: https://jito-foundation.gitbook.io/mev/jito-relayer/running-a-relayer

_IMPORTANT_: You must update Cargo.toml with a path to your solana SDK built in Step 1.
Update `/home/me` in the below command to the directory your solana-jito folder is in.

```
git clone https://github.com/nasmithan/jito-relayer.git
cd jito-relayer
git checkout ip-fees-1.17.31

sed -i 's|/path/to/your/jito-solana|/home/me/jito-solana|g' Cargo.toml

git submodule update --init --recursive
cargo build --release
```

Add `--ipfee-host 127.0.0.1:15555` to your startup command and then restart your relayer.

### Step 3. Build & run ipfee

```
git clone https://github.com/nasmithan/ipfee.git
cd ipfee
cargo build --release

[TODO: ADD CONFIG.yml?]

target/release/ipfee 127.0.0.1 15555
```

### Step 4. Run new binaries

You need to restart your validator and relayer with the the `--ipfee-host` flag, and then you need to run

## Ideas

- print IP stats, overlap with high/low/gossip
- env var to how often to divide everything by 2
- commit pre built binaries for east setup

## References

### bji txingester

- solana https://github.com/solana-labs/solana/compare/v1.17.31...bji:solana:v1.17.31_txingest
- solana-jito bji https://github.com/solana-labs/solana/compare/v1.17.31...bji:solana:v1.17.31-jito_txingest
- jito relayer https://github.com/jito-foundation/jito-relayer/compare/master...bji:jito-relayer:v0.1.12_txingest

### nasmithan ipfee

When building solana-jito, you need to run `git submodule update --init --recursive`

- solana ipfee https://github.com/solana-labs/solana/compare/v1.17.31...nasmithan:solana:ip-fees-1.17.31
- solana-jito https://github.com/jito-foundation/jito-solana/compare/v1.17.31-jito...nasmithan:solana-jito:ip-fees-1.17.31
- jito relayer https://github.com/jito-foundation/jito-relayer/compare/master...nasmithan:jito-relayer:ip-fees-1.17.31

# What are shinobi's stats? Juicy? Aurora? Latitude?
