use crate::model::article::{HeaderField, RawArticle};
use base64::{engine::general_purpose::STANDARD, Engine};
use encoding_rs::{Encoding, UTF_8};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedArticle {
    pub headers: Vec<DecodedHeader>,
    pub body: String,
    pub issues: Vec<DecodeIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeIssue {
    MalformedEncodedWord { header: String },
    UnsupportedCharset { label: String },
    InvalidCharsetData { label: String },
    MalformedTransferEncoding { encoding: String },
    UnsupportedTransferEncoding { encoding: String },
    UnsupportedContentType { media_type: String },
    MalformedMultipart,
    MultipartDepthExceeded,
    NoPlainTextPart,
}

impl std::fmt::Display for DecodeIssue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MalformedEncodedWord { header } => {
                write!(formatter, "malformed encoded word in {header}")
            }
            Self::UnsupportedCharset { label } => write!(formatter, "unsupported charset {label}"),
            Self::InvalidCharsetData { label } => {
                write!(formatter, "invalid data for charset {label}")
            }
            Self::MalformedTransferEncoding { encoding } => {
                write!(formatter, "malformed {encoding} transfer-encoded data")
            }
            Self::UnsupportedTransferEncoding { encoding } => {
                write!(formatter, "unsupported transfer encoding {encoding}")
            }
            Self::UnsupportedContentType { media_type } => {
                write!(formatter, "unsupported content type {media_type}")
            }
            Self::MalformedMultipart => formatter.write_str("malformed multipart article"),
            Self::MultipartDepthExceeded => {
                formatter.write_str("multipart nesting exceeds the supported depth")
            }
            Self::NoPlainTextPart => {
                formatter.write_str("multipart article has no supported text/plain part")
            }
        }
    }
}

pub fn decode_article(article: &RawArticle) -> DecodedArticle {
    let mut issues = Vec::new();
    let headers = article
        .headers
        .iter()
        .map(|header| {
            let (value, mut header_issues) = decode_header_value(&header.name, &header.value);
            issues.append(&mut header_issues);
            DecodedHeader {
                name: String::from_utf8_lossy(&header.name).into_owned(),
                value,
            }
        })
        .collect();
    let body_bytes = join_lines(&article.body);
    let body = decode_entity(&article.headers, &body_bytes, 0, &mut issues).unwrap_or_default();
    DecodedArticle {
        headers,
        body,
        issues,
    }
}

pub fn decode_header_value(name: &[u8], value: &[u8]) -> (String, Vec<DecodeIssue>) {
    let mut issues = Vec::new();
    let decoded = if is_mime_control_header(name) {
        decode_utf8(value)
    } else {
        decode_encoded_words(name, value, &mut issues)
    };
    (decoded, issues)
}

fn decode_entity(
    headers: &[HeaderField],
    body: &[u8],
    depth: usize,
    issues: &mut Vec<DecodeIssue>,
) -> Option<String> {
    const MAX_MULTIPART_DEPTH: usize = 16;
    if depth > MAX_MULTIPART_DEPTH {
        issues.push(DecodeIssue::MultipartDepthExceeded);
        return None;
    }

    let content_type = header_value(headers, b"Content-Type");
    let (media_type, parameters) =
        parse_content_type(content_type.unwrap_or(b"text/plain; charset=us-ascii"));
    if media_type.starts_with("multipart/") {
        let Some(boundary) = parameters.get("boundary") else {
            issues.push(DecodeIssue::MalformedMultipart);
            return None;
        };
        let Some(parts) = split_multipart(body, boundary.as_bytes()) else {
            issues.push(DecodeIssue::MalformedMultipart);
            return None;
        };
        for part in parts {
            let Some((part_headers, part_body)) = split_entity(&part) else {
                issues.push(DecodeIssue::MalformedMultipart);
                continue;
            };
            let part_type = header_value(&part_headers, b"Content-Type")
                .map(|value| parse_content_type(value).0)
                .unwrap_or_else(|| "text/plain".to_owned());
            if part_type != "text/plain" && !part_type.starts_with("multipart/") {
                continue;
            }
            if let Some(decoded) = decode_entity(&part_headers, &part_body, depth + 1, issues) {
                return Some(decoded);
            }
        }
        issues.push(DecodeIssue::NoPlainTextPart);
        return None;
    }
    if media_type != "text/plain" {
        issues.push(DecodeIssue::UnsupportedContentType { media_type });
        return None;
    }

    let transfer_encoding = header_value(headers, b"Content-Transfer-Encoding")
        .map(trim_ascii)
        .unwrap_or(b"7bit");
    let decoded_bytes = decode_transfer_encoding(body, transfer_encoding, issues);
    let charset = parameters
        .get("charset")
        .map(String::as_str)
        .unwrap_or("us-ascii");
    let text = decode_charset(&decoded_bytes, charset, issues);
    let text = normalize_line_endings(&text);
    Some(text)
}

fn decode_transfer_encoding(
    body: &[u8],
    encoding: &[u8],
    issues: &mut Vec<DecodeIssue>,
) -> Vec<u8> {
    if encoding.eq_ignore_ascii_case(b"7bit")
        || encoding.eq_ignore_ascii_case(b"8bit")
        || encoding.eq_ignore_ascii_case(b"binary")
    {
        return body.to_vec();
    }
    if encoding.eq_ignore_ascii_case(b"base64") {
        let compact = body
            .iter()
            .copied()
            .filter(|byte| !byte.is_ascii_whitespace())
            .collect::<Vec<_>>();
        return match STANDARD.decode(compact) {
            Ok(decoded) => decoded,
            Err(_) => {
                issues.push(DecodeIssue::MalformedTransferEncoding {
                    encoding: "base64".to_owned(),
                });
                body.to_vec()
            }
        };
    }
    if encoding.eq_ignore_ascii_case(b"quoted-printable") {
        return decode_quoted_printable(body, issues);
    }
    let label = String::from_utf8_lossy(encoding).into_owned();
    issues.push(DecodeIssue::UnsupportedTransferEncoding { encoding: label });
    body.to_vec()
}

fn decode_quoted_printable(body: &[u8], issues: &mut Vec<DecodeIssue>) -> Vec<u8> {
    let mut decoded = Vec::with_capacity(body.len());
    let mut index = 0;
    let mut malformed = false;
    while index < body.len() {
        if body[index] != b'=' {
            decoded.push(body[index]);
            index += 1;
            continue;
        }
        if body.get(index + 1..index + 3) == Some(b"\r\n") {
            index += 3;
            continue;
        }
        if body.get(index + 1) == Some(&b'\n') {
            index += 2;
            continue;
        }
        if let (Some(high), Some(low)) = (
            body.get(index + 1).and_then(|byte| hex_value(*byte)),
            body.get(index + 2).and_then(|byte| hex_value(*byte)),
        ) {
            decoded.push((high << 4) | low);
            index += 3;
            continue;
        }
        malformed = true;
        decoded.push(b'=');
        index += 1;
    }
    if malformed {
        issues.push(DecodeIssue::MalformedTransferEncoding {
            encoding: "quoted-printable".to_owned(),
        });
    }
    decoded
}

fn decode_charset(bytes: &[u8], label: &str, issues: &mut Vec<DecodeIssue>) -> String {
    if label.eq_ignore_ascii_case("us-ascii") || label.eq_ignore_ascii_case("ascii") {
        if bytes.iter().any(|byte| !byte.is_ascii()) {
            issues.push(DecodeIssue::InvalidCharsetData {
                label: label.to_owned(),
            });
        }
        return decode_utf8(bytes);
    }
    let Some(encoding) = Encoding::for_label(label.trim().as_bytes()) else {
        issues.push(DecodeIssue::UnsupportedCharset {
            label: label.to_owned(),
        });
        return decode_utf8(bytes);
    };
    let (decoded, _, had_errors) = encoding.decode(bytes);
    if had_errors {
        issues.push(DecodeIssue::InvalidCharsetData {
            label: label.to_owned(),
        });
    }
    decoded.into_owned()
}

fn decode_encoded_words(header_name: &[u8], value: &[u8], issues: &mut Vec<DecodeIssue>) -> String {
    let header = String::from_utf8_lossy(header_name).into_owned();
    let mut output = String::new();
    let mut cursor = 0;
    let mut previous_was_encoded = false;
    while let Some(relative_start) = find_bytes(&value[cursor..], b"=?") {
        let start = cursor + relative_start;
        let literal = &value[cursor..start];
        let token = parse_encoded_word(value, start);
        if previous_was_encoded
            && literal.iter().all(|byte| matches!(byte, b' ' | b'\t'))
            && token.is_some()
        {
            cursor = start;
            previous_was_encoded = false;
            continue;
        }
        output.push_str(&decode_utf8(literal));
        let Some((end, charset, encoding, encoded)) = token else {
            // Keep the `=?` as text and look for later encoded words.
            issues.push(DecodeIssue::MalformedEncodedWord {
                header: header.clone(),
            });
            output.push_str("=?");
            cursor = start + 2;
            previous_was_encoded = false;
            continue;
        };
        let bytes = match encoding {
            b'B' | b'b' => {
                let compact = encoded
                    .iter()
                    .copied()
                    .filter(|byte| !byte.is_ascii_whitespace())
                    .collect::<Vec<_>>();
                match STANDARD.decode(compact) {
                    Ok(bytes) => bytes,
                    Err(_) => {
                        issues.push(DecodeIssue::MalformedEncodedWord {
                            header: header.clone(),
                        });
                        output.push_str(&decode_utf8(&value[start..end]));
                        cursor = end;
                        previous_was_encoded = false;
                        continue;
                    }
                }
            }
            b'Q' | b'q' => match decode_header_q(encoded) {
                Some(bytes) => bytes,
                None => {
                    issues.push(DecodeIssue::MalformedEncodedWord {
                        header: header.clone(),
                    });
                    output.push_str(&decode_utf8(&value[start..end]));
                    cursor = end;
                    previous_was_encoded = false;
                    continue;
                }
            },
            _ => {
                issues.push(DecodeIssue::MalformedEncodedWord {
                    header: header.clone(),
                });
                output.push_str(&decode_utf8(&value[start..end]));
                cursor = end;
                previous_was_encoded = false;
                continue;
            }
        };
        let decoded = if charset.eq_ignore_ascii_case(b"us-ascii") {
            if bytes.iter().any(|byte| !byte.is_ascii()) {
                issues.push(DecodeIssue::InvalidCharsetData {
                    label: "us-ascii".to_owned(),
                });
            }
            decode_utf8(&bytes)
        } else if let Some(encoding) = Encoding::for_label(charset) {
            let (decoded, _, had_errors) = encoding.decode(&bytes);
            if had_errors {
                issues.push(DecodeIssue::InvalidCharsetData {
                    label: String::from_utf8_lossy(charset).into_owned(),
                });
            }
            decoded.into_owned()
        } else {
            let label = String::from_utf8_lossy(charset).into_owned();
            issues.push(DecodeIssue::UnsupportedCharset {
                label: label.clone(),
            });
            output.push_str(&decode_utf8(&value[start..end]));
            cursor = end;
            previous_was_encoded = false;
            continue;
        };
        output.push_str(&decoded);
        cursor = end;
        previous_was_encoded = true;
    }
    output.push_str(&decode_utf8(&value[cursor..]));
    output
}

fn parse_encoded_word(value: &[u8], start: usize) -> Option<(usize, &[u8], u8, &[u8])> {
    let after_start = value.get(start + 2..)?;
    let charset_end = after_start.iter().position(|byte| *byte == b'?')?;
    let charset = &after_start[..charset_end];
    if charset.is_empty() {
        return None;
    }
    let encoding_start = start + 2 + charset_end + 1;
    let encoding = *value.get(encoding_start)?;
    if value.get(encoding_start + 1) != Some(&b'?') {
        return None;
    }
    let payload_start = encoding_start + 2;
    let terminator = find_bytes(value.get(payload_start..)?, b"?=")? + payload_start;
    Some((
        terminator + 2,
        charset,
        encoding,
        &value[payload_start..terminator],
    ))
}

fn decode_header_q(value: &[u8]) -> Option<Vec<u8>> {
    let mut decoded = Vec::with_capacity(value.len());
    let mut index = 0;
    while index < value.len() {
        match value[index] {
            b'_' => {
                decoded.push(b' ');
                index += 1;
            }
            b'=' => {
                let high = hex_value(*value.get(index + 1)?)?;
                let low = hex_value(*value.get(index + 2)?)?;
                decoded.push((high << 4) | low);
                index += 3;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    Some(decoded)
}

fn split_entity(entity: &[u8]) -> Option<(Vec<HeaderField>, Vec<u8>)> {
    let lines = entity.split(|byte| *byte == b'\n').collect::<Vec<_>>();
    let separator = lines.iter().position(|line| trim_cr(line).is_empty())?;
    let mut headers: Vec<HeaderField> = Vec::new();
    for line in &lines[..separator] {
        let line = trim_cr(line);
        if matches!(line.first(), Some(b' ' | b'\t')) {
            if let Some(header) = headers.last_mut() {
                header.value.push(b' ');
                header.value.extend_from_slice(trim_ascii_start(line));
            }
            continue;
        }
        let colon = line.iter().position(|byte| *byte == b':')?;
        if colon == 0 {
            return None;
        }
        headers.push(HeaderField {
            name: line[..colon].to_vec(),
            value: trim_ascii_start(&line[colon + 1..]).to_vec(),
        });
    }
    let mut body = Vec::new();
    for (index, line) in lines[separator + 1..].iter().enumerate() {
        if index > 0 {
            body.push(b'\n');
        }
        body.extend_from_slice(trim_cr(line));
    }
    Some((headers, body))
}

fn split_multipart(body: &[u8], boundary: &[u8]) -> Option<Vec<Vec<u8>>> {
    if boundary.is_empty() || boundary.iter().any(|byte| *byte == b'\r' || *byte == b'\n') {
        return None;
    }
    let mut parts = Vec::new();
    let mut current = None::<Vec<u8>>;
    let marker = [b"--".as_slice(), boundary].concat();
    let closing_marker = [marker.as_slice(), b"--"].concat();
    for line in body.split(|byte| *byte == b'\n') {
        let line = trim_cr(line);
        if line == marker || line == closing_marker {
            if let Some(part) = current.take() {
                parts.push(part);
            }
            if line == closing_marker {
                return Some(parts);
            }
            current = Some(Vec::new());
        } else if let Some(part) = current.as_mut() {
            if !part.is_empty() {
                part.push(b'\n');
            }
            part.extend_from_slice(line);
        }
    }
    None
}

fn parse_content_type(value: &[u8]) -> (String, std::collections::BTreeMap<String, String>) {
    let mut segments = split_semicolon_parameters(value);
    let media_type = segments
        .next()
        .map(trim_ascii)
        .map(|value| String::from_utf8_lossy(value).to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "text/plain".to_owned());
    let mut parameters = std::collections::BTreeMap::new();
    for segment in segments {
        let Some(equals) = segment.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        let name = String::from_utf8_lossy(trim_ascii(&segment[..equals])).to_ascii_lowercase();
        let mut value = trim_ascii(&segment[equals + 1..]);
        if value.len() >= 2 && value.first() == Some(&b'"') && value.last() == Some(&b'"') {
            value = &value[1..value.len() - 1];
        }
        let value = unescape_quoted_parameter(value);
        parameters.insert(name, String::from_utf8_lossy(&value).into_owned());
    }
    (media_type, parameters)
}

fn split_semicolon_parameters(value: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut segments = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let mut escaped = false;
    for (index, byte) in value.iter().enumerate() {
        if escaped {
            escaped = false;
        } else if in_quotes && *byte == b'\\' {
            escaped = true;
        } else if *byte == b'"' {
            in_quotes = !in_quotes;
        } else if *byte == b';' && !in_quotes {
            segments.push(&value[start..index]);
            start = index + 1;
        }
    }
    segments.push(&value[start..]);
    segments.into_iter()
}

fn unescape_quoted_parameter(value: &[u8]) -> Vec<u8> {
    let mut unescaped = Vec::with_capacity(value.len());
    let mut escaped = false;
    for byte in value {
        if escaped {
            unescaped.push(*byte);
            escaped = false;
        } else if *byte == b'\\' {
            escaped = true;
        } else {
            unescaped.push(*byte);
        }
    }
    if escaped {
        unescaped.push(b'\\');
    }
    unescaped
}

fn header_value<'a>(headers: &'a [HeaderField], name: &[u8]) -> Option<&'a [u8]> {
    headers
        .iter()
        .rev()
        .find(|header| header.name.eq_ignore_ascii_case(name))
        .map(|header| header.value.as_slice())
}

fn join_lines(lines: &[Vec<u8>]) -> Vec<u8> {
    let capacity = lines.iter().map(Vec::len).sum::<usize>() + lines.len().saturating_sub(1) * 2;
    let mut joined = Vec::with_capacity(capacity);
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            joined.extend_from_slice(b"\r\n");
        }
        joined.extend_from_slice(line);
    }
    joined
}

fn normalize_line_endings(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n")
}

fn decode_utf8(value: &[u8]) -> String {
    UTF_8.decode(value).0.into_owned()
}

fn find_bytes(value: &[u8], needle: &[u8]) -> Option<usize> {
    value
        .windows(needle.len())
        .position(|window| window == needle)
}

fn is_mime_control_header(name: &[u8]) -> bool {
    name.eq_ignore_ascii_case(b"Content-Type")
        || name.eq_ignore_ascii_case(b"Content-Transfer-Encoding")
        || name.eq_ignore_ascii_case(b"Content-Disposition")
        || name.eq_ignore_ascii_case(b"MIME-Version")
        || name.eq_ignore_ascii_case(b"Message-ID")
        || name.eq_ignore_ascii_case(b"References")
        || name.eq_ignore_ascii_case(b"In-Reply-To")
}

fn trim_ascii(value: &[u8]) -> &[u8] {
    let start = value
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(value.len());
    let end = value
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(start, |index| index + 1);
    &value[start..end]
}

fn trim_ascii_start(value: &[u8]) -> &[u8] {
    let start = value
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t'))
        .unwrap_or(value.len());
    &value[start..]
}

fn trim_cr(value: &[u8]) -> &[u8] {
    value.strip_suffix(b"\r").unwrap_or(value)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn article(headers: &[(&[u8], &[u8])], body: &[u8]) -> RawArticle {
        RawArticle {
            headers: headers
                .iter()
                .map(|(name, value)| HeaderField {
                    name: name.to_vec(),
                    value: value.to_vec(),
                })
                .collect(),
            body: body
                .split(|byte| *byte == b'\n')
                .map(|line| trim_cr(line).to_vec())
                .collect(),
        }
    }

    #[test]
    fn decodes_utf8_text_and_normalizes_line_endings() {
        let decoded = decode_article(&article(
            &[(b"Content-Type", b"text/plain; charset=utf-8")],
            "hello\r\nworld\r!".as_bytes(),
        ));
        assert_eq!(decoded.body, "hello\nworld\n!");
        assert!(decoded.issues.is_empty());
    }

    #[test]
    fn decodes_base64_and_quoted_printable_bodies() {
        let base64 = decode_article(&article(
            &[
                (b"Content-Type", b"text/plain; charset=utf-8"),
                (b"Content-Transfer-Encoding", b"base64"),
            ],
            b"SGVsbG8gV29ybGQh",
        ));
        assert_eq!(base64.body, "Hello World!");
        assert!(base64.issues.is_empty());

        let quoted = decode_article(&article(
            &[
                (b"Content-Type", b"text/plain; charset=utf-8"),
                (b"Content-Transfer-Encoding", b"quoted-printable"),
            ],
            b"caf=C3=A9=\r\n!",
        ));
        assert_eq!(quoted.body, "caf\u{e9}!");
        assert!(quoted.issues.is_empty());
    }

    #[test]
    fn decodes_legacy_charsets_and_reports_unknown_or_invalid_data() {
        let latin = decode_article(&article(
            &[(b"Content-Type", b"text/plain; charset=windows-1252")],
            b"caf\xe9",
        ));
        assert_eq!(latin.body, "caf\u{e9}");
        assert!(latin.issues.is_empty());

        let unknown = decode_article(&article(
            &[(b"Content-Type", b"text/plain; charset=x-unknown")],
            b"plain",
        ));
        assert_eq!(unknown.body, "plain");
        assert!(unknown.issues.contains(&DecodeIssue::UnsupportedCharset {
            label: "x-unknown".to_owned()
        }));

        let invalid = decode_article(&article(
            &[(b"Content-Type", b"text/plain; charset=utf-8")],
            b"\xff",
        ));
        assert!(invalid.issues.contains(&DecodeIssue::InvalidCharsetData {
            label: "utf-8".to_owned()
        }));
    }

    #[test]
    fn decodes_rfc2047_b_and_q_words_and_suppresses_inter_word_space() {
        let decoded = decode_article(&article(
            &[
                (b"Subject", b"=?UTF-8?Q?caf=C3=A9?= =?UTF-8?B?IQ==?="),
                (b"From", b"=?UTF-8?B?SmFuZSBEb2U=?="),
            ],
            b"body",
        ));
        assert_eq!(decoded.headers[0].value, "caf\u{e9}!");
        assert_eq!(decoded.headers[1].value, "Jane Doe");
        assert!(decoded.issues.is_empty());
    }

    #[test]
    fn reports_malformed_encoded_words_and_keeps_the_literal_text() {
        let mut issues = Vec::new();
        let value = decode_encoded_words(b"Subject", b"bad =?UTF-8?Q?=ZZ?=", &mut issues);
        assert_eq!(value, "bad =?UTF-8?Q?=ZZ?=");
        assert_eq!(
            issues,
            vec![DecodeIssue::MalformedEncodedWord {
                header: "Subject".to_owned()
            }]
        );
    }

    #[test]
    fn an_unparseable_encoded_word_start_does_not_stop_later_decoding() {
        let mut issues = Vec::new();
        let value = decode_encoded_words(b"Subject", b"a =? b =?UTF-8?Q?x?=", &mut issues);
        assert_eq!(value, "a =? b x");
        assert_eq!(
            issues,
            vec![DecodeIssue::MalformedEncodedWord {
                header: "Subject".to_owned()
            }]
        );
    }

    #[test]
    fn reports_malformed_and_unsupported_transfer_encodings() {
        let malformed = decode_article(&article(
            &[
                (b"Content-Type", b"text/plain; charset=utf-8"),
                (b"Content-Transfer-Encoding", b"base64"),
            ],
            b"%%%=",
        ));
        assert!(malformed.issues.iter().any(|issue| matches!(
            issue,
            DecodeIssue::MalformedTransferEncoding { encoding } if encoding == "base64"
        )));

        let unsupported = decode_article(&article(
            &[
                (b"Content-Type", b"text/plain"),
                (b"Content-Transfer-Encoding", b"x-custom"),
            ],
            b"body",
        ));
        assert!(unsupported.issues.iter().any(|issue| matches!(
            issue,
            DecodeIssue::UnsupportedTransferEncoding { encoding } if encoding == "x-custom"
        )));

        let malformed_quoted = decode_article(&article(
            &[
                (b"Content-Type", b"text/plain; charset=utf-8"),
                (b"Content-Transfer-Encoding", b"quoted-printable"),
            ],
            b"invalid=QZ",
        ));
        assert!(malformed_quoted.issues.iter().any(|issue| matches!(
            issue,
            DecodeIssue::MalformedTransferEncoding { encoding } if encoding == "quoted-printable"
        )));
    }

    #[test]
    fn chooses_plain_text_from_multipart_alternative() {
        let decoded = decode_article(&article(
            &[
                (b"Content-Type", b"multipart/alternative; boundary=\"b-1\""),
            ],
            b"--b-1\r\nContent-Type: text/html\r\n\r\n<b>html</b>\r\n--b-1\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nplain text\r\n--b-1--",
        ));
        assert_eq!(decoded.body, "plain text");
        assert!(decoded.issues.is_empty());
    }

    /// A body of `levels` multiparts nested one inside the next, around a
    /// plain text part.
    fn nested_multipart(levels: usize) -> (String, String) {
        let mut body = "Content-Type: text/plain\r\n\r\ninnermost".to_owned();
        for level in (0..levels).rev() {
            body = format!("--b{level}\r\n{body}\r\n--b{level}--");
            if level > 0 {
                body =
                    format!("Content-Type: multipart/mixed; boundary=\"b{level}\"\r\n\r\n{body}");
            }
        }
        (format!("multipart/mixed; boundary=\"b0\""), body)
    }

    #[test]
    fn finds_plain_text_inside_nested_multiparts_up_to_the_depth_limit() {
        let mixed = decode_article(&article(
            &[(b"Content-Type", b"multipart/mixed; boundary=\"outer\"")],
            b"--outer\r\nContent-Type: multipart/alternative; boundary=\"inner\"\r\n\r\n\
              --inner\r\nContent-Type: text/html\r\n\r\n<p>html</p>\r\n\
              --inner\r\nContent-Type: text/plain\r\n\r\nnested plain\r\n--inner--\r\n\
              --outer\r\nContent-Type: application/octet-stream\r\n\r\nxyz\r\n--outer--",
        ));
        assert_eq!(mixed.body, "nested plain");
        assert!(mixed.issues.is_empty(), "{:?}", mixed.issues);

        let (content_type, body) = nested_multipart(16);
        let deepest = decode_article(&article(
            &[(b"Content-Type", content_type.as_bytes())],
            body.as_bytes(),
        ));
        assert_eq!(deepest.body, "innermost");
        assert!(deepest.issues.is_empty(), "{:?}", deepest.issues);

        let (content_type, body) = nested_multipart(17);
        let too_deep = decode_article(&article(
            &[(b"Content-Type", content_type.as_bytes())],
            body.as_bytes(),
        ));
        assert_eq!(too_deep.body, "");
        assert!(too_deep
            .issues
            .contains(&DecodeIssue::MultipartDepthExceeded));
    }

    #[test]
    fn multipart_without_a_plain_text_part_reports_it() {
        let decoded = decode_article(&article(
            &[(b"Content-Type", b"multipart/alternative; boundary=\"b\"")],
            b"--b\r\nContent-Type: text/html\r\n\r\n<p>only html</p>\r\n--b--",
        ));
        assert_eq!(decoded.body, "");
        assert_eq!(decoded.issues, [DecodeIssue::NoPlainTextPart]);
    }

    #[test]
    fn reports_unsupported_media_and_malformed_multipart() {
        let html = decode_article(&article(&[(b"Content-Type", b"text/html")], b"<b>html</b>"));
        assert!(html.issues.contains(&DecodeIssue::UnsupportedContentType {
            media_type: "text/html".to_owned()
        }));

        let malformed = decode_article(&article(
            &[(b"Content-Type", b"multipart/alternative")],
            b"no boundary",
        ));
        assert!(malformed.issues.contains(&DecodeIssue::MalformedMultipart));
    }
}
