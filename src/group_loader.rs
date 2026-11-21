use crate::nntp::{GroupSelection, NntpSession, OverviewError};
use crate::persistence::newsrc::plan_unread_chunks;
use crate::state::thread_list::ThreadList;
use crate::state::{SessionState, StateError};
use std::io;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedGroupThreads {
    pub threads: ThreadList,
    pub invalid_rows: usize,
    pub out_of_range_rows: usize,
}

#[derive(Debug)]
pub enum GroupOverviewError {
    Overview(OverviewError),
    State(StateError),
    Progress(io::Error),
}

impl std::fmt::Display for GroupOverviewError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Overview(error) => write!(formatter, "{error}"),
            Self::State(error) => write!(formatter, "{error}"),
            Self::Progress(error) => {
                write!(formatter, "could not render loading progress: {error}")
            }
        }
    }
}

impl std::error::Error for GroupOverviewError {}

pub fn load_group_threads(
    session: &mut NntpSession,
    state: &mut SessionState,
    group: &GroupSelection,
    initial_overview_limit: usize,
    overview_chunk_size: usize,
) -> Result<LoadedGroupThreads, GroupOverviewError> {
    load_group_threads_with_progress(
        session,
        state,
        group,
        initial_overview_limit,
        overview_chunk_size,
        |_, _| Ok(()),
    )
}

pub fn load_group_threads_with_progress(
    session: &mut NntpSession,
    state: &mut SessionState,
    selection: &GroupSelection,
    initial_overview_limit: usize,
    overview_chunk_size: usize,
    mut on_progress: impl FnMut(usize, usize) -> io::Result<()>,
) -> Result<LoadedGroupThreads, GroupOverviewError> {
    let group = selection.name.as_str();
    let read_ranges = state
        .newsrc
        .entry(group)
        .map_or(&[][..], |entry| entry.read_ranges.as_slice());
    let ranges = plan_unread_chunks(
        selection.first_article,
        selection.last_article,
        read_ranges,
        initial_overview_limit,
        overview_chunk_size,
    );
    let total_chunks = ranges.len();
    on_progress(0, total_chunks).map_err(GroupOverviewError::Progress)?;
    let mut invalid_rows = 0;
    let mut out_of_range_rows = 0;
    for (completed_chunks, range) in ranges.into_iter().enumerate() {
        let rows = session
            .list_overview(range)
            .map_err(GroupOverviewError::Overview)?;
        invalid_rows += rows.invalid_rows.len();
        out_of_range_rows += rows.out_of_range_rows.len();
        state
            .load_overview(group, rows)
            .map_err(GroupOverviewError::State)?;
        on_progress(completed_chunks + 1, total_chunks).map_err(GroupOverviewError::Progress)?;
    }
    if !state.groups.contains_key(group) {
        state
            .load_articles(group, [])
            .map_err(GroupOverviewError::State)?;
    }
    let threads = ThreadList::from_session(state, group).map_err(GroupOverviewError::State)?;
    Ok(LoadedGroupThreads {
        threads,
        invalid_rows,
        out_of_range_rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{format_flat_overview, format_thread_summary};
    use crate::persistence::newsrc::{ArticleRange, NewsrcEntry};
    use crate::state::article_navigation::first_unread_article_for_thread_root;
    use crate::state::SessionState;
    use std::io::{BufRead, BufReader, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::thread;
    use std::time::Duration;

    fn fake_server(
        handler: impl FnOnce(TcpStream) + Send + 'static,
    ) -> (SocketAddr, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handler(stream);
        });
        (address, server)
    }

    fn read_command(reader: &mut impl BufRead, expected: &str) {
        let mut command = String::new();
        reader.read_line(&mut command).unwrap();
        assert_eq!(command, format!("{expected}\r\n"));
    }

    #[test]
    fn loads_bounded_overview_into_read_aware_threads() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            read_command(&mut reader, "CAPABILITIES");
            stream
                .write_all(b"101 capabilities\r\nREADER\r\nOVER\r\n.\r\n")
                .unwrap();
            read_command(&mut reader, "GROUP comp.test");
            stream.write_all(b"211 5 1 5 comp.test\r\n").unwrap();
            read_command(&mut reader, "LIST OVERVIEW.FMT");
            stream
                .write_all(
                    b"215 overview format\r\n\
                      Subject:\r\nFrom:\r\nDate:\r\nMessage-ID:\r\nReferences:\r\n:bytes\r\n:lines\r\n.\r\n",
                )
                .unwrap();
            read_command(&mut reader, "OVER 3-4");
            stream
                .write_all(
                    b"224 overview follows\r\n\
                      3\t=?UTF-8?Q?caf=C3=A9?=\tA <a@b>\tWed, 02 Jan 2019 10:00:00 +0000\t<root@b>\t\t12\t2\r\n\
                      4\tRe: topic\tA <a@b>\tWed, 02 Jan 2019 11:00:00 +0000\t<reply@b>\t<root@b>\t12\t2\r\n\
                      malformed\trow\r\n\
                      5\toutside requested range\tA <a@b>\tdate\t<outside@b>\t\t12\t2\r\n\
                      .\r\n",
                )
                .unwrap();
            read_command(&mut reader, "OVER 5-5");
            stream
                .write_all(
                    b"224 overview follows\r\n\
                      5\torphan\tA <a@b>\tThu, 03 Jan 2019 10:00:00 +0000\t<orphan@b>\t<missing@b>\t12\t2\r\n\
                      .\r\n",
                )
                .unwrap();
            read_command(&mut reader, "HEAD 3");
            stream
                .write_all(
                    b"221 3 <root@b> article retrieved\r\n\
                      Subject: =?UTF-8?Q?caf=C3=A9?=\r\n\
                      Content-Type: text/plain; charset=utf-8\r\n\
                      .\r\n",
                )
                .unwrap();
            read_command(&mut reader, "BODY 3");
            stream
                .write_all(b"222 3 <root@b> body follows\r\nhello\r\n.\r\n")
                .unwrap();
        });

        let mut nntp = NntpSession::connect(
            "127.0.0.1",
            address.port(),
            false,
            Duration::from_secs(2),
            Duration::from_secs(5),
        )
        .unwrap();
        nntp.discover_capabilities().unwrap();
        let selection = nntp.select_group("comp.test").unwrap();
        let mut state = SessionState::default();
        state.newsrc.entries.insert(
            "comp.test".to_owned(),
            NewsrcEntry {
                subscribed: true,
                read_ranges: vec![ArticleRange { first: 1, last: 2 }],
            },
        );

        let mut progress = Vec::new();
        let loaded = load_group_threads_with_progress(
            &mut nntp,
            &mut state,
            &selection,
            3,
            2,
            |completed, total| {
                progress.push((completed, total));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(progress, [(0, 2), (1, 2), (2, 2)]);
        assert_eq!(loaded.threads.rows.len(), 2);
        assert_eq!(loaded.threads.rows[0].root_article_number, 5);
        assert_eq!(loaded.threads.rows[0].subject, "orphan");
        assert_eq!(loaded.threads.rows[0].unread_count, 1);
        assert!(loaded.threads.rows[0].has_unresolved_parent);
        assert_eq!(loaded.threads.rows[1].root_article_number, 3);
        assert_eq!(loaded.threads.rows[1].subject, "caf\u{e9}");
        assert_eq!(loaded.threads.rows[1].article_numbers, [3, 4]);
        assert_eq!(loaded.threads.rows[1].unread_count, 2);
        assert_eq!(loaded.invalid_rows, 1);
        assert_eq!(loaded.out_of_range_rows, 1);
        assert!(state.newsrc.entry("comp.test").unwrap().subscribed);
        assert_eq!(
            state.newsrc.entry("comp.test").unwrap().read_ranges,
            [ArticleRange { first: 1, last: 2 }]
        );
        assert!(format_thread_summary(&loaded).contains("2 threads, 3 articles, 3 unread"));
        assert_eq!(
            format_flat_overview(&state, "comp.test"),
            "3 =?UTF-8?Q?caf=C3=A9?=\n4 Re: topic\n5 orphan\n"
        );
        let target = first_unread_article_for_thread_root(&state, "comp.test", 3).unwrap();
        assert_eq!(target, 3);
        let mut article_lines = nntp.article_headers(target).unwrap();
        article_lines.push(Vec::new());
        article_lines.extend(nntp.article_body(target).unwrap());
        let parsed = crate::model::article::RawArticle::parse(&article_lines);
        let decoded = crate::decoder::decode_article(&parsed.article);
        assert_eq!(decoded.headers[0].value, "caf\u{e9}");
        assert_eq!(decoded.body, "hello");
        let mut rendered = Vec::new();
        let context = crate::terminal::article_view::ArticleViewContext {
            group: "comp.test",
            article_number: target,
            more_unread: 0,
            group_last_article: selection.last_article,
        };
        let layout = crate::terminal::article_view::ArticleLayout::new(&context, &decoded, 80);
        crate::terminal::article_view::present_article(
            &mut rendered,
            &mut state,
            &context,
            &layout,
            80,
            layout.full_height(),
        )
        .unwrap();
        assert!(state.is_read("comp.test", target));
        assert!(String::from_utf8(rendered).unwrap().contains("hello"));
        server.join().unwrap();
    }

    #[test]
    fn empty_group_loads_without_requesting_an_invalid_overview_range() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            read_command(&mut reader, "CAPABILITIES");
            stream
                .write_all(b"101 capabilities\r\nREADER\r\nOVER\r\n.\r\n")
                .unwrap();
        });
        let mut nntp = NntpSession::connect(
            "127.0.0.1",
            address.port(),
            false,
            Duration::from_secs(2),
            Duration::from_secs(5),
        )
        .unwrap();
        nntp.discover_capabilities().unwrap();
        let mut state = SessionState::default();
        let mut progress = Vec::new();
        let loaded = load_group_threads_with_progress(
            &mut nntp,
            &mut state,
            &GroupSelection {
                name: "empty.test".to_owned(),
                count: 0,
                first_article: 0,
                last_article: 0,
                count_consistent: true,
            },
            10,
            5,
            |completed, total| {
                progress.push((completed, total));
                Ok(())
            },
        )
        .unwrap();
        assert!(loaded.threads.rows.is_empty());
        assert_eq!(progress, [(0, 0)]);
        assert_eq!(
            format_thread_summary(&loaded),
            "empty.test: 0 threads, 0 articles, 0 unread\n"
        );
        server.join().unwrap();
    }
}
