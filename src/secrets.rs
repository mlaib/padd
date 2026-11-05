//! The secrets syntax: `label::: value`.
//!
//! A line that *starts* with `:::` is a Pandoc/Docusaurus directive, not a secret.

use std::ops::Range;

pub const SEP: &str = ":::";

pub struct Span {
    pub label: Range<usize>,
    pub sep: Range<usize>,
    pub value: Range<usize>,
}

/// Parse one line (may include its trailing newline). Ranges are relative to `line`.
pub fn parse_line(line: &str) -> Option<Span> {
    let content = line.trim_end_matches(['\n', '\r']);
    let lead = content.len() - content.trim_start().len();
    if content[lead..].starts_with(SEP) {
        return None;
    }
    let at = content.find(SEP)?;
    let label_end = content[..at].trim_end().len();
    // Allow list items: "- key::: value" → label is "key".
    let body = &content[lead..label_end];
    let skip = ["- ", "* ", "+ "]
        .iter()
        .find(|p| body.starts_with(*p))
        .map_or(0, |p| p.len());
    let label = lead + skip..label_end;
    if label.is_empty() {
        return None;
    }
    let after = at + SEP.len();
    let vstart = after + (content[after..].len() - content[after..].trim_start().len());
    let vend = content.trim_end().len().max(vstart);
    Some(Span {
        label,
        sep: at..after,
        value: vstart..vend,
    })
}

/// Template for File ▸ New secrets file.
pub const TEMPLATE: &str = "# My secrets\n\n\
Each line written as `label::: value` is hidden and gets *show* and *copy* buttons.\n\n\
## Example service\n\
url: https://example.com\n\
user: me@example.com\n\
password::: change-me\n\
api key::: sk-xxxxxxxxxxxxxxxx\n";

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(l: &str) -> Option<(&str, &str)> {
        parse_line(l).map(|s| (&l[s.label], &l[s.value]))
    }

    #[test]
    fn parses() {
        assert_eq!(parts("api key::: sk-1 \n"), Some(("api key", "sk-1")));
        assert_eq!(parts("- token :::ghp_x"), Some(("token", "ghp_x")));
        assert_eq!(parts("  pw:::  a b c  "), Some(("pw", "a b c")));
        assert_eq!(parts("::: note"), None);
        assert_eq!(parts("   :::warning"), None);
        assert_eq!(parts("no secret: here"), None);
        assert_eq!(parts("empty:::"), Some(("empty", "")));
    }
}
