use bincode::Options;
use chrono::Utc;
use clap::{App, Arg};
use crossbeam::channel::{unbounded, RecvTimeoutError};
use lru::LruCache;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::to_writer_pretty;
use solana_sdk::ipfee::IpFeeMsg;
use solana_sdk::signature::Signature;
use std::collections::HashMap;
use std::error::Error;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::net::{IpAddr, Ipv4Addr, TcpListener};
use std::num::NonZeroUsize;
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

const BLOCK_AVG_FEE_BELOW: u64 = 30000;
const BLOCK_MIN_TXS: u64 = 300;

const GET_EPOCH_INFO_INTERVAL: u64 = 1000 * 5; // 5 seconds
const WRITE_STATS_INTERVAL: u64 = 1000 * 60 * 1; // 1 minute
const CREATE_IP_BLOCKLIST_INTERVAL: u64 = 1000 * 60 * 2; // 2 minutes

// How close to leader slots to be considered near leader slots
const NEAR_LEADER_SLOTS: u64 = 150;

const RPC_URL: &str = "http://127.0.0.1:8899";

#[derive(Clone, Serialize, Deserialize)]
struct IpStats {
    tx_count: u64,
    tx_count_under_50k: u64,
    dup_count: u64,
    avg_fee: u64,
    min_fee: u64,
    max_fee: u64,
    avg_cu_limit: u64,
    avg_cu_used: u64,
    leader_tx_count: u64,
    leader_tx_count_under_50k: u64,
    leader_dup_count: u64,
    leader_avg_fee: u64,
    leader_min_fee: u64,
    leader_max_fee: u64,
    leader_avg_cu_limit: u64,
    leader_avg_cu_used: u64,
    blocked: bool,
}

impl Default for IpStats {
    fn default() -> Self {
        IpStats {
            tx_count: 0,
            tx_count_under_50k: 0,
            avg_fee: 0,
            min_fee: 0,
            max_fee: 0,
            dup_count: 0,
            avg_cu_limit: 0,
            avg_cu_used: 0,
            blocked: false,
            leader_tx_count: 0,
            leader_tx_count_under_50k: 0,
            leader_avg_fee: 0,
            leader_min_fee: 0,
            leader_max_fee: 0,
            leader_dup_count: 0,
            leader_avg_cu_limit: 0,
            leader_avg_cu_used: 0,
        }
    }
}

struct State {
    identity: String,
    current_slot: u64,
    current_epoch: u64,
    current_epoch_start_slot: u64,
    near_leader_slots: bool,
    leader_schedule: Vec<u64>,
    ip_lookup: LruCache<Signature, IpAddr>,
    ip_avg_fees: LruCache<IpAddr, IpStats>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            identity: String::new(),
            current_slot: 0,
            current_epoch: 0,
            current_epoch_start_slot: 0,
            near_leader_slots: false,
            leader_schedule: vec![],
            ip_lookup: LruCache::new(NonZeroUsize::new(100_000).unwrap()),
            ip_avg_fees: LruCache::new(NonZeroUsize::new(100_000).unwrap()),
        }
    }
}

impl State {
    pub fn new(
        capacity: NonZeroUsize,
        identity: String,
    ) -> Self {
        Self {
            ip_lookup: LruCache::new(capacity),
            ip_avg_fees: LruCache::new(capacity),
            identity,
            current_slot: 0,
            current_epoch: 0,
            current_epoch_start_slot: 0,
            near_leader_slots: false,
            leader_schedule: vec![],
        }
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
            if self.near_leader_slots {
                entry.leader_dup_count += 1;
            }
        }
    }

    pub fn fee(
        &mut self,
        signature: Signature,
        cu_limit: u64,
        cu_used: u64,
        fee: u64,
    ) {
        if let Some(ip) = self.ip_lookup.get(&signature) {
            let entry = self.ip_avg_fees.get_or_insert_mut(*ip, || IpStats::default());

            let new_count = entry.tx_count + 1;
            // Calculate the new average fee for this IP, rounding
            // to the nearest whole number.
            entry.avg_fee = (entry.tx_count * entry.avg_fee + fee) / new_count;
            entry.avg_cu_limit = (entry.tx_count * entry.avg_cu_limit + fee) / new_count;
            entry.avg_cu_used = (entry.tx_count * entry.avg_cu_used + fee) / new_count;
            entry.tx_count = new_count;

            if entry.min_fee == 0 || fee < entry.min_fee {
                entry.min_fee = fee;
            } else if fee > entry.max_fee {
                entry.max_fee = fee;
            }

            if fee <= 50000 {
                entry.tx_count_under_50k += 1;
            }

            if self.near_leader_slots {
                let leader_new_count = entry.leader_tx_count + 1;
                // Calculate the new average fee for this IP, rounding
                // to the nearest whole number.
                entry.leader_avg_fee = (entry.leader_tx_count * entry.leader_avg_fee + fee) / leader_new_count;
                entry.leader_avg_cu_limit =
                    (entry.leader_tx_count * entry.leader_avg_cu_limit + cu_limit) / leader_new_count;
                entry.leader_avg_cu_used =
                    (entry.leader_tx_count * entry.leader_avg_cu_used + cu_used) / leader_new_count;
                entry.leader_tx_count = leader_new_count;

                if entry.leader_min_fee == 0 || fee < entry.leader_min_fee {
                    entry.leader_min_fee = fee;
                } else if fee > entry.leader_max_fee {
                    entry.leader_max_fee = fee;
                }

                if fee <= 50000 {
                    entry.leader_tx_count_under_50k += 1;
                }
            }
        }
    }

    pub fn create_ip_blocklist(&mut self) {
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
                if ((stats.avg_fee < BLOCK_AVG_FEE_BELOW && stats.tx_count > BLOCK_MIN_TXS) // fee < 30k, count > 500
                    || (stats.tx_count < 1 && stats.dup_count > 100) // >100 duplicates, 0 first time sender
                    || (stats.tx_count > 3 && stats.avg_fee == 9544)) // Block 9544 fee sender
                    && !stats.blocked
                {
                    // Interesting stats from 4hr data collection without firewall
                    // - 11% of top 100 IPs only send low fee txs (<10k lamports). 1 of these low IP senders is in gossip! This number will likely grow)
                    // - 12.4% of the top 500 IPs only send >10k lamports txs, no low fee txs. Half not in gossip.
                    // - 34.4% of the top 500 IPs sent a tx fee average >100k lamports (1/5th of these high fee payers aren't in gossip!)
                    // - 80% of the top 100 tx senders send txs >150 slots away from leader slots, while only 11% of the next 400 big tx senders.
                    // - A single entity used 279 different IPs in a 4 hour period to send many unique low quality <10k lamport txs.

                    Some(*ip) // Dereference and copy the IP address
                } else {
                    None
                }
            })
            .collect();

        if ips_to_block.len() > 0 {
            // Print the number of IPs to unblock with timestamp
            let current_time = Utc::now();
            println!("[{}] Blocking {} IPs", current_time.format("%Y-%m-%d %H:%M:%S"), ips_to_block.len());
        }

        // TODO: automate this, it's required to run at least once
        // sudo ipset create custom-blocklist-ips hash:net

        // TODO: Use Command::new()
        for ip in ips_to_block {
            if let Some(stats) = self.ip_avg_fees.get_mut(&ip) {
                stats.blocked = true;
            }

            let command_string = format!("sudo ipset add custom-blocklist-ips {}", ip);

            let output = Command::new("sh").arg("-c").arg(command_string).output().expect("failed to execute process");

            if output.status.success() {
                let current_time = Utc::now();
                println!("[{}] Successfully blocked IP: {}", current_time.format("%Y-%m-%d %H:%M:%S"), ip);
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
        let json_map: HashMap<String, IpStats> =
            self.ip_avg_fees.iter().map(|(ip, stats)| (ip.to_string(), stats.clone())).collect();
        let file = File::create(file_path).expect("Unable to create file");
        let writer = BufWriter::new(file);

        to_writer_pretty(writer, &json_map).expect("Unable to write JSON");
    }

    pub fn read_ip_stats_from_json(
        &mut self,
        file_path: &str,
    ) {
        let start = Instant::now();
        let file = match File::open(file_path) {
            Ok(file) => file,
            Err(_) => {
                println!("Error reading file on startup. Ignore if first time running.");
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
        let duration = start.elapsed(); // Calculate elapsed time
        println!("Successfully loaded {} records from file in {:?}", count, duration);
    }

    fn get_epoch_info(&mut self) {
        let client = Client::new();
        let request_body = r#"{"jsonrpc":"2.0","id":1,"method":"getEpochInfo"}"#;

        // Handle the result of `send` using `match`
        let response = match client.post(RPC_URL).header("Content-Type", "application/json").body(request_body).send() {
            Ok(response) => response,
            Err(e) => {
                eprintln!("Failed to send getEpochInfo request: {e}");
                return; // Exit the function early
            },
        };

        // Handle the result of `json` using `match`
        let epoch_info_response: EpochInfoResponse = match response.json() {
            Ok(parsed_response) => parsed_response,
            Err(e) => {
                eprintln!("Failed to parse getEpochInfo JSON response: {e}");
                return; // Exit the function early
            },
        };

        self.current_slot = epoch_info_response.result.absolute_slot;
        if self.current_epoch != epoch_info_response.result.epoch {
            // If new epoch or on startup
            self.current_epoch = epoch_info_response.result.epoch;
            self.current_epoch_start_slot = self.current_slot - epoch_info_response.result.slot_index;
            self.get_leader_schedule();
            println!("Current Identity: {}, Slot: {}, Epoch: {}", self.identity, self.current_slot, self.current_epoch);
        }

        let mut slots_since_leader = None;
        let mut slots_until_next_leader = None;

        for &slot in &self.leader_schedule {
            if slot <= self.current_slot {
                slots_since_leader = Some(self.current_slot - slot);
            } else if slots_until_next_leader.is_none() && slot >= self.current_slot {
                // First slot greater than current_slot
                slots_until_next_leader = Some(slot - self.current_slot);
            }
            if slots_since_leader.is_some() && slots_until_next_leader.is_some() {
                break;
            }
        }

        if slots_since_leader.unwrap_or(u64::MIN) < NEAR_LEADER_SLOTS
            || slots_until_next_leader.unwrap_or(u64::MIN) < NEAR_LEADER_SLOTS
        {
            self.near_leader_slots = true;
        } else {
            self.near_leader_slots = false;
        }

        // println!("Slots since leader: {:?}, Slots until leader: {:?}", slots_since_leader, slots_until_next_leader);
    }

    fn get_leader_schedule(&mut self) {
        println!("Getting Leader Schedule");
        let client = Client::new();
        let request_body = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"getLeaderSchedule","params":[null,{{"identity":"{}"}}]}}"#,
            self.identity
        );

        // Handle the result of `send` using `match`
        let response = match client.post(RPC_URL).header("Content-Type", "application/json").body(request_body).send() {
            Ok(response) => response,
            Err(e) => {
                eprintln!("Failed to send getEpochInfo request: {e}");
                return; // Exit the function early
            },
        };

        // Handle the result of `json` using `match`
        let leader_schedule_response: LeaderScheduleResponse = match response.json() {
            Ok(parsed_response) => parsed_response,
            Err(e) => {
                eprintln!("Failed to parse getEpochInfo JSON response: {e}");
                return; // Exit the function early
            },
        };

        if let Some(schedule) = leader_schedule_response.result.get(&self.identity) {
            // Assign the retrieved vector to `self.leader_schedule`
            self.leader_schedule = schedule.iter().map(|&slot| slot + self.current_epoch_start_slot).collect();
        } else {
            eprintln!("Identity not found in leader schedule");
        }
    }
}

// Struct for deserializing JSON response
#[derive(Deserialize)]
struct EpochInfoResponse {
    result: EpochInfoResult,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EpochInfoResult {
    absolute_slot: u64,
    slot_index: u64,
    epoch: u64,
}

// Struct for deserializing JSON response
#[derive(Deserialize)]
struct LeaderScheduleResponse {
    result: HashMap<String, Vec<u64>>,
}

fn now_millis() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64
}

// Struct for deserializing JSON response
#[derive(Deserialize)]
struct IdentityResponse {
    result: IdentityResult,
}

#[derive(Deserialize)]
struct IdentityResult {
    identity: String,
}

// Modified synchronous versions of async functions
fn get_validator_identity() -> Result<String, Box<dyn Error>> {
    let client = Client::new();
    let request_body = r#"{"jsonrpc":"2.0","id":1,"method":"getIdentity"}"#;

    let response = client.post(RPC_URL).header("Content-Type", "application/json").body(request_body).send()?;

    let identity_response: IdentityResponse = response.json()?;
    Ok(identity_response.result.identity)
}

fn main() {
    let matches = App::new("ipfee")
        .arg(Arg::with_name("address").help("The IP address to listen on").required(true).index(1))
        .arg(Arg::with_name("port").help("The port to listen on").required(true).index(2))
        .arg(Arg::with_name("file_path").help("The .json file path to read/write state").required(true).index(3))
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

    // Initialize identity synchronously
    let identity = get_validator_identity().unwrap_or_else(|e| {
        eprintln!("Failed to fetch validator identity: {e}");
        std::process::exit(-1);
    });

    let mut state = State::new(NonZeroUsize::new(100_000).unwrap(), identity);

    // Load state from existing json file (if exists)
    state.read_ip_stats_from_json(&file_path);

    let mut last_write_timestamp = now_millis();
    let mut last_get_epoch_info_timestamp = now_millis();
    let mut last_create_ip_blocklist_timestamp: u64 = now_millis();

    loop {
        // Receive with a timeout
        match receiver.recv_timeout(std::time::Duration::from_millis(100)) {
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => (),
            Ok(IpFeeMsg::UserTx { ip, signature }) => state.usertx(ip, signature),
            Ok(IpFeeMsg::Fee { signature, cu_limit, cu_used, fee }) => state.fee(signature, cu_limit, cu_used, fee),
        }

        let now = now_millis();

        // Check if it's time to write stats
        if now >= (last_write_timestamp + WRITE_STATS_INTERVAL) {
            state.write_ip_stats_to_json(&file_path);
            last_write_timestamp = now;
        }

        if now >= (last_get_epoch_info_timestamp + GET_EPOCH_INFO_INTERVAL) {
            state.get_epoch_info();
            last_get_epoch_info_timestamp = now;
        }

        // Check if it's time to create ip blocklist
        if now >= (last_create_ip_blocklist_timestamp + CREATE_IP_BLOCKLIST_INTERVAL) {
            state.create_ip_blocklist();
            last_create_ip_blocklist_timestamp = now;
            state.write_ip_stats_to_json(&file_path);
            last_write_timestamp = now;
        }
    }
}
