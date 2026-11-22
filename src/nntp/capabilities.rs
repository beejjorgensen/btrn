use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerCapabilities {
    entries: BTreeMap<String, Vec<Vec<u8>>>,
    pub invalid_lines: Vec<usize>,
}

impl ServerCapabilities {
    pub fn parse(lines: &[Vec<u8>]) -> Self {
        let mut entries = BTreeMap::new();
        let mut invalid_lines = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            let mut fields = line
                .split(|byte| byte.is_ascii_whitespace())
                .filter(|field| !field.is_empty());
            let Some(name) = fields.next() else {
                invalid_lines.push(index + 1);
                continue;
            };
            if !name.is_ascii() {
                invalid_lines.push(index + 1);
                continue;
            }
            let name = String::from_utf8(name.iter().map(u8::to_ascii_uppercase).collect())
                .expect("ASCII capability names are valid UTF-8");
            let arguments = fields.map(<[u8]>::to_vec).collect::<Vec<_>>();
            entries.insert(name, arguments);
        }
        Self {
            entries,
            invalid_lines,
        }
    }

    pub fn supports(&self, name: &str) -> bool {
        self.entries.contains_key(&name.to_ascii_uppercase())
    }

    pub fn supports_overview(&self) -> bool {
        self.supports("XOVER") || self.supports("OVER")
    }

    pub fn overview_command(&self) -> Option<&'static str> {
        if self.supports("XOVER") {
            Some("XOVER")
        } else if self.supports("OVER") {
            Some("OVER")
        } else {
            None
        }
    }

    #[cfg(test)]
    pub fn arguments(&self, name: &str) -> Option<&[Vec<u8>]> {
        self.entries
            .get(&name.to_ascii_uppercase())
            .map(Vec::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_capabilities_case_insensitively_and_preserves_arguments() {
        let caps = ServerCapabilities::parse(&[
            b"reader".to_vec(),
            b"OVER".to_vec(),
            b"AUTHINFO USER SASL".to_vec(),
            b"X-EXTENSION opaque value".to_vec(),
            b"".to_vec(),
            b"\xff".to_vec(),
        ]);
        assert!(caps.supports("READER"));
        assert!(caps.supports("over"));
        assert_eq!(
            caps.arguments("authinfo"),
            Some([b"USER".to_vec(), b"SASL".to_vec()].as_slice())
        );
        assert_eq!(caps.arguments("x-extension").unwrap()[0], b"opaque");
        assert_eq!(caps.invalid_lines, [5, 6]);
    }

    #[test]
    fn prefers_xover_then_over() {
        let xover = ServerCapabilities::parse(&[b"OVER".to_vec(), b"XOVER".to_vec()]);
        assert_eq!(xover.overview_command(), Some("XOVER"));
        let over = ServerCapabilities::parse(&[b"OVER".to_vec()]);
        assert_eq!(over.overview_command(), Some("OVER"));
        assert_eq!(
            ServerCapabilities::parse(&[b"READER".to_vec()]).overview_command(),
            None
        );
    }
}
