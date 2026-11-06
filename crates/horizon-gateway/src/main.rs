//! `horizon-gateway serve`: the public frontend (settings from `GATEWAY_*`,
//! docs/public-address.md). `horizon-gateway expose <gateway> <name>`: put
//! a local server behind it. Prints counts every five minutes, nothing
//! about players.
use horizon_gateway::{
    Config, ErrorCode, Gateway,
    agent::{self, ExposeConfig, ExposeError},
};
use std::{collections::HashSet, net::TcpListener, process::exit, time::Duration};

const USAGE: &str = "usage:
  horizon-gateway serve                       (GATEWAY_DOMAIN=mc.example.net, see docs/public-address.md)
  horizon-gateway expose <gateway:port> <name> [--to 127.0.0.1:25565] [--key KEY] [--claim CLAIM]";

fn var<T: std::str::FromStr>(name: &str) -> Option<T> {
    std::env::var(name).ok()?.trim().parse().ok()
}

fn listen(bind: Option<&str>, port: u16) -> TcpListener {
    match bind {
        Some(address) => TcpListener::bind((address, port)),
        None => TcpListener::bind(("::", port)).or_else(|_| TcpListener::bind(("0.0.0.0", port))),
    }
    .unwrap_or_else(|e| {
        eprintln!("horizon-gateway: can't listen on port {port}: {e}");
        exit(1);
    })
}

fn serve() {
    let Some(domain) = std::env::var("GATEWAY_DOMAIN")
        .ok()
        .filter(|d| !d.trim().is_empty())
    else {
        eprintln!(
            "horizon-gateway: set GATEWAY_DOMAIN to the wildcard domain players' addresses end in"
        );
        exit(2);
    };
    let mut config = Config::new(&domain);
    let minecraft_port = var("GATEWAY_MINECRAFT_PORT").unwrap_or(25565);
    let control_port = var("GATEWAY_CONTROL_PORT").unwrap_or(7001);
    config.public_port = var("GATEWAY_PUBLIC_PORT").unwrap_or(minecraft_port);
    if let Some(v) = var("GATEWAY_MAX_HOSTS") {
        config.max_hosts = v;
    }
    if let Some(v) = var("GATEWAY_MAX_BRIDGES") {
        config.max_bridges = v;
    }
    if let Some(v) = var::<f64>("GATEWAY_SESSION_KBPS") {
        config.session_rate = v * 1024.0;
    }
    if let Some(v) = var::<f64>("GATEWAY_TOTAL_MBPS") {
        config.total_rate = v * 1_000_000.0 / 8.0;
    }
    if let Some(v) = var::<f64>("GATEWAY_NAME_KEPT_HOURS") {
        config.name_kept = Duration::from_secs_f64(v * 3600.0);
    }
    if let Some(v) = var::<u8>("GATEWAY_PER_ADDRESS_LIMITS") {
        config.per_address = v != 0;
    }
    if let Ok(path) = std::env::var("GATEWAY_HOST_KEYS") {
        let keys: HashSet<String> = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| {
                eprintln!("horizon-gateway: can't read GATEWAY_HOST_KEYS {path}: {e}");
                exit(1);
            })
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_owned)
            .collect();
        config.host_keys = Some(keys);
    }
    let bind = std::env::var("GATEWAY_BIND")
        .ok()
        .filter(|b| !b.trim().is_empty());
    let minecraft = listen(bind.as_deref(), minecraft_port);
    let control = listen(bind.as_deref(), control_port);
    println!(
        "horizon-gateway {} serving *.{} on port {minecraft_port}, hosts on port {control_port} ({}, hosts {}, bridges {}, {} KiB/s per player)",
        env!("CARGO_PKG_VERSION"),
        config.domain,
        if config.host_keys.is_some() {
            "keys required"
        } else {
            "open registration"
        },
        config.max_hosts,
        config.max_bridges,
        (config.session_rate / 1024.0) as u64,
    );
    let gateway = Gateway::new(config);
    let g = gateway.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(300));
            println!("{}", g.counts().json());
        }
    });
    gateway.serve(minecraft, control);
}

fn expose(args: &[String]) {
    let [gateway, name, rest @ ..] = args else {
        eprintln!("{USAGE}");
        exit(2);
    };
    let mut config = ExposeConfig {
        gateway: gateway.clone(),
        name: name.clone(),
        to: "127.0.0.1:25565".into(),
        claim: None,
        key: std::env::var("HORIZON_GATEWAY_KEY").ok(),
    };
    let mut it = rest.iter();
    while let Some(flag) = it.next() {
        let value = it.next().cloned();
        match (flag.as_str(), value) {
            ("--to", Some(v)) => config.to = v,
            ("--key", Some(v)) => config.key = Some(v),
            ("--claim", Some(v)) => config.claim = Some(v),
            _ => {
                eprintln!("{USAGE}");
                exit(2);
            }
        }
    }
    // Stay registered: reconnect with the claim when the gateway drops us.
    let mut backoff = Duration::from_secs(1);
    loop {
        match agent::expose(config.clone()) {
            Ok(exposed) => {
                if config.claim.as_deref() != Some(exposed.claim.as_str()) {
                    println!("Players join: {}", exposed.address);
                    println!(
                        "To keep this name after a restart: --claim {}",
                        exposed.claim
                    );
                }
                config.claim = Some(exposed.claim.clone());
                backoff = Duration::from_secs(1);
                exposed.wait();
                eprintln!("horizon-gateway: lost the gateway, reconnecting");
            }
            Err(ExposeError::Refused(
                code @ (ErrorCode::BadName | ErrorCode::NameTaken | ErrorCode::Unauthorized),
            )) => {
                eprintln!("horizon-gateway: {}", ExposeError::Refused(code));
                exit(1);
            }
            Err(e) => eprintln!("horizon-gateway: {e}; retrying in {}s", backoff.as_secs()),
        }
        std::thread::sleep(backoff);
        backoff = (backoff * 2).min(Duration::from_secs(60));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("serve") => serve(),
        Some("expose") => expose(&args[1..]),
        _ => {
            eprintln!("{USAGE}");
            exit(2);
        }
    }
}
