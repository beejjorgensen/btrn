mod cli;
mod config;
mod decoder;
mod group_loader;
mod model;
mod nntp;
mod persistence;
mod posting;
mod state;
mod terminal;
mod thread;

use group_loader::LoadedGroupThreads;
use model::article::RawArticle;
use nntp::active::{parse_active_groups, ActiveGroupList};
use nntp::{GroupSelection, NntpSession};
use persistence::newsrc::NewsrcState;
use posting::PostingSettings;
use state::SessionState;
use std::fmt::Display;
use std::fs;
use std::io;
use terminal::session::TerminalSession;

fn main() {
    let arguments = cli::Arguments::parse(std::env::args().skip(1))
        .unwrap_or_else(|message| usage_error(message));

    let mut config = config::Config::load().unwrap_or_else(|error| {
        eprintln!("configuration failed: {error}");
        std::process::exit(2);
    });
    if let Some(host) = arguments.host.clone() {
        config.server = Some(host);
    }
    if arguments.plaintext {
        config.use_tls = false;
    }
    if let Some(port) = arguments.port {
        config.port = Some(port);
    }
    let posting_settings = PostingSettings {
        identity: posting::PostingIdentity {
            display_name: config.display_name.clone(),
            display_email: config.display_email.clone(),
            organization: config.organization.clone(),
        },
        editor: config.editor.clone(),
    };
    let Some(host) = config.server.as_deref() else {
        usage_error(config::ConfigError::MissingServer);
    };

    let mut session = NntpSession::connect(
        host,
        config.port(),
        config.use_tls,
        config.connect_timeout,
        config.read_timeout,
    )
    .unwrap_or_else(|error| fail(format_args!("connection failed: {error}")));

    let mode_reader = session
        .mode_reader()
        .unwrap_or_else(|error| fail(format_args!("MODE READER failed: {error}")));
    if !matches!(mode_reader.status, 200 | 201 | 500 | 501 | 502) {
        fail(format_args!(
            "MODE READER failed: status {}",
            mode_reader.status
        ));
    }

    let capabilities = match session.discover_capabilities() {
        Ok(capabilities) => capabilities.clone(),
        Err(error) => fail(format_args!("CAPABILITIES failed: {error}")),
    };

    if let Some(credentials) = config
        .resolve_credentials()
        .unwrap_or_else(|error| fail(format_args!("credentials failed: {error}")))
    {
        if let Err(error) = session.authenticate(&credentials.username, &credentials.password) {
            fail(format_args!("authentication failed: {error}"));
        }
    }

    if let Err(message) = arguments.check_combinations() {
        usage_error(message);
    }

    if let Some(name) = &arguments.group {
        run_group(&mut session, &config, &posting_settings, &arguments, name);
        return;
    }

    let active = session
        .list_active()
        .unwrap_or_else(|error| fail(format_args!("LIST ACTIVE failed: {error}")));
    if active.status != 215 {
        fail(format_args!("LIST ACTIVE failed: status {}", active.status));
    }

    let groups = parse_active_groups(&active.data);
    for invalid in &groups.invalid_lines {
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
    if arguments.interactive {
        run_group_selector(&mut session, &config, &posting_settings, &groups);
        return;
    }

    println!("{}", groups.groups.len());
    for group in groups.groups {
        println!(
            "{} {} {} {:?}",
            group.name, group.first_article, group.last_article, group.posting_status
        );
    }
}

/// The `--group` modes: the interactive reader, one article, or an overview
/// listing.
fn run_group(
    session: &mut NntpSession,
    config: &config::Config,
    posting_settings: &PostingSettings,
    arguments: &cli::Arguments,
    name: &str,
) {
    let group = session
        .select_group(name)
        .unwrap_or_else(|error| fail(format_args!("GROUP {name} failed: {error}")));
    println!(
        "{}: {} articles, range {}-{} (count consistent: {})",
        group.name, group.count, group.first_article, group.last_article, group.count_consistent
    );
    let mut state = load_session_state(config);
    if arguments.interactive {
        let loaded = load_threads(session, &mut state, &group, config, name);
        let reader = terminal::reader::ReaderState::new(loaded.threads, group.last_article);
        let reader_result =
            terminal::reader::run_interactive(session, &mut state, reader, posting_settings);
        save_after_reader(&state, config, reader_result).unwrap_or_else(|message| fail(message));
        return;
    }
    let article_to_present = if let Some(root_article_number) = arguments.thread_root {
        let loaded = load_threads(session, &mut state, &group, config, name);
        report_ignored_rows(&loaded);
        Some(
            state::article_navigation::first_unread_article_for_thread_root(
                &state,
                name,
                root_article_number,
            )
            .unwrap_or_else(|error| {
                fail(format_args!(
                    "thread root {root_article_number} unavailable: {error}"
                ))
            }),
        )
    } else {
        arguments.article
    };
    if let Some(article_number) = article_to_present {
        present_one_article(session, &mut state, config, &group, name, article_number);
        return;
    }
    let loaded = load_threads(session, &mut state, &group, config, name);
    report_ignored_rows(&loaded);
    if arguments.flat_overview {
        print!("{}", cli::format_flat_overview(&state, name));
    } else {
        print!("{}", cli::format_thread_summary(&loaded));
    }
}

/// Fetches, decodes, and prints one article, then records it as read.
fn present_one_article(
    session: &mut NntpSession,
    state: &mut SessionState,
    config: &config::Config,
    group: &GroupSelection,
    name: &str,
    article_number: u64,
) {
    let headers = session
        .article_headers(article_number)
        .unwrap_or_else(|error| fail(format_args!("HEAD {article_number} failed: {error}")));
    let body = session
        .article_body(article_number)
        .unwrap_or_else(|error| fail(format_args!("BODY {article_number} failed: {error}")));
    let mut lines = headers;
    lines.push(Vec::new());
    lines.extend(body);
    let parsed = RawArticle::parse(&lines);
    let decoded = decoder::decode_article(&parsed.article);
    for issue in &decoded.issues {
        eprintln!("article decoding issue: {issue}");
    }
    let width = crossterm::terminal::size().map_or(80, |(width, _)| width);
    let more_unread = state.groups.get(name).map_or(0, |group_state| {
        group_state
            .articles
            .keys()
            .filter(|number| **number != article_number && !state.is_read(name, **number))
            .count()
    });
    let context = terminal::article_view::ArticleViewContext {
        group: name,
        article_number,
        more_unread,
        group_last_article: group.last_article,
        header_mode: Default::default(),
    };
    let layout = terminal::article_view::ArticleLayout::new(&context, &decoded, width);
    let mut stdout = io::stdout().lock();
    if let Err(error) = terminal::article_view::present_article(
        &mut stdout,
        state,
        &context,
        &layout,
        width,
        layout.full_height(),
    ) {
        fail(error);
    }
    drop(stdout);
    save_newsrc(state, config).unwrap_or_else(|message| fail(message));
}

/// The interactive newsgroup selector, reading each chosen group in turn.
fn run_group_selector(
    session: &mut NntpSession,
    config: &config::Config,
    posting_settings: &PostingSettings,
    groups: &ActiveGroupList,
) {
    let mut state = load_session_state(config);
    let mut terminal_session = TerminalSession::enter()
        .unwrap_or_else(|error| fail(format_args!("could not initialize terminal: {error}")));
    loop {
        let selected_groups = match terminal::group_selector::run_interactive_in_session(
            groups,
            &state.newsrc,
            &mut terminal_session,
        ) {
            Ok(Some(selected_groups)) => selected_groups,
            Ok(None) => return,
            Err(error) => fail_in_terminal(
                terminal_session,
                format_args!("newsgroup selector failed: {error}"),
            ),
        };
        for name in selected_groups {
            if let Err(error) =
                terminal::group_selector::render_loading_threads(&mut terminal_session.output, 0)
            {
                fail_in_terminal(
                    terminal_session,
                    format_args!("could not render loading screen: {error}"),
                );
            }
            let group = match session.select_group(&name) {
                Ok(group) => group,
                Err(error) => fail_in_terminal(
                    terminal_session,
                    format_args!("GROUP {name} failed: {error}"),
                ),
            };
            let loaded = match group_loader::load_group_threads_with_progress(
                session,
                &mut state,
                &group,
                overview_limits(config),
                |completed, total| {
                    let percent = if total == 0 {
                        100
                    } else {
                        completed.saturating_mul(100).div_ceil(total)
                    };
                    terminal::group_selector::render_loading_threads(
                        &mut terminal_session.output,
                        percent,
                    )
                },
            ) {
                Ok(loaded) => loaded,
                Err(error) => fail_in_terminal(
                    terminal_session,
                    format_args!("overview for {name} failed: {error}"),
                ),
            };
            report_ignored_rows(&loaded);
            let reader = terminal::reader::ReaderState::new(loaded.threads, group.last_article);
            let reader_result = terminal::reader::run_interactive_in_session(
                session,
                &mut state,
                reader,
                &mut terminal_session,
                posting_settings,
            );
            let reader_exit = match save_after_reader(&state, config, reader_result) {
                Ok(reader_exit) => reader_exit,
                Err(message) => fail_in_terminal(terminal_session, message),
            };
            if reader_exit == terminal::reader::ReaderExit::QuitApplication {
                return;
            }
        }
    }
}

/// Session state starting from the saved newsrc, reporting any lines that
/// could not be parsed.
fn load_session_state(config: &config::Config) -> SessionState {
    let newsrc = match fs::read_to_string(&config.newsrc_path) {
        Ok(contents) => NewsrcState::parse(&contents),
        Err(error) if error.kind() == io::ErrorKind::NotFound => NewsrcState::parse(""),
        Err(error) => fail(format_args!(
            "could not read {}: {error}",
            config.newsrc_path.display()
        )),
    };
    for issue in &newsrc.issues {
        eprintln!("newsrc line {}: {}", issue.line_number, issue.message);
    }
    SessionState {
        newsrc: newsrc.state,
        ..SessionState::default()
    }
}

fn save_newsrc(state: &SessionState, config: &config::Config) -> Result<(), String> {
    state
        .newsrc
        .write_atomic(&config.newsrc_path)
        .map_err(|error| format!("could not write {}: {error}", config.newsrc_path.display()))
}

/// Saves newsrc after an interactive reader returns, whether or not it
/// failed, so articles already read are not lost. Returns the reader's
/// result, or the failure message(s) to report.
fn save_after_reader<T>(
    state: &SessionState,
    config: &config::Config,
    reader_result: Result<T, terminal::reader::ReaderError>,
) -> Result<T, String> {
    let saved = save_newsrc(state, config);
    match (reader_result, saved) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(save_error)) => Err(save_error),
        (Err(error), Ok(())) => Err(format!("interactive reader failed: {error}")),
        (Err(error), Err(save_error)) => {
            Err(format!("interactive reader failed: {error}\n{save_error}"))
        }
    }
}

fn load_threads(
    session: &mut NntpSession,
    state: &mut SessionState,
    group: &GroupSelection,
    config: &config::Config,
    name: &str,
) -> LoadedGroupThreads {
    group_loader::load_group_threads(session, state, group, overview_limits(config))
        .unwrap_or_else(|error| fail(format_args!("overview for {name} failed: {error}")))
}

fn report_ignored_rows(loaded: &LoadedGroupThreads) {
    if loaded.invalid_rows > 0 {
        eprintln!("ignored {} malformed overview rows", loaded.invalid_rows);
    }
    if loaded.out_of_range_rows > 0 {
        eprintln!(
            "ignored {} overview rows outside requested ranges",
            loaded.out_of_range_rows
        );
    }
}

fn overview_limits(config: &config::Config) -> group_loader::OverviewLimits {
    group_loader::OverviewLimits {
        unread: config.initial_overview_limit,
        read_context: config.context_overview_limit,
        chunk_size: config.overview_chunk_size,
    }
}

/// Reports a usage error and exits with status 2.
fn usage_error(message: impl Display) -> ! {
    eprintln!("{message}");
    eprintln!("{}", cli::USAGE);
    std::process::exit(2);
}

/// Reports a runtime failure and exits with status 1.
fn fail(message: impl Display) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}

/// Restores the terminal before reporting a failure, so the message is
/// visible after the screen is torn down.
fn fail_in_terminal(terminal_session: TerminalSession, message: impl Display) -> ! {
    drop(terminal_session);
    fail(message);
}
