//! Pseudonymisation: replace identifiers (bank account numbers, PAN, mobile numbers, customer
//! IDs...) with surrogate tokens, and turn the tokens back into the originals later.
//!
//! - A token is a keyed hash (HMAC-SHA-256) of the value: the same value always gets the same
//!   token, so tables can still be joined and counted, but without the key nobody can work out the
//!   value from the token, not even by trying every possible mobile number. A plain SHA-256 hash
//!   would not be safe for this: there are only 10 billion mobile numbers.
//! - The originals are kept in a vault (a SQLite file), encrypted with AES-256-GCM. Only someone
//!   with the key can turn tokens back into values.
//! - The key lives in Windows Credential Manager (a secret name, default `pseudonym_key`); it is
//!   created on first use. Back it up: without it the vault cannot be read. A passphrase stored
//!   with `autoflow secret set` works too (use a long one), so several PCs can share a vault.

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{anyhow, bail, Context as _, Result};
use base64::Engine as _;
use hmac::{Hmac, Mac};
use rusqlite::OptionalExtension as _;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;

type HmacSha256 = Hmac<Sha256>;

fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut m = <HmacSha256 as Mac>::new_from_slice(key).expect("any key length");
    for p in parts {
        m.update(p);
    }
    m.finalize().into_bytes().into()
}

/// The 32-byte key from a stored secret: a base64 key made by AutoFlowMate, or any passphrase.
pub fn key_from_secret(secret: &str) -> [u8; 32] {
    let s = secret.trim();
    if let Ok(b) = base64::engine::general_purpose::STANDARD.decode(s) {
        if b.len() == 32 {
            let mut k = [0u8; 32];
            k.copy_from_slice(&b);
            return k;
        }
    }
    Sha256::digest(s.as_bytes()).into()
}

/// A new random key, as text to store in Credential Manager.
pub fn new_key_text() -> String {
    let key = Aes256Gcm::generate_key(&mut OsRng);
    base64::engine::general_purpose::STANDARD.encode(key)
}

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

fn base32(bytes: &[u8], chars: usize) -> String {
    let mut out = String::with_capacity(chars);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &b in bytes {
        // Enough characters: stop before the bit buffer can grow past 32 bits.
        if out.len() >= chars {
            break;
        }
        acc = (acc << 8) | b as u32;
        bits += 8;
        while bits >= 5 && out.len() < chars {
            bits -= 5;
            out.push(CROCKFORD[((acc >> bits) & 31) as usize] as char);
        }
        acc &= (1 << bits) - 1;
    }
    out
}

/// Tidy a value before making its token, so "1234 5678-90" and "1234567890" match.
pub fn normalize(v: &str) -> String {
    v.chars().filter(|c| !c.is_whitespace() && *c != '-').flat_map(char::to_uppercase).collect()
}

pub struct Vault {
    conn: rusqlite::Connection,
    token_key: [u8; 32],
    cipher: Aes256Gcm,
    cache: HashMap<String, String>,
    dry: bool,
    in_tx: bool,
}

impl Vault {
    /// Open (or, with `create`, make) a vault. Fails if it was made with a different key.
    /// In a dry run nothing is written.
    pub fn open(path: &Path, key: &[u8; 32], create: bool, dry: bool) -> Result<Self> {
        let exists = path.exists();
        if !exists && !create {
            bail!("vault {} not found", path.display());
        }
        let conn = if dry && !exists {
            rusqlite::Connection::open_in_memory()?
        } else {
            if let Some(d) = path.parent() {
                std::fs::create_dir_all(d).ok();
            }
            let flags = if dry { rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY } else { rusqlite::OpenFlags::default() };
            rusqlite::Connection::open_with_flags(path, flags).with_context(|| format!("cannot open vault {}", path.display()))?
        };
        conn.busy_timeout(std::time::Duration::from_secs(30))?;
        if !(dry && exists) {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS meta (name TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE IF NOT EXISTS pseudonyms (token TEXT PRIMARY KEY, domain TEXT NOT NULL, nonce BLOB NOT NULL, value BLOB NOT NULL, created TEXT NOT NULL);",
            )?;
        }
        let token_key = hmac(key, &[b"autoflowmate:token"]);
        let enc_key = hmac(key, &[b"autoflowmate:encrypt"]);
        let check = hex(&hmac(key, &[b"autoflowmate:check"])[..16]);
        let stored: Option<String> = conn
            .query_row("SELECT value FROM meta WHERE name = 'key_check'", [], |r| r.get(0))
            .optional()
            .with_context(|| format!("{} is not a readable AutoFlowMate vault", path.display()))?;
        match stored {
            Some(s) if s != check => bail!("vault {} was made with a different key: use the same key_secret as when it was made", path.display()),
            Some(_) => {}
            None if !(dry && exists) => {
                conn.execute("INSERT INTO meta (name, value) VALUES ('key_check', ?1)", [&check])?;
                conn.execute("INSERT OR IGNORE INTO meta (name, value) VALUES ('made_by', 'AutoFlowMate')", [])?;
            }
            None => {}
        }
        let cipher = Aes256Gcm::new_from_slice(&enc_key).map_err(|e| anyhow!("{e}"))?;
        Ok(Self { conn, token_key, cipher, cache: HashMap::new(), dry, in_tx: false })
    }

    fn begin(&mut self) -> Result<()> {
        if !self.in_tx && !self.dry {
            self.conn.execute_batch("BEGIN")?;
            self.in_tx = true;
        }
        Ok(())
    }

    /// Save everything written since the last commit.
    pub fn commit(&mut self) -> Result<()> {
        if self.in_tx {
            self.conn.execute_batch("COMMIT")?;
            self.in_tx = false;
        }
        Ok(())
    }

    /// The token for a value (and remember the value in the vault).
    pub fn token(&mut self, domain: &str, prefix: &str, value: &str) -> Result<String> {
        let mac = hmac(&self.token_key, &[domain.as_bytes(), &[0], value.as_bytes()]);
        let token = format!("{prefix}{}", base32(&mac, 16));
        if self.cache.contains_key(&token) {
            return Ok(token);
        }
        let known: Option<String> =
            self.conn.query_row("SELECT domain FROM pseudonyms WHERE token = ?1", [&token], |r| r.get(0)).optional().context("vault")?;
        if known.is_none() && !self.dry {
            self.begin()?;
            let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
            let ct = self.cipher.encrypt(&nonce, value.as_bytes()).map_err(|e| anyhow!("encrypt: {e}"))?;
            let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
            self.conn.execute(
                "INSERT OR IGNORE INTO pseudonyms (token, domain, nonce, value, created) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![token, domain, nonce.as_slice(), ct, now],
            )?;
        }
        self.cache.insert(token.clone(), value.to_string());
        Ok(token)
    }

    /// The original value of a token, if the vault has it.
    pub fn reveal(&mut self, token: &str) -> Result<Option<String>> {
        let t = token.trim();
        if let Some(v) = self.cache.get(t) {
            return Ok(Some(v.clone()));
        }
        let row: Option<(Vec<u8>, Vec<u8>)> =
            self.conn.query_row("SELECT nonce, value FROM pseudonyms WHERE token = ?1", [t], |r| Ok((r.get(0)?, r.get(1)?))).optional().context("vault")?;
        let Some((nonce, ct)) = row else { return Ok(None) };
        if nonce.len() != 12 {
            bail!("vault entry for {t} is damaged");
        }
        let pt = self.cipher.decrypt(Nonce::from_slice(&nonce), ct.as_slice()).map_err(|_| anyhow!("cannot decrypt {t}: wrong key or damaged vault"))?;
        let v = String::from_utf8(pt).map_err(|_| anyhow!("vault entry for {t} is damaged"))?;
        self.cache.insert(t.to_string(), v.clone());
        Ok(Some(v))
    }

    /// Number of values in the vault.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> Result<usize> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM pseudonyms", [], |r| r.get::<_, i64>(0)).context("vault")? as usize)
    }
}

impl Drop for Vault {
    /// Anything not committed is undone: a step that fails half-way leaves the vault as it was.
    fn drop(&mut self) {
        if self.in_tx {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_round_trip_and_wrong_key() {
        let dir = std::env::temp_dir().join(format!("afm_pseudo_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("vault.db");
        let key = key_from_secret(&new_key_text());
        let (t1, t2, t3);
        {
            let mut v = Vault::open(&path, &key, true, false).unwrap();
            t1 = v.token("", "ACC-", "001234567890").unwrap();
            t2 = v.token("", "ACC-", "001234567890").unwrap();
            t3 = v.token("pan", "ACC-", "001234567890").unwrap();
            v.commit().unwrap();
            assert_eq!(v.len().unwrap(), 2);
        }
        assert_eq!(t1, t2, "same value, same token");
        assert_ne!(t1, t3, "other domain, other token");
        assert!(t1.starts_with("ACC-") && t1.len() == 20 && !t1.contains("1234567890"));
        let mut v = Vault::open(&path, &key, false, false).unwrap();
        assert_eq!(v.reveal(&t1).unwrap().as_deref(), Some("001234567890"));
        assert_eq!(v.reveal("ACC-NOTATOKEN").unwrap(), None);
        drop(v);
        // The vault file holds no plain values.
        let raw = std::fs::read(&path).unwrap();
        assert!(!raw.windows(12).any(|w| w == b"001234567890"));
        let other = key_from_secret("a different passphrase");
        assert!(Vault::open(&path, &other, false, false).is_err());
        // Passphrases give stable keys.
        assert_eq!(key_from_secret("x y z"), key_from_secret(" x y z "));
        assert_eq!(normalize(" ab-12 34 "), "AB1234");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_step_leaves_vault_unchanged() {
        let path = std::env::temp_dir().join(format!("afm_pseudo_rb_{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let key = key_from_secret("rollback test");
        {
            let mut v = Vault::open(&path, &key, true, false).unwrap();
            v.token("", "", "first").unwrap();
            v.commit().unwrap();
        }
        {
            // A step that fails half-way: tokens made, then dropped without commit.
            let mut v = Vault::open(&path, &key, true, false).unwrap();
            v.token("", "", "second").unwrap();
            v.token("", "", "third").unwrap();
        }
        let v = Vault::open(&path, &key, false, false).unwrap();
        assert_eq!(v.len().unwrap(), 1, "uncommitted values must be rolled back");
        drop(v);
        // A file that is not a vault is an error, not "no value".
        let bogus = std::env::temp_dir().join(format!("afm_pseudo_bogus_{}.db", std::process::id()));
        std::fs::write(&bogus, b"not a database at all, just some text that is long enough").unwrap();
        assert!(Vault::open(&bogus, &key, false, false).is_err());
        let _ = std::fs::remove_file(&bogus);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn dry_run_writes_nothing() {
        let path = std::env::temp_dir().join(format!("afm_pseudo_dry_{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let key = key_from_secret("k");
        let mut v = Vault::open(&path, &key, true, true).unwrap();
        let t = v.token("", "", "9876543210").unwrap();
        assert_eq!(v.reveal(&t).unwrap().as_deref(), Some("9876543210"));
        drop(v);
        assert!(!path.exists());
    }
}
