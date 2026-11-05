//! Loading and saving text files: text-only check, encodings, line endings,
//! atomic writes.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// How a file was encoded on disk, so that saving writes it back the same way.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Encoding {
    #[default]
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Format {
    pub encoding: Encoding,
    pub crlf: bool,
}

pub enum Loaded {
    /// Plain text, normalised to `\n` line endings.
    Text(String, Format),
    /// An age-encrypted file (armored or binary); needs a passphrase.
    Encrypted(Vec<u8>),
}

#[derive(Debug)]
pub enum LoadError {
    Binary,
    Io(io::Error),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Binary => write!(f, "not a text file (binary content)"),
            LoadError::Io(e) => write!(f, "{e}"),
        }
    }
}

const AGE_ARMOR: &[u8] = b"-----BEGIN AGE ENCRYPTED FILE-----";
const AGE_BINARY: &[u8] = b"age-encryption.org/v1";
const SNIFF: usize = 8 * 1024;

pub fn is_age(bytes: &[u8]) -> bool {
    let start = bytes.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(0);
    let b = &bytes[start..];
    b.starts_with(AGE_ARMOR) || b.starts_with(AGE_BINARY)
}

pub fn load(path: &Path) -> Result<Loaded, LoadError> {
    let bytes = fs::read(path).map_err(LoadError::Io)?;
    if is_age(&bytes) {
        return Ok(Loaded::Encrypted(bytes));
    }
    let (text, fmt) = decode(&bytes)?;
    Ok(Loaded::Text(text, fmt))
}

/// Decode bytes as text, refusing anything that looks binary.
pub fn decode(bytes: &[u8]) -> Result<(String, Format), LoadError> {
    let (raw, encoding) = if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        (utf8(rest)?, Encoding::Utf8Bom)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        (utf16(rest, u16::from_le_bytes)?, Encoding::Utf16Le)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        (utf16(rest, u16::from_be_bytes)?, Encoding::Utf16Be)
    } else {
        if bytes[..bytes.len().min(SNIFF)].contains(&0) {
            return Err(LoadError::Binary);
        }
        (utf8(bytes)?, Encoding::Utf8)
    };
    if has_binary_controls(&raw) {
        return Err(LoadError::Binary);
    }
    let crlf = raw.contains("\r\n");
    let text = if crlf { raw.replace("\r\n", "\n") } else { raw };
    Ok((text, Format { encoding, crlf }))
}

fn utf8(b: &[u8]) -> Result<String, LoadError> {
    String::from_utf8(b.to_vec()).map_err(|_| LoadError::Binary)
}

fn utf16(b: &[u8], f: fn([u8; 2]) -> u16) -> Result<String, LoadError> {
    if !b.len().is_multiple_of(2) {
        return Err(LoadError::Binary);
    }
    let units = b.as_chunks::<2>().0.iter().map(|c| f([c[0], c[1]]));
    char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .map_err(|_| LoadError::Binary)
}

/// NUL or many C0 control characters (other than tab/newline/CR/formfeed/ESC)
/// in the head of the file means it is not human-readable text.
fn has_binary_controls(s: &str) -> bool {
    let head = &s.as_bytes()[..s.len().min(SNIFF)];
    let bad = head
        .iter()
        .filter(|&&b| b < 0x20 && !matches!(b, b'\t' | b'\n' | b'\r' | 0x0C | 0x1B))
        .count();
    head.contains(&0) || bad * 100 > head.len().max(1)
}

/// Encode text for disk according to its original format.
pub fn encode(text: &str, fmt: Format) -> Vec<u8> {
    let text = if fmt.crlf {
        std::borrow::Cow::Owned(text.replace('\n', "\r\n"))
    } else {
        std::borrow::Cow::Borrowed(text)
    };
    match fmt.encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Utf8Bom => [&[0xEF, 0xBB, 0xBF][..], text.as_bytes()].concat(),
        Encoding::Utf16Le => std::iter::once(0xFEFFu16)
            .chain(text.encode_utf16())
            .flat_map(u16::to_le_bytes)
            .collect(),
        Encoding::Utf16Be => std::iter::once(0xFEFFu16)
            .chain(text.encode_utf16())
            .flat_map(u16::to_be_bytes)
            .collect(),
    }
}

/// Write via a temp file in the same directory, then rename over the target,
/// so a crash never leaves a half-written file. Keeps unix permissions.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp: PathBuf = dir.join(format!(".{name}.padd-tmp"));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    #[cfg(unix)]
    if let Ok(meta) = fs::metadata(path) {
        let _ = fs::set_permissions(&tmp, meta.permissions());
    }
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_binary() {
        assert!(matches!(decode(b"PNG\0\x01\x02"), Err(LoadError::Binary)));
        assert!(matches!(decode(&[0xC3, 0x28, b'a']), Err(LoadError::Binary)));
    }

    #[test]
    fn roundtrips_formats() {
        for enc in [
            Encoding::Utf8,
            Encoding::Utf8Bom,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
        ] {
            for crlf in [false, true] {
                let fmt = Format { encoding: enc, crlf };
                let bytes = encode("héllo\nwörld ✓\n", fmt);
                let (text, got) = decode(&bytes).unwrap();
                assert_eq!(text, "héllo\nwörld ✓\n");
                assert_eq!(got, fmt);
            }
        }
    }

    #[test]
    fn detects_age() {
        assert!(is_age(b"\n-----BEGIN AGE ENCRYPTED FILE-----\nabc"));
        assert!(!is_age(b"# hello"));
    }
}
