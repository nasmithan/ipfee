use bincode::Options;
use clap::{App, Arg};
use crossbeam::channel::{unbounded, RecvTimeoutError};
use lru::LruCache;
use serde::{Deserialize, Serialize};
use serde_json::to_writer_pretty;
use solana_sdk::ipfee::IpFeeMsg;
use solana_sdk::signature::Signature;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::net::{IpAddr, Ipv4Addr, TcpListener};
use std::num::NonZeroUsize;
use std::process::Command;
use std::sync::Arc;

const BLOCK_AVG_FEE_BELOW: u64 = 60000;
const BLOCK_MIN_TXS: u64 = 100;
const BLOCK_ABOVE_DUPS_TX_RATIO: f64 = 10.0;
const PRINT_STATS_INTERVAL: u64 = 1000 * 60 * 10; // 10 minute
const CREATE_IP_BLOCKLIST_INTERVAL: u64 = 1000 * 60 * 2; // 2 minutes;

// const TX_COUNT_HALVING_INTERVAL: u64 = 1000 * 60 * 60 * 6; // 6 hours;

#[derive(Clone, Serialize, Deserialize)]
struct IpStats {
    tx_count: u64,
    avg_fee: u64,
    min_fee: u64,
    max_fee: u64,
    dup_count: u64,
    blocked: bool,
}

impl Default for IpStats {
    fn default() -> Self {
        IpStats { tx_count: 0, avg_fee: 0, min_fee: 0, max_fee: 0, dup_count: 0, blocked: false }
    }
}

struct State {
    // Map from tx to the IpAddr that submitted it
    ip_lookup: LruCache<Signature, IpAddr>,
    ip_avg_fees: LruCache<IpAddr, IpStats>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            ip_lookup: LruCache::new(NonZeroUsize::new(100_000).unwrap()),
            ip_avg_fees: LruCache::new(NonZeroUsize::new(100_000).unwrap()),
        }
    }
}

impl State {
    pub fn new(capacity: NonZeroUsize) -> Self {
        Self { ip_lookup: LruCache::new(capacity), ip_avg_fees: LruCache::new(capacity) }
    }

    pub fn usertx(
        &mut self,
        ip: IpAddr,
        signature: Signature,
    ) {
        let already_exists = self.ip_lookup.put(signature, ip);

        if already_exists.is_some() {
            // Count duplicate hashes per ip
            let entry = self.ip_avg_fees.get_or_insert_mut(ip, || IpStats::default());

            entry.dup_count += 1;
        }
    }

    pub fn fee(
        &mut self,
        signature: Signature,
        fee: u64,
    ) {
        if let Some(ip) = self.ip_lookup.get(&signature) {
            let entry = self.ip_avg_fees.get_or_insert_mut(*ip, || IpStats::default());

            let new_count = entry.tx_count + 1;
            // Calculate the new average fee for this IP, rounding
            // to the nearest whole number.
            entry.avg_fee = (entry.tx_count * entry.avg_fee + fee) / new_count;
            entry.tx_count = new_count;

            if entry.min_fee == 0 || fee < entry.min_fee {
                entry.min_fee = fee;
            } else if fee > entry.max_fee {
                entry.max_fee = fee;
            }
        }
    }

    // pub fn tx_count_halving(&mut self) {
    //     // Iterate over each key in the cache
    //     let keys: Vec<IpAddr> = self.ip_avg_fees.iter().map(|(ip, _)| *ip).collect();

    //     println!("Halving tx counts");
    //     for key in keys {
    //         if let Some(stats) = self.ip_avg_fees.get_mut(&key) {
    //             stats.tx_count /= 2; // Halve the tx count
    //             stats.dup_count /= 2; // Halve the tx count
    //         }
    //     }
    // }

    pub fn create_ip_blocklist(&mut self) {
        // Step 1: Extract and sort all records by txs descending
        let mut all_records: Vec<(IpAddr, IpStats)> = Vec::new();

        for (ip, stats) in self.ip_avg_fees.iter() {
            all_records.push((*ip, stats.clone())); // Clone each entry
        }

        if all_records.is_empty() {
            return;
        }

        let ips_to_block: Vec<IpAddr> = all_records
            .iter()
            .filter_map(|(ip, stats)| {
                if ((stats.avg_fee < BLOCK_AVG_FEE_BELOW && stats.tx_count > BLOCK_MIN_TXS)
                    || ((stats.dup_count as f64 / stats.tx_count as f64) > BLOCK_ABOVE_DUPS_TX_RATIO
                        && stats.avg_fee < 120000
                        && (stats.tx_count > 50 || stats.dup_count > 500)))
                    && !stats.blocked
                {
                    // Block if:
                    // 1. AvgFee < 60k lamports && TxCount > 100
                    // 2. (DupCount / TxCount) > 10, avg fee below 120k lamports, and above 50txs or >500 dups.
                    Some(*ip) // Dereference and copy the IP address
                } else {
                    None
                }
            })
            .collect();

        println!("Blocking {} IPs", ips_to_block.len());

        // TODO: automate this, it's required to run at least once
        // sudo ipset create custom-blocklist-ips hash:net

        // TODO: Use Command::new()
        for ip in ips_to_block {
            // Set IP as blocked
            if let Some(stats) = self.ip_avg_fees.get_mut(&ip) {
                stats.blocked = true;
            }

            let command_string = format!("sudo ipset add custom-blocklist-ips {}", ip);

            let output = Command::new("sh").arg("-c").arg(command_string).output().expect("failed to execute process");

            if output.status.success() {
                println!("Successfully blocked IP: {}", ip);
            } else {
                let err = String::from_utf8_lossy(&output.stderr);
                println!("Error blocking IP {}: {}", ip, err);
            }
        }
    }

    pub fn write_ip_stats_to_json(
        &self,
        file_path: &str,
    ) {
        let mut outputs: Vec<(&IpAddr, &IpStats)> = self.ip_avg_fees.iter().collect();
        outputs.sort_by(|a, b| b.1.tx_count.cmp(&a.1.tx_count)); // Sort by tx count desc

        let json_map: HashMap<String, IpStats> =
            outputs.into_iter().map(|(ip, stats)| (ip.to_string(), stats.clone())).collect();

        let file = File::create(file_path).expect("Unable to create file");
        let writer = BufWriter::new(file);

        to_writer_pretty(writer, &json_map).expect("Unable to write JSON");
    }

    pub fn read_ip_stats_from_json(
        &mut self,
        file_path: &str,
    ) {
        let file = match File::open(file_path) {
            Ok(file) => file,
            Err(_) => {
                println!("Error reading file");
                return;
            },
        };
        let reader = BufReader::new(file);

        let json_map: HashMap<String, IpStats> = match serde_json::from_reader(reader) {
            Ok(data) => data,
            Err(_) => {
                println!("Error parsing JSON");
                return;
            },
        };

        self.ip_avg_fees.clear();
        let mut count = 0;
        for (ip_str, stats) in json_map {
            if let Ok(ip) = ip_str.parse::<IpAddr>() {
                self.ip_avg_fees.put(ip, stats);
                count += 1;
            }
        }
        println!("Successfully loaded {} records from file", count);
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64
}

fn main() {
    let matches = App::new("ipfee")
        .arg(Arg::with_name("address").help("The IP address to listen on").required(true).index(1))
        .arg(Arg::with_name("port").help("The port to listen on").required(true).index(2))
        .arg(Arg::with_name("file_path").help("The file path to read/write state").required(true).index(3))
        .get_matches();

    let addr = matches.value_of("address").unwrap().parse::<Ipv4Addr>().unwrap_or_else(|e| {
        eprintln!("ERROR: Invalid listen address: {e}");
        std::process::exit(-1);
    });

    let port = matches.value_of("port").unwrap().parse::<u16>().unwrap_or_else(|e| {
        eprintln!("ERROR: Invalid listen port: {e}");
        std::process::exit(-1);
    });

    let file_path = matches.value_of("file_path").unwrap().parse::<String>().unwrap_or_else(|e| {
        eprintln!("ERROR: Invalid file path: {e}");
        std::process::exit(-1);
    });

    // Listen
    let tcp_listener = loop {
        match TcpListener::bind(std::net::SocketAddr::V4(std::net::SocketAddrV4::new(addr, port))) {
            Ok(tcp_listener) => break tcp_listener,
            Err(e) => {
                eprintln!("Failed bind because {e}, trying again in 1 second");
                std::thread::sleep(std::time::Duration::from_secs(1));
            },
        }
    };

    let (sender, receiver) = unbounded::<IpFeeMsg>();

    let sender = Arc::new(sender);

    // Spawn the listener
    std::thread::spawn(move || {
        loop {
            let mut tcp_stream = loop {
                match tcp_listener.accept() {
                    Ok((tcp_stream, _)) => break tcp_stream,
                    Err(e) => eprintln!("Failed accept because {e}"),
                }
            };

            {
                let sender = sender.clone();

                // Spawn a thread to handle this TCP stream.  Multiple streams are accepted at once, to allow e.g.
                // a JITO relayer and a validator to both connect.
                std::thread::spawn(move || {
                    let options = bincode::DefaultOptions::new();

                    loop {
                        match options.deserialize_from::<_, IpFeeMsg>(&mut tcp_stream) {
                            Ok(tx_ingest_msg) => sender.send(tx_ingest_msg).expect("crossbeam failed"),
                            Err(e) => {
                                eprintln!("Failed deserialize because {e}; closing connection");
                                tcp_stream.shutdown(std::net::Shutdown::Both).ok();
                                break;
                            },
                        }
                    }
                });
            }
        }
    });

    let mut state = State::new(NonZeroUsize::new(100_000).unwrap());

    // Load state from existing json file (if exists)
    state.read_ip_stats_from_json(&file_path);

    let mut last_write_timestamp = now_millis();
    // let mut last_tx_count_halving_timestamp: u64 = now_millis();
    let mut last_create_ip_blocklist_timestamp: u64 = now_millis();

    loop {
        // Receive with a timeout
        match receiver.recv_timeout(std::time::Duration::from_millis(100)) {
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => (),
            Ok(IpFeeMsg::UserTx { ip, signature }) => state.usertx(ip, signature),
            Ok(IpFeeMsg::Fee { signature, fee }) => state.fee(signature, fee),
        }

        let now = now_millis();

        // Check if it's time to write stats
        if now >= (last_write_timestamp + PRINT_STATS_INTERVAL) {
            state.write_ip_stats_to_json(&file_path);
            last_write_timestamp = now;
        }

        // Check if it's time to halve the transaction count
        // if now >= (last_tx_count_halving_timestamp + TX_COUNT_HALVING_INTERVAL) {
        //     state.tx_count_halving();
        //     last_tx_count_halving_timestamp = now;
        // }

        // Check if it's time to create ip blocklist
        if now >= (last_create_ip_blocklist_timestamp + CREATE_IP_BLOCKLIST_INTERVAL) {
            state.create_ip_blocklist();
            last_create_ip_blocklist_timestamp = now;
        }
    }
}
