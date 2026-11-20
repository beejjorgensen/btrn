pub mod config;
pub mod model;
pub mod nntp;
pub mod persistence;
pub mod state;
pub mod thread;

use nntp::active::parse_active_groups;
use nntp::NntpSession;
use persistence::newsrc::{plan_unread_chunks, NewsrcState};
use std::fs;

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

    let config = config::Config {
        server: Some(host.clone()),
        use_tls,
        port: port.unwrap_or(if use_tls { 563 } else { 119 }),
        ..config::Config::default()
    };

    let mut session = match NntpSession::connect(
        config.server.as_deref().expect("server was set"),
        config.port,
        config.use_tls,
        config.connect_timeout,
        config.read_timeout,
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

    let capabilities = match session.discover_capabilities() {
        Ok(capabilities) => capabilities.clone(),
        Err(error) => {
            eprintln!("CAPABILITIES failed: {error}");
            std::process::exit(1);
        }
    };

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
                let newsrc = match fs::read_to_string(&config.newsrc_path) {
                    Ok(contents) => NewsrcState::parse(&contents),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        NewsrcState::parse("")
                    }
                    Err(error) => {
                        eprintln!("could not read {}: {error}", config.newsrc_path.display());
                        std::process::exit(1);
                    }
                };
                for issue in &newsrc.issues {
                    eprintln!("newsrc line {}: {}", issue.line_number, issue.message);
                }
                let read_ranges = newsrc
                    .state
                    .entry(&name)
                    .map_or(&[][..], |entry| entry.read_ranges.as_slice());
                let ranges = plan_unread_chunks(
                    group.first_article,
                    group.last_article,
                    read_ranges,
                    config.initial_overview_limit,
                    config.overview_chunk_size,
                );
                for range in ranges {
                    let overview = match session.list_overview(range) {
                        Ok(rows) => rows,
                        Err(error) => {
                            eprintln!("overview {}-{} failed: {error}", range.first, range.last);
                            std::process::exit(1);
                        }
                    };
                    for article in overview.articles {
                        let subject = article
                            .subject
                            .as_deref()
                            .map(String::from_utf8_lossy)
                            .unwrap_or_default();
                        println!("{} {}", article.article_number, subject);
                    }
                }
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
    if !capabilities.invalid_lines.is_empty() {
        eprintln!(
            "ignored malformed CAPABILITIES lines: {:?}",
            capabilities.invalid_lines
        );
    }
}

fn print_usage() {
    eprintln!("usage: btrn <host> [port] [--plaintext] [--group <name>]");
}
