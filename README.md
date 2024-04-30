# IPFee [DRAFT]

This runs together with Solana validator software to track IP addresses and the fees for each address. It is a combination of https://github.com/bji/txingest-receiver and https://github.com/rpcpool/tpu-traffic-classifier to put IPs into different categories based on the fees paid, to prioritize high fee payers and throttle low fee payer IPs. Thanks to Zantentus and Triton for their influence and much of the code included so far.

This still needs the firewall rules and combination with tpuclassifier which haven't been committed yet.

## Setup

These instructions are for _jito validators running their own relayer_.

### Step 1. Jito Solana with ipfee

```
git clone https://github.com/nasmithan/jito-solana.git
cd jito-solana
git checkout v1.17.31-ipfee
git submodule update --init --recursive
cargo build --release
```

Add `--ipfee-host 127.0.0.1:15111` to your startup command and then restart your validator.

### Step 2. Jito Relayer with ipfee

Setup your own relayer with these instructions if you don't already have one: https://jito-foundation.gitbook.io/mev/jito-relayer/running-a-relayer

_IMPORTANT_: You must update Cargo.toml with a path to your solana SDK built in Step 1.
Update `/home/me` in the below command to the directory your jito-solana folder is in.

```
git clone https://github.com/nasmithan/jito-relayer.git
cd jito-relayer
git checkout v0.1.12-ipfee

sed -i 's|/path/to/your/jito-solana|/home/me/jito-solana|g' Cargo.toml

git submodule update --init --recursive
cargo build --release
```

Add `--ipfee-host 127.0.0.1:15111` to your startup command and then restart your relayer.

### Step 3. Build & run ipfee

```
git clone https://github.com/nasmithan/ipfee.git
cd ipfee

sed -i 's|/path/to/your/jito-solana|/home/solana/jito-solana|g' Cargo.toml

cargo build --release

[TODO: ADD CONFIG.yml?]

target/release/ipfee 127.0.0.1 15111
```

### Step 4. Run new binaries

You need to restart your validator and relayer with the the `--ipfee-host` flag, and then you need to run

## Ideas / TODO

Variables to define:
REFRESH_IP_SETS_FREQUENCY = 5000 # Defaults to every 5000 txs observed

1. Creates a hashmap of <ip_addr, (tx_count, avg_fee)>
2. Every REFRESH_IP_SETS_FREQUENCY txs observed, update ipsets
3. Of the top 80% of tx count senders, those that are in the bottom 10% of fee senders will be throttled.

- print IP stats, overlap with high/low/gossip
- env var to how often to divide everything by 2
- commit pre built binaries for east setup

- Track incoming traffic with iptables, filter any IP not sending high quality txs or any obvious abusers.

## References

### bji txingester

- jito-solana bji https://github.com/solana-labs/solana/compare/v1.17.31...bji:solana:v1.17.31-jito_txingest
- jito relayer https://github.com/jito-foundation/jito-relayer/compare/master...bji:jito-relayer:v0.1.12_txingest

### nasmithan ipfee

When building jito-solana, you need to run `git submodule update --init --recursive`

- jito-solana https://github.com/jito-foundation/jito-solana/compare/v1.17.31-jito...nasmithan:jito-solana:v1.17.31-ipfee
- jito relayer https://github.com/jito-foundation/jito-relayer/compare/master...nasmithan:jito-relayer:v0.1.12-ipfee

# What are shinobi's stats? Juicy? Aurora? Latitude?
