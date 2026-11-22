//! Parsing and formatting the `From` mailbox.

/// A parsed `From` value: an optional display name and an address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mailbox {
    pub name: Option<String>,
    pub address: String,
}

/// Parses `Name <addr>`, `"Quoted Name" <addr>`, or a bare `addr`.
pub fn parse_mailbox(value: &str) -> Option<Mailbox> {
    let value = value.trim();
    let (name, address) = match (value.rfind('<'), value.ends_with('>')) {
        (Some(open), true) => (
            unquote(value[..open].trim()),
            &value[open + 1..value.len() - 1],
        ),
        _ => (None, value),
    };
    valid_address(address).then(|| Mailbox {
        name,
        address: address.to_owned(),
    })
}

fn unquote(name: &str) -> Option<String> {
    let name = match name
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
    {
        Some(inner) => {
            let mut unescaped = String::new();
            let mut characters = inner.chars();
            while let Some(character) = characters.next() {
                if character == '\\' {
                    unescaped.extend(characters.next());
                } else {
                    unescaped.push(character);
                }
            }
            unescaped
        }
        None => name.to_owned(),
    };
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// A simple `local@domain` address check: one `@`, non-empty parts, and
/// only printable ASCII without address specials.
fn valid_address(address: &str) -> bool {
    let Some((local, domain)) = address.split_once('@') else {
        return false;
    };
    let allowed =
        |character: char| character.is_ascii_graphic() && !"<>()[]\\,;:\"@".contains(character);
    !local.is_empty()
        && !domain.is_empty()
        && local.chars().all(allowed)
        && domain
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-.".contains(character))
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains("..")
}

impl Mailbox {
    /// The domain of the address.
    pub fn domain(&self) -> &str {
        self.address
            .rsplit_once('@')
            .map_or("", |(_, domain)| domain)
    }

    /// Formats the mailbox for the posted `From` header. An ASCII name is an
    /// RFC 5322 quoted-string with `"` and `\` escaped; a non-ASCII name is
    /// RFC 2047 encoded-words, which may not appear inside quotes.
    pub fn header_value(&self) -> String {
        match &self.name {
            None => self.address.clone(),
            Some(name) if name.is_ascii() => {
                let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
                format!("\"{escaped}\" <{}>", self.address)
            }
            Some(name) => format!(
                "{} <{}>",
                super::outgoing::encoded_words(name).join(" "),
                self.address
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mailbox(name: Option<&str>, address: &str) -> Mailbox {
        Mailbox {
            name: name.map(str::to_owned),
            address: address.to_owned(),
        }
    }

    #[test]
    fn parses_named_quoted_and_bare_addresses() {
        assert_eq!(
            parse_mailbox("Example Name <user@example.com>"),
            Some(mailbox(Some("Example Name"), "user@example.com"))
        );
        assert_eq!(
            parse_mailbox(r#""Doe, \"JD\" John" <jd@example.com>"#),
            Some(mailbox(Some(r#"Doe, "JD" John"#), "jd@example.com"))
        );
        assert_eq!(
            parse_mailbox(" user@example.com "),
            Some(mailbox(None, "user@example.com"))
        );
        assert_eq!(
            parse_mailbox("<user@example.com>"),
            Some(mailbox(None, "user@example.com"))
        );
    }

    #[test]
    fn rejects_values_without_a_valid_address() {
        for value in [
            "",
            "Example Name",
            "Example <>",
            "user@",
            "@example.com",
            "a@b@c",
            "user name@example.com",
            "user@exa mple.com",
            "user@.example.com",
            "user@example..com",
        ] {
            assert_eq!(parse_mailbox(value), None, "{value}");
        }
    }

    #[test]
    fn formats_quoted_escaped_and_encoded_names() {
        assert_eq!(
            mailbox(Some("Example Name"), "user@example.com").header_value(),
            r#""Example Name" <user@example.com>"#
        );
        assert_eq!(
            mailbox(Some(r#"Odd "Q" \ Name"#), "u@example.com").header_value(),
            r#""Odd \"Q\" \\ Name" <u@example.com>"#
        );
        assert_eq!(
            mailbox(None, "u@example.com").header_value(),
            "u@example.com"
        );
        assert_eq!(
            mailbox(Some("Zoë"), "u@example.com").header_value(),
            "=?UTF-8?B?Wm/Dqw==?= <u@example.com>"
        );
        assert_eq!(
            mailbox(None, "u@mail.example.com").domain(),
            "mail.example.com"
        );
    }
}
