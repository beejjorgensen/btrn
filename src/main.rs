mod nntp;

use nntp::{NntpSession, DEFAULT_CONNECT_TIMEOUT, DEFAULT_IO_TIMEOUT};

fn main() {
    let mut args = std::env::args().skip(1);
    let host = args.next();
    let port = args.next();
    let plaintext = args.next().as_deref() == Some("--plaintext");

    let Some(host) = host else {
        eprintln!("usage: btrn <host> [port] [--plaintext]");
        std::process::exit(2);
    };

    let use_tls = !plaintext;
    let port = match port {
        Some(value) => value.parse().unwrap_or_else(|_| {
            eprintln!("invalid port");
            std::process::exit(2);
        }),
        None => {
            if use_tls {
                563
            } else {
                119
            }
        }
    };

    let mut session = match NntpSession::connect(
        &host,
        port,
        use_tls,
        DEFAULT_CONNECT_TIMEOUT,
        DEFAULT_IO_TIMEOUT,
    ) {
        Ok(session) => session,
        Err(error) => {
            eprintln!("connection failed: {error}");
            std::process::exit(1);
        }
    };

    let mode_reader = match session.mode_reader() {
        Ok(response) => response,
        Err(error) => {
            eprintln!("MODE READER failed: {error}");
            std::process::exit(1);
        }
    };
    if !matches!(mode_reader.status, 200 | 201 | 500 | 501 | 502) {
        eprintln!("MODE READER failed: status {}", mode_reader.status);
        std::process::exit(1);
    }

    let capabilities = match session.capabilities() {
        Ok(response) => response,
        Err(error) => {
            eprintln!("CAPABILITIES failed: {error}");
            std::process::exit(1);
        }
    };
    if !(100..300).contains(&capabilities.status) {
        eprintln!("CAPABILITIES failed: status {}", capabilities.status);
        std::process::exit(1);
    }

    let active = match session.list_active() {
        Ok(response) => response,
        Err(error) => {
            eprintln!("LIST ACTIVE failed: {error}");
            std::process::exit(1);
        }
    };
    if active.status != 215 {
        eprintln!("LIST ACTIVE failed: status {}", active.status);
        std::process::exit(1);
    }

    println!("{}", active.data.len());
    for line in active.data {
        if let Some(group) = line.split(|byte| *byte == b' ').next() {
            if let Ok(group) = std::str::from_utf8(group) {
                println!("{group}");
            }
        }
    }
}
