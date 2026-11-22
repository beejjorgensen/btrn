//! Command-line argument parsing and plain-text output for the
//! non-interactive modes.

use crate::group_loader::LoadedGroupThreads;
use crate::state::SessionState;

pub const USAGE: &str = "usage: btrn [host] [port] [--plaintext] [--group <name>] [--article <number>] [--thread-root <number>] [--flat] [--interactive]\n\
     use --interactive without --group to select newsgroups interactively\n\
     configure server and optional credentials in ~/.config/btrn/config.toml";

/// The parsed command line.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Arguments {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub plaintext: bool,
    pub flat_overview: bool,
    pub interactive: bool,
    pub group: Option<String>,
    pub article: Option<u64>,
    pub thread_root: Option<u64>,
}

impl Arguments {
    /// Parses the arguments after the program name. The error is the
    /// message to show above the usage text.
    pub fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut arguments = arguments.into_iter();
        let mut parsed = Self::default();
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--plaintext" => parsed.plaintext = true,
                "--flat" => parsed.flat_overview = true,
                "--interactive" => parsed.interactive = true,
                "--group" => {
                    if parsed.group.is_some() {
                        return Err("--group may only be specified once".to_owned());
                    }
                    parsed.group = Some(arguments.next().ok_or("--group requires a group name")?);
                }
                "--article" => {
                    if parsed.article.is_some() {
                        return Err("--article may only be specified once".to_owned());
                    }
                    parsed.article = Some(
                        positive_article_number(arguments.next())
                            .ok_or("--article requires a positive article number")?,
                    );
                }
                "--thread-root" => {
                    if parsed.thread_root.is_some() {
                        return Err("--thread-root may only be specified once".to_owned());
                    }
                    parsed.thread_root = Some(
                        positive_article_number(arguments.next())
                            .ok_or("--thread-root requires a positive article number")?,
                    );
                }
                _ if parsed.host.is_none() => parsed.host = Some(argument),
                _ if parsed.port.is_none() => {
                    parsed.port = Some(
                        argument
                            .parse::<u16>()
                            .map_err(|_| format!("unexpected argument: {argument}"))?,
                    );
                }
                _ => return Err(format!("unexpected argument: {argument}")),
            }
        }
        Ok(parsed)
    }

    /// Rejects option combinations that have no meaning together.
    pub fn check_combinations(&self) -> Result<(), &'static str> {
        let group = self.group.is_some();
        let article = self.article.is_some();
        let thread_root = self.thread_root.is_some();
        if article && !group {
            return Err("--article requires --group");
        }
        if thread_root && !group {
            return Err("--thread-root requires --group");
        }
        if thread_root && article {
            return Err("--thread-root cannot be used with --article");
        }
        if thread_root && self.flat_overview {
            return Err("--thread-root cannot be used with --flat");
        }
        if self.flat_overview && !group {
            return Err("--flat requires --group");
        }
        if self.flat_overview && article {
            return Err("--flat cannot be used with --article");
        }
        if self.interactive && (article || thread_root || self.flat_overview) {
            return Err(
                "--interactive cannot be combined with --article, --thread-root, or --flat",
            );
        }
        Ok(())
    }
}

fn positive_article_number(value: Option<String>) -> Option<u64> {
    value?.parse::<u64>().ok().filter(|number| *number > 0)
}

pub fn format_thread_summary(loaded: &LoadedGroupThreads) -> String {
    let article_count = loaded
        .threads
        .rows
        .iter()
        .map(|row| row.article_numbers.len())
        .sum::<usize>();
    let unread_count = loaded
        .threads
        .rows
        .iter()
        .map(|row| row.unread_count)
        .sum::<usize>();
    let mut output = format!(
        "{}: {} threads, {} articles, {} unread\n",
        loaded.threads.group_name,
        loaded.threads.rows.len(),
        article_count,
        unread_count
    );
    for row in &loaded.threads.rows {
        let unresolved = if row.has_unresolved_parent {
            " (?)"
        } else {
            ""
        };
        output.push_str(&format!(
            "{}/{} {}{} {}\n",
            row.unread_count,
            row.article_numbers.len(),
            row.root_article_number,
            unresolved,
            row.subject
        ));
    }
    output
}

pub fn format_flat_overview(state: &SessionState, group: &str) -> String {
    let mut output = String::new();
    if let Some(group_state) = state.groups.get(group) {
        for (number, metadata) in &group_state.articles {
            let subject = metadata
                .subject
                .as_deref()
                .map(String::from_utf8_lossy)
                .unwrap_or_default();
            output.push_str(&format!("{number} {subject}\n"));
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> Result<Arguments, String> {
        Arguments::parse(arguments.iter().map(|argument| (*argument).to_owned()))
    }

    #[test]
    fn parses_positional_host_and_port_and_options() {
        let parsed =
            parse(&["news.example", "119", "--plaintext", "--group", "comp.test"]).unwrap();
        assert_eq!(parsed.host.as_deref(), Some("news.example"));
        assert_eq!(parsed.port, Some(119));
        assert!(parsed.plaintext);
        assert_eq!(parsed.group.as_deref(), Some("comp.test"));
        assert_eq!(parse(&[]).unwrap(), Arguments::default());
    }

    #[test]
    fn reports_bad_values_duplicates_and_extra_arguments() {
        assert_eq!(parse(&["host", "x"]).unwrap_err(), "unexpected argument: x");
        assert_eq!(
            parse(&["host", "1", "extra"]).unwrap_err(),
            "unexpected argument: extra"
        );
        assert_eq!(
            parse(&["--group"]).unwrap_err(),
            "--group requires a group name"
        );
        assert_eq!(
            parse(&["--group", "a", "--group", "b"]).unwrap_err(),
            "--group may only be specified once"
        );
        assert_eq!(
            parse(&["--article", "0"]).unwrap_err(),
            "--article requires a positive article number"
        );
        assert_eq!(
            parse(&["--thread-root", "1", "--thread-root", "2"]).unwrap_err(),
            "--thread-root may only be specified once"
        );
    }

    #[test]
    fn rejects_meaningless_option_combinations() {
        let check = |arguments: &[&str]| parse(arguments).unwrap().check_combinations();
        assert_eq!(
            check(&["--article", "1"]),
            Err("--article requires --group")
        );
        assert_eq!(
            check(&["--group", "g", "--thread-root", "1", "--article", "2"]),
            Err("--thread-root cannot be used with --article")
        );
        assert_eq!(check(&["--flat"]), Err("--flat requires --group"));
        assert_eq!(
            check(&["--group", "g", "--interactive", "--flat"]),
            Err("--interactive cannot be combined with --article, --thread-root, or --flat")
        );
        assert_eq!(check(&["--group", "g", "--article", "1"]), Ok(()));
        assert_eq!(check(&["--interactive"]), Ok(()));
    }
}
