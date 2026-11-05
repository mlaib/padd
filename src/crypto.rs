//! Passphrase encryption using the age format (scrypt + ChaCha20-Poly1305),
//! ASCII-armored so encrypted files stay plain text. Files are compatible
//! with the `age` / `rage` command line tools.

use std::io::{Read, Write};

use age::secrecy::SecretString;
use zeroize::Zeroizing;

/// scrypt work factor (2^18): about 0.5–1 s on a laptop. Fixed rather than
/// auto-calibrated so a file made on a fast machine still opens quickly on a slow one.
const WORK_FACTOR: u8 = 18;
/// Refuse files that demand absurd work (denial-of-service protection).
const MAX_WORK_FACTOR: u8 = 22;

#[derive(Debug)]
pub enum CryptoError {
    WrongPassword,
    NotPassphrase,
    Corrupt(String),
}

impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CryptoError::WrongPassword => write!(f, "wrong password"),
            CryptoError::NotPassphrase => {
                write!(f, "file is encrypted to a key, not a password")
            }
            CryptoError::Corrupt(e) => write!(f, "cannot decrypt: {e}"),
        }
    }
}

pub fn encrypt(plain: &[u8], pass: &SecretString) -> Result<Vec<u8>, CryptoError> {
    let mut recipient = age::scrypt::Recipient::new(pass.clone());
    recipient.set_work_factor(WORK_FACTOR);
    let enc = age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
        .map_err(|e| CryptoError::Corrupt(e.to_string()))?;
    let io = |e: std::io::Error| CryptoError::Corrupt(e.to_string());
    let mut out = Vec::with_capacity(plain.len() * 4 / 3 + 512);
    let armor =
        age::armor::ArmoredWriter::wrap_output(&mut out, age::armor::Format::AsciiArmor).map_err(io)?;
    let mut w = enc.wrap_output(armor).map_err(io)?;
    w.write_all(plain).map_err(io)?;
    w.finish().and_then(|a| a.finish()).map_err(io)?;
    Ok(out)
}

pub fn decrypt(data: &[u8], pass: &SecretString) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    let reader = age::armor::ArmoredReader::new(data);
    let dec = age::Decryptor::new_buffered(reader).map_err(|e| CryptoError::Corrupt(e.to_string()))?;
    if !dec.is_scrypt() {
        return Err(CryptoError::NotPassphrase);
    }
    let mut id = age::scrypt::Identity::new(pass.clone());
    id.set_max_work_factor(MAX_WORK_FACTOR);
    let mut r = dec
        .decrypt(std::iter::once(&id as &dyn age::Identity))
        .map_err(|e| match e {
            age::DecryptError::DecryptionFailed | age::DecryptError::NoMatchingKeys => {
                CryptoError::WrongPassword
            }
            other => CryptoError::Corrupt(other.to_string()),
        })?;
    let mut out = Zeroizing::new(Vec::with_capacity(data.len()));
    r.read_to_end(&mut out)
        .map_err(|e| CryptoError::Corrupt(e.to_string()))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_wrong_password() {
        let pass = SecretString::from("correct horse".to_string());
        let ct = encrypt(b"api key::: sk-123\n", &pass).unwrap();
        assert!(ct.starts_with(b"-----BEGIN AGE ENCRYPTED FILE-----"));
        assert!(crate::fileio::is_age(&ct));
        let pt = decrypt(&ct, &pass).unwrap();
        assert_eq!(&pt[..], b"api key::: sk-123\n");
        let bad = SecretString::from("nope".to_string());
        assert!(matches!(decrypt(&ct, &bad), Err(CryptoError::WrongPassword)));
    }

    /// Writes an encrypted sample for manual testing:
    /// `PADD_SAMPLE=in.md cargo test -- --ignored write_sample` → in.md.enc (password "padd-demo")
    #[test]
    #[ignore]
    fn write_sample() {
        let src = std::env::var("PADD_SAMPLE").expect("PADD_SAMPLE");
        let pt = std::fs::read(&src).unwrap();
        let ct = encrypt(&pt, &SecretString::from("padd-demo".to_string())).unwrap();
        std::fs::write(format!("{src}.enc"), ct).unwrap();
    }

    /// Prints a decrypted sample: `PADD_SAMPLE=x.md.enc cargo test -- --ignored read_sample --nocapture`
    #[test]
    #[ignore]
    fn read_sample() {
        let src = std::env::var("PADD_SAMPLE").expect("PADD_SAMPLE");
        let pt = decrypt(
            &std::fs::read(&src).unwrap(),
            &SecretString::from("padd-demo".to_string()),
        )
        .unwrap();
        println!("{}", String::from_utf8_lossy(&pt));
    }

    #[test]
    #[ignore]
    fn time_decrypt() {
        let pass = SecretString::from("p".to_string());
        let ct = encrypt(b"x", &pass).unwrap();
        let t = std::time::Instant::now();
        decrypt(&ct, &pass).unwrap();
        println!("decrypt: {:?}", t.elapsed());
    }
}
