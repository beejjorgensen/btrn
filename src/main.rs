pub mod config;
pub mod decoder;
pub mod group_overview;
pub mod model;
pub mod nntp;
pub mod persistence;
pub mod state;
pub mod terminal;
pub mod thread;

use model::article::RawArticle;
use nntp::active::parse_active_groups;
use nntp::NntpSession;
use persistence::newsrc::NewsrcState;
use state::SessionState;
use std::fs;
use std::io;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut host = None;
    let mut plaintext = false;
    let mut flat_overview = false;
    let mut port = None;
    let mut selected_group = None;
    let mut selected_article = None;
    let mut selected_thread_root = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--plaintext" => plaintext = true,
            "--flat" => flat_overview = true,
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
            "--article" => {
                if selected_article.is_some() {
                    eprintln!("--article may only be specified once");
                    print_usage();
                    std::process::exit(2);
                }
                selected_article = Some(
                    args.next()
                        .and_then(|value| value.parse::<u64>().ok())
                        .filter(|number| *number > 0)
                        .unwrap_or_else(|| {
                            eprintln!("--article requires a positive article number");
                            print_usage();
                            std::process::exit(2);
                        }),
                );
            }
            "--thread-root" => {
                if selected_thread_root.is_some() {
                    eprintln!("--thread-root may only be specified once");
                    print_usage();
                    std::process::exit(2);
                }
                selected_thread_root = Some(
                    args.next()
                        .and_then(|value| value.parse::<u64>().ok())
                        .filter(|number| *number > 0)
                        .unwrap_or_else(|| {
                            eprintln!("--thread-root requires a positive article number");
                            print_usage();
                            std::process::exit(2);
                        }),
                );
            }
            _ if host.is_none() => host = Some(argument),
            _ if port.is_none() => {
                port = Some(argument.parse::<u16>().unwrap_or_else(|_| {
                    eprintln!("unexpected argument: {argument}");
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

    let mut config = config::Config::load().unwrap_or_else(|error| {
        eprintln!("configuration failed: {error}");
        std::process::exit(2);
    });
    if let Some(host) = host {
        config.server = Some(host);
    }
    if plaintext {
        config.use_tls = false;
        if port.is_none() && config.port == 563 {
            config.port = 119;
        }
    }
    if let Some(port) = port {
        config.port = port;
    }
    let Some(host) = config.server.as_deref() else {
        eprintln!("{}", config::ConfigError::MissingServer);
        print_usage();
        std::process::exit(2);
    };

    let mut session = match NntpSession::connect(
        host,
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

    if let Some(credentials) = config.resolve_credentials().unwrap_or_else(|error| {
        eprintln!("credentials failed: {error}");
        std::process::exit(1);
    }) {
        if let Err(error) = session.authenticate(&credentials.username, &credentials.password) {
            eprintln!("authentication failed: {error}");
            std::process::exit(1);
        }
    }

    if selected_article.is_some() && selected_group.is_none() {
        eprintln!("--article requires --group");
        print_usage();
        std::process::exit(2);
    }
    if selected_thread_root.is_some() && selected_group.is_none() {
        eprintln!("--thread-root requires --group");
        print_usage();
        std::process::exit(2);
    }
    if selected_thread_root.is_some() && selected_article.is_some() {
        eprintln!("--thread-root cannot be used with --article");
        print_usage();
        std::process::exit(2);
    }
    if selected_thread_root.is_some() && flat_overview {
        eprintln!("--thread-root cannot be used with --flat");
        print_usage();
        std::process::exit(2);
    }
    if flat_overview && selected_group.is_none() {
        eprintln!("--flat requires --group");
        print_usage();
        std::process::exit(2);
    }
    if flat_overview && selected_article.is_some() {
        eprintln!("--flat cannot be used with --article");
        print_usage();
        std::process::exit(2);
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
                let mut state = SessionState {
                    newsrc: newsrc.state,
                    ..SessionState::default()
                };
                let article_to_present = if let Some(root_article_number) = selected_thread_root {
                    let loaded = match group_overview::load_group_threads(
                        &mut session,
                        &mut state,
                        &name,
                        group.first_article,
                        group.last_article,
                        config.initial_overview_limit,
                        config.overview_chunk_size,
                    ) {
                        Ok(loaded) => loaded,
                        Err(error) => {
                            eprintln!("overview for {name} failed: {error}");
                            std::process::exit(1);
                        }
                    };
                    if loaded.invalid_rows > 0 {
                        eprintln!("ignored {} malformed overview rows", loaded.invalid_rows);
                    }
                    if loaded.out_of_range_rows > 0 {
                        eprintln!(
                            "ignored {} overview rows outside requested ranges",
                            loaded.out_of_range_rows
                        );
                    }
                    match group_overview::first_unread_article_for_thread_root(
                        &state,
                        &name,
                        root_article_number,
                    ) {
                        Ok(article_number) => Some(article_number),
                        Err(error) => {
                            eprintln!("thread root {root_article_number} unavailable: {error}");
                            std::process::exit(1);
                        }
                    }
                } else {
                    selected_article
                };
                if let Some(article_number) = article_to_present {
                    let headers = match session.article_headers(article_number) {
                        Ok(headers) => headers,
                        Err(error) => {
                            eprintln!("HEAD {article_number} failed: {error}");
                            std::process::exit(1);
                        }
                    };
                    let body = match session.article_body(article_number) {
                        Ok(body) => body,
                        Err(error) => {
                            eprintln!("BODY {article_number} failed: {error}");
                            std::process::exit(1);
                        }
                    };
                    let mut lines = headers;
                    lines.push(Vec::new());
                    lines.extend(body);
                    let parsed = RawArticle::parse(&lines);
                    let decoded = decoder::decode_article(&parsed.article);
                    for issue in &decoded.issues {
                        eprintln!("article decoding issue: {issue}");
                    }
                    let mut stdout = io::stdout().lock();
                    if let Err(error) = terminal::article_view::present_article(
                        &mut stdout,
                        &mut state,
                        &name,
                        article_number,
                        &decoded,
                    ) {
                        eprintln!("{error}");
                        std::process::exit(1);
                    }
                    drop(stdout);
                    if let Err(error) = state.newsrc.write_atomic(&config.newsrc_path) {
                        eprintln!("could not write {}: {error}", config.newsrc_path.display());
                        std::process::exit(1);
                    }
                    return;
                }
                let loaded = match group_overview::load_group_threads(
                    &mut session,
                    &mut state,
                    &name,
                    group.first_article,
                    group.last_article,
                    config.initial_overview_limit,
                    config.overview_chunk_size,
                ) {
                    Ok(loaded) => loaded,
                    Err(error) => {
                        eprintln!("overview for {name} failed: {error}");
                        std::process::exit(1);
                    }
                };
                if loaded.invalid_rows > 0 {
                    eprintln!("ignored {} malformed overview rows", loaded.invalid_rows);
                }
                if loaded.out_of_range_rows > 0 {
                    eprintln!(
                        "ignored {} overview rows outside requested ranges",
                        loaded.out_of_range_rows
                    );
                }
                if flat_overview {
                    print!("{}", group_overview::format_flat_overview(&state, &name));
                } else {
                    print!("{}", group_overview::format_thread_summary(&loaded));
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
    eprintln!(
        "usage: btrn [host] [port] [--plaintext] [--group <name>] [--article <number>] [--thread-root <number>] [--flat]\n\
         configure server and optional credentials in ~/.config/btrn/config.toml"
    );
}
