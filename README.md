# IPFee

This runs together with Solana validator software to track IP addresses and the fees for each address. It is a combination of https://github.com/bji/txingest-receiver and https://github.com/rpcpool/tpu-traffic-classifier to put IPs into different categories based on the fees paid, to prioritize high fee payers and throttle low fee payer IPs.

Variables to define:
REFRESH_IP_SETS_FREQUENCY = 5000 # Defaults to every 5000 txs observed

1. Creates a hashmap of <ip_addr, (tx_count, avg_fee)>
2. Every REFRESH_IP_SETS_FREQUENCY txs observed, update ipsets
3. Of the top 80% of tx count senders, those that are in the bottom 10% of fee senders will be throttled.

## Ideas

- print IP stats, overlap with high/low/gossip
- env var to how often to divide everything by 2
- commit pre built binaries for east setup

## References

### bji's

- solana https://github.com/solana-labs/solana/compare/v1.17.31...bji:solana:v1.17.31_txingest
- solana-jito bji https://github.com/solana-labs/solana/compare/v1.17.31...bji:solana:v1.17.31-jito_txingest
- jito relayer https://github.com/jito-foundation/jito-relayer/compare/master...bji:jito-relayer:v0.1.12_txingest

### nasmithan's

When building solana-jito, you need to run `git submodule update --init --recursive`

- solana ipfee https://github.com/solana-labs/solana/compare/v1.17.31...nasmithan:solana:ip-fees-1.17.31
- solana-jito ipfee https://github.com/jito-foundation/jito-solana/compare/v1.17.31-jito...nasmithan:solana-jito:ip-fees-1.17.31

# What are shinobi's stats? Juicy? Aurora? Latitude?
