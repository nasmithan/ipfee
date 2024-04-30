use bincode::Options;
use crossbeam::channel::{unbounded, RecvTimeoutError};
use lru::LruCache;
use solana_sdk::ipfee::IpFeeMsg;
use solana_sdk::signature::Signature;
use std::net::{IpAddr, Ipv4Addr, TcpListener};
use std::num::NonZeroUsize;
use std::sync::Arc;

struct State {
    // Map from tx to the IpAddr that submitted it
    ip_lookup: LruCache<Signature, IpAddr>,
    ip_avg_fees: LruCache<IpAddr, (u64, u64)>,
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
        self.ip_lookup.put(signature, ip);
    }

    pub fn fee(
        &mut self,
        signature: Signature,
        fee: u64,
    ) {
        if let Some(ip) = self.ip_lookup.get(&signature) {
            let entry = self.ip_avg_fees.get_or_insert_mut(*ip, || (0, 0));

            // println!("{} {}", entry.0, entry.1);
            let new_count = entry.0 + 1;
            // Calculate the new average fee for this IP, rounding
            // to the nearest whole number.
            entry.1 = (entry.0 * entry.1 + fee) / new_count;
            entry.0 = new_count;
        }
    }

    // pub fn dump_ip_avg_fees(&self) {
    //     // let mut map = HashMap::new();
    //     let now = now_millis();
    //     println!("data dump");
    //     for (ip, fees) in self.ip_avg_fees.iter() {
    //         println!("{} {} {} {}", now, ip, fees.0, fees.1);
    //     }
    // }
    pub fn dump_ip_avg_fees(&self) {
        let mut outputs: Vec<(u64, String)> = Vec::new();

        for (ip, fees) in self.ip_avg_fees.iter() {
            outputs.push((fees.0, format!("{}\t{}\t{}", ip, fees.0, fees.1)));
        }

        outputs.sort_by(|a, b| b.0.cmp(&a.0)); // Sort by tx count desc

        println!("IP\t\tTxCount\tAvgFees");
        for (_, output) in outputs {
            println!("{}", output);
        }
        println!("");
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64
}

fn main() {
    // Listen on a specific port; for the time being just dump events out to stdout

    let input_args = std::env::args().skip(1).collect::<Vec<String>>();

    if input_args.len() != 2 {
        eprintln!("ERROR: Incorrect number of arguments: must be: <LISTEN_ADDRESS> <LISTEN_PORT>");
        std::process::exit(-1);
    }

    let addr = input_args[0]
        .parse::<Ipv4Addr>()
        .unwrap_or_else(|e| error_exit(format!("ERROR: Invalid listen address {}: {e}", input_args[0])));

    let port = input_args[1]
        .parse::<u16>()
        .unwrap_or_else(|e| error_exit(format!("ERROR: Invalid listen port {}: {e}", input_args[1])));

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
    let mut last_log_timestamp = 0;

    loop {
        // Receive with a timeout
        match receiver.recv_timeout(std::time::Duration::from_millis(100)) {
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => (),
            Ok(IpFeeMsg::UserTx { ip, signature }) => state.usertx(ip, signature),
            Ok(IpFeeMsg::Fee { signature, fee }) => state.fee(signature, fee),
        }

        let now = now_millis();
        if now < (last_log_timestamp + 10000) {
            continue;
        }

        state.dump_ip_avg_fees();

        last_log_timestamp = now;
    }
}

fn error_exit(msg: String) -> ! {
    eprintln!("{msg}");
    std::process::exit(-1);
}
