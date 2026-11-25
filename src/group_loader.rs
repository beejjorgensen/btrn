use crate::nntp::{GroupSelection, NntpSession, OverviewError, OverviewRows};
use crate::persistence::newsrc::plan_overview_chunks;
use crate::state::thread_list::ThreadList;
use crate::state::{SessionState, StateError};
use std::io;

/// How many overview rows a group load requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverviewLimits {
    /// The newest unread articles to load.
    pub unread: usize,
    /// The newest read articles also loaded, only to give the thread tree
    /// already-read parents and siblings.
    pub read_context: usize,
    /// Article numbers per `OVER` request.
    pub chunk_size: usize,
}

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
    limits: OverviewLimits,
) -> Result<LoadedGroupThreads, GroupOverviewError> {
    load_group_threads_with_progress(session, state, group, limits, |_, _| Ok(()))
}

pub fn load_group_threads_with_progress(
    session: &mut NntpSession,
    state: &mut SessionState,
    selection: &GroupSelection,
    limits: OverviewLimits,
    mut on_progress: impl FnMut(usize, usize) -> io::Result<()>,
) -> Result<LoadedGroupThreads, GroupOverviewError> {
    let group = selection.name.as_str();
    let read_ranges = state
        .newsrc
        .entry(group)
        .map_or(&[][..], |entry| entry.read_ranges.as_slice());
    let ranges = plan_overview_chunks(
        selection.first_article,
        selection.last_article,
        read_ranges,
        limits.unread,
        limits.read_context,
        limits.chunk_size,
    );
    let total_chunks = ranges.len();
    on_progress(0, total_chunks).map_err(GroupOverviewError::Progress)?;
    let mut invalid_rows = 0;
    let mut out_of_range_rows = 0;
    // Rows from every chunk are loaded together so the thread tree is built
    // once per group load.
    let mut loaded = OverviewRows {
        articles: Vec::new(),
        invalid_rows: Vec::new(),
        out_of_range_rows: Vec::new(),
    };
    for (completed_chunks, range) in ranges.into_iter().enumerate() {
        let rows = session
            .list_overview(range)
            .map_err(GroupOverviewError::Overview)?;
        invalid_rows += rows.invalid_rows.len();
        out_of_range_rows += rows.out_of_range_rows.len();
        loaded.articles.extend(rows.articles);
        on_progress(completed_chunks + 1, total_chunks).map_err(GroupOverviewError::Progress)?;
    }
    state
        .load_overview(group, loaded)
        .map_err(GroupOverviewError::State)?;
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
    use crate::nntp::overview::OverviewError;
    use crate::nntp::GroupSelection;
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
            OverviewLimits {
                unread: 3,
                read_context: 0,
                chunk_size: 2,
            },
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
            header_mode: Default::default(),
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
    fn read_context_rows_attach_replies_to_already_read_parents() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            read_command(&mut reader, "CAPABILITIES");
            stream
                .write_all(b"101 capabilities\r\nREADER\r\nOVER\r\n.\r\n")
                .unwrap();
            read_command(&mut reader, "GROUP comp.test");
            stream.write_all(b"211 4 1 4 comp.test\r\n").unwrap();
            read_command(&mut reader, "LIST OVERVIEW.FMT");
            stream
                .write_all(
                    b"215 overview format\r\n\
                      Subject:\r\nFrom:\r\nDate:\r\nMessage-ID:\r\nReferences:\r\n:bytes\r\n:lines\r\n.\r\n",
                )
                .unwrap();
            // The newest read article joins the unread range in one request.
            read_command(&mut reader, "OVER 2-4");
            stream
                .write_all(
                    b"224 overview follows\r\n\
                      2\tRe: topic\tA <a@b>\tWed, 02 Jan 2019 10:00:00 +0000\t<parent@b>\t<root@b>\t12\t2\r\n\
                      3\tRe: topic\tB <b@b>\tWed, 02 Jan 2019 11:00:00 +0000\t<reply@b>\t<root@b> <parent@b>\t12\t2\r\n\
                      4\tother\tC <c@b>\tWed, 02 Jan 2019 12:00:00 +0000\t<other@b>\t\t12\t2\r\n\
                      .\r\n",
                )
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

        let loaded = load_group_threads(
            &mut nntp,
            &mut state,
            &selection,
            OverviewLimits {
                unread: 10,
                read_context: 1,
                chunk_size: 10,
            },
        )
        .unwrap();
        server.join().unwrap();

        let group = &state.groups["comp.test"];
        assert_eq!(group.threads.nodes[&3].parent, Some(2));
        assert_eq!(group.threads.nodes[&3].unresolved_parent, None);
        let thread = loaded
            .threads
            .rows
            .iter()
            .find(|row| row.article_numbers.contains(&3))
            .unwrap();
        assert_eq!(thread.article_numbers, [2, 3]);
        assert_eq!(thread.unread_count, 1);
        assert_eq!(
            state.newsrc.entry("comp.test").unwrap().read_ranges,
            [ArticleRange { first: 1, last: 2 }]
        );
        assert_eq!(
            first_unread_article_for_thread_root(&state, "comp.test", 2),
            Ok(3)
        );
    }

    #[test]
    fn interleaved_read_marks_load_in_full_chunks_and_thread_across_them() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            read_command(&mut reader, "CAPABILITIES");
            stream
                .write_all(b"101 capabilities\r\nREADER\r\nOVER\r\n.\r\n")
                .unwrap();
            read_command(&mut reader, "GROUP comp.test");
            stream.write_all(b"211 6 1 6 comp.test\r\n").unwrap();
            read_command(&mut reader, "LIST OVERVIEW.FMT");
            stream
                .write_all(
                    b"215 overview format\r\n\
                      Subject:\r\nFrom:\r\nDate:\r\nMessage-ID:\r\nReferences:\r\n:bytes\r\n:lines\r\n.\r\n",
                )
                .unwrap();
            // Read marks 1, 3 and 5 alternate with unread articles, yet the
            // plan asks for whole chunks.
            read_command(&mut reader, "OVER 1-3");
            stream
                .write_all(
                    b"224 overview follows\r\n\
                      1\ttopic\tA <a@b>\tWed, 02 Jan 2019 10:00:00 +0000\t<a1@b>\t\t12\t2\r\n\
                      2\tRe: topic\tB <b@b>\tWed, 02 Jan 2019 10:01:00 +0000\t<a2@b>\t<a1@b>\t12\t2\r\n\
                      3\tRe: topic\tA <a@b>\tWed, 02 Jan 2019 10:02:00 +0000\t<a3@b>\t<a1@b> <a2@b>\t12\t2\r\n\
                      .\r\n",
                )
                .unwrap();
            read_command(&mut reader, "OVER 4-6");
            stream
                .write_all(
                    b"224 overview follows\r\n\
                      4\tRe: topic\tB <b@b>\tWed, 02 Jan 2019 10:03:00 +0000\t<a4@b>\t<a1@b> <a2@b> <a3@b>\t12\t2\r\n\
                      5\tRe: topic\tA <a@b>\tWed, 02 Jan 2019 10:04:00 +0000\t<a5@b>\t<a1@b>\t12\t2\r\n\
                      6\tRe: topic\tB <b@b>\tWed, 02 Jan 2019 10:05:00 +0000\t<a6@b>\t<a1@b> <a5@b>\t12\t2\r\n\
                      .\r\n",
                )
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
        let read = [1, 3, 5].map(|number| ArticleRange {
            first: number,
            last: number,
        });
        state.newsrc.entries.insert(
            "comp.test".to_owned(),
            NewsrcEntry {
                subscribed: true,
                read_ranges: read.to_vec(),
            },
        );

        let mut progress = Vec::new();
        let loaded = load_group_threads_with_progress(
            &mut nntp,
            &mut state,
            &selection,
            OverviewLimits {
                unread: 100,
                read_context: 100,
                chunk_size: 3,
            },
            |completed, total| {
                progress.push((completed, total));
                Ok(())
            },
        )
        .unwrap();
        server.join().unwrap();

        assert_eq!(progress, [(0, 2), (1, 2), (2, 2)]);
        let tree = &state.groups["comp.test"].threads;
        assert_eq!(tree.nodes[&4].parent, Some(3));
        assert_eq!(tree.nodes[&6].parent, Some(5));
        assert_eq!(loaded.threads.rows.len(), 1);
        assert_eq!(loaded.threads.rows[0].article_numbers, [1, 2, 3, 4, 5, 6]);
        assert_eq!(loaded.threads.rows[0].unread_count, 3);
        assert_eq!(state.newsrc.entry("comp.test").unwrap().read_ranges, read);
    }

    /// Serves the greeting, capabilities, GROUP for comp.test 1-4, and the
    /// overview format, then hands the stream to `rest`.
    fn four_article_group_server(
        rest: impl FnOnce(&mut TcpStream, &mut BufReader<TcpStream>) + Send + 'static,
    ) -> (SocketAddr, thread::JoinHandle<()>) {
        fake_server(move |mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            read_command(&mut reader, "CAPABILITIES");
            stream
                .write_all(b"101 capabilities\r\nREADER\r\nOVER\r\n.\r\n")
                .unwrap();
            read_command(&mut reader, "GROUP comp.test");
            stream.write_all(b"211 4 1 4 comp.test\r\n").unwrap();
            rest(&mut stream, &mut reader);
        })
    }

    fn serve_overview_format(stream: &mut TcpStream, reader: &mut BufReader<TcpStream>) {
        read_command(reader, "LIST OVERVIEW.FMT");
        stream
            .write_all(
                b"215 overview format\r\n\
                  Subject:\r\nFrom:\r\nDate:\r\nMessage-ID:\r\nReferences:\r\n:bytes\r\n:lines\r\n.\r\n",
            )
            .unwrap();
    }

    fn connect_to_four_article_group(address: SocketAddr) -> (NntpSession, GroupSelection) {
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
        (nntp, selection)
    }

    const TWO_PER_CHUNK: OverviewLimits = OverviewLimits {
        unread: 10,
        read_context: 0,
        chunk_size: 2,
    };

    #[test]
    fn a_failed_later_chunk_leaves_session_state_unchanged() {
        let (address, server) = four_article_group_server(|stream, reader| {
            serve_overview_format(stream, reader);
            read_command(reader, "OVER 1-2");
            stream
                .write_all(
                    b"224 overview follows\r\n\
                      2\tsubject\tA <a@b>\tWed, 02 Jan 2019 10:00:00 +0000\t<four@b>\t\t12\t2\r\n\
                      .\r\n",
                )
                .unwrap();
            read_command(reader, "OVER 3-4");
            stream.write_all(b"503 overview unavailable\r\n").unwrap();
        });
        let (mut nntp, selection) = connect_to_four_article_group(address);
        let mut state = SessionState::default();

        let error = load_group_threads(&mut nntp, &mut state, &selection, TWO_PER_CHUNK)
            .err()
            .unwrap();
        server.join().unwrap();
        assert!(
            matches!(
                error,
                GroupOverviewError::Overview(OverviewError::UnexpectedStatus(503))
            ),
            "{error:?}"
        );
        assert!(state.groups.is_empty());
        assert!(state.newsrc.entries.is_empty());
    }

    #[test]
    fn a_progress_failure_stops_the_load() {
        let (address, server) = four_article_group_server(|stream, reader| {
            serve_overview_format(stream, reader);
            read_command(reader, "OVER 1-2");
            stream.write_all(b"224 overview follows\r\n.\r\n").unwrap();
        });
        let (mut nntp, selection) = connect_to_four_article_group(address);
        let mut state = SessionState::default();
        let mut reports = Vec::new();

        let error = load_group_threads_with_progress(
            &mut nntp,
            &mut state,
            &selection,
            TWO_PER_CHUNK,
            |completed, total| {
                reports.push((completed, total));
                if completed == 1 {
                    Err(io::Error::other("terminal gone"))
                } else {
                    Ok(())
                }
            },
        )
        .err()
        .unwrap();
        server.join().unwrap();
        assert!(
            matches!(error, GroupOverviewError::Progress(_)),
            "{error:?}"
        );
        assert_eq!(
            error.to_string(),
            "could not render loading progress: terminal gone"
        );
        // The second chunk was never requested.
        assert_eq!(reports, [(0, 2), (1, 2)]);
        assert!(state.groups.is_empty());
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
            OverviewLimits {
                unread: 10,
                read_context: 10,
                chunk_size: 5,
            },
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
