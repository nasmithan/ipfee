# IPFee - Solana Validator IP Fee Tracking

This runs together with Solana validator software to track IP addresses and the fees for each address. It uses some code and insights from https://github.com/bji/txingest-receiver and https://github.com/rpcpool/tpu-traffic-classifier. Thanks to Zantentu and Triton for their influence and some of the code.

See analysis.txt for 24hrs of analysis on a 300k SOL staked node.
<img width="704" alt="image" src="https://github.com/nasmithan/ipfee/assets/6745038/669ae32f-d6b5-4dd4-b8aa-06cb40b6343f">

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

Add `--ipfee-host 127.0.0.1:15111` to your startup command and then restart your validator. Be sure to `sudo systemctl daemon-reload` if needed.

### Step 2. Jito Relayer with ipfee

Setup your own relayer with these instructions if you don't already have one: https://jito-foundation.gitbook.io/mev/jito-relayer/running-a-relayer

_IMPORTANT_: You must update Cargo.toml with a path to your solana SDK built in Step 1.
Update `/home/solana` (my user name is solana) in the below command to the directory your jito-solana folder is in.

```
git clone https://github.com/nasmithan/jito-relayer.git
cd jito-relayer
git checkout v0.1.12-ipfee

sed -i 's|/path/to/your/jito-solana|/home/solana/jito-solana|g' Cargo.toml

# Verify the path looks correct
cat Cargo.toml

git submodule update --init --recursive
cargo build --release
```

Add `--ipfee-host 127.0.0.1:15111` to your startup command and then restart your relayer. Be sure to `sudo systemctl daemon-reload` if needed.

### Step 3. Build & run ipfee

```
git clone https://github.com/nasmithan/ipfee.git
cd ipfee

sed -i 's|/path/to/your/jito-solana|/home/solana/jito-solana|g' Cargo.toml

# Verify the path looks correct
cat Cargo.toml

cargo build --release
```

Run the file (need to run as root user if you want to add the ipset blocks)

```
target/release/ipfee 127.0.0.1 15111 ipfee.json
```

You can view the json output pretty printed this this command: `jq -r '["IP", "TxCount", "DupCount", "AvgFee", "MinFee", "MaxFee", "Blocked"], (to_entries | sort_by(-.value.tx_count)[] | [.key, .value.tx_count, .value.dup_count, .value.avg_fee, .value.min_fee, .value.max_fee, .value.blocked]) | @tsv' ipfee.json | column -t`

### Step 4. Run new binaries

You need to restart your validator and relayer with the the `--ipfee-host` flag, and then you need to run

### Firewall rules

This will add an ipset, but you will need to firewall it off.

```
sudo ipset create custom-blocklist-ips hash:net

sudo iptables -F solana-tpu-custom-quic
sudo iptables -A solana-tpu-custom-quic -m set --match-set custom-blocklist-ips src -j DROP
sudo iptables -A solana-tpu-custom-quic -j ACCEPT

# Optionally block all 11229 fwd traffic
sudo iptables -I ufw-user-input 1 -p udp --dport 11229 -j DROP

```

View IPs blocked: `sudo ipset list custom-blocklist-ips`
Flush IPs blocked: `sudo ipset flush custom-blocklist-ips`

## Ideas / TODO

- 2 caches, one close to leader slots and one far away.
- Store/read from .json file?
- Add local DB to store cached information each time banning is performed. Load on startup.
- Halve data every X hours?
- Unblock IPs occassionally?
- Temporary jail IPs
- Cluster analysis? Find outliers and boot them?

## References

### nasmithan ipfee

When building jito-solana, you need to run `git submodule update --init --recursive`

- jito-solana https://github.com/jito-foundation/jito-solana/compare/v1.17.31-jito...nasmithan:jito-solana:v1.17.31-ipfee
- jito relayer https://github.com/jito-foundation/jito-relayer/compare/master...nasmithan:jito-relayer:v0.1.12-ipfee

### bji txingester

- jito-solana bji https://github.com/solana-labs/solana/compare/v1.17.31...bji:solana:v1.17.31-jito_txingest
- jito relayer https://github.com/jito-foundation/jito-relayer/compare/master...bji:jito-relayer:v0.1.12_txingest
