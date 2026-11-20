mod nntp;

use nntp::active::parse_active_groups;
use nntp::{NntpSession, DEFAULT_CONNECT_TIMEOUT, DEFAULT_IO_TIMEOUT};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(host) = args.next() else {
        print_usage();
        std::process::exit(2);
    };

    let mut use_tls = true;
    let mut port = None;
    let mut selected_group = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--plaintext" => use_tls = false,
            "--group" => {
                if selected_group.is_some() {
                    eprintln!("--group may only be specified once");
                    print_usage();
                    std::process::exit(2);
                }
                selected_group = args.next();
                if selected_group.is_none() {
                    eprintln!("--group requires a group name");
                    print_usage();
                    std::process::exit(2);
                }
            }
            _ if port.is_none() => {
                port = Some(argument.parse::<u16>().unwrap_or_else(|_| {
                    eprintln!("invalid port");
                    print_usage();
                    std::process::exit(2);
                }));
            }
            _ => {
                eprintln!("unexpected argument: {argument}");
                print_usage();
                std::process::exit(2);
            }
        }
    }

    let port = port.unwrap_or(if use_tls { 563 } else { 119 });

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

    if let Some(name) = selected_group {
        match session.select_group(&name) {
            Ok(group) => {
                println!(
                    "{}: {} articles, range {}-{} (count consistent: {})",
                    group.name,
                    group.count,
                    group.first_article,
                    group.last_article,
                    group.count_consistent
                );
            }
            Err(error) => {
                eprintln!("GROUP {name} failed: {error}");
                std::process::exit(1);
            }
        }
        return;
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

    let groups = parse_active_groups(&active.data);
    println!("{}", groups.groups.len());
    for group in groups.groups {
        println!(
            "{} {} {} {:?}",
            group.name, group.first_article, group.last_article, group.posting_status
        );
    }
    for invalid in groups.invalid_lines {
        eprintln!(
            "ignoring malformed LIST ACTIVE row {}: {:?}",
            invalid.line_number, invalid.reason
        );
    }
}

fn print_usage() {
    eprintln!("usage: btrn <host> [port] [--plaintext] [--group <name>]");
}
