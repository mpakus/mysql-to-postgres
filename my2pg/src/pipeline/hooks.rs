use crate::config::MAX_SQL_HOOK_BYTES;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Read},
    path::Path,
};

pub fn read_sql(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(MAX_SQL_HOOK_BYTES.min(64 * 1024));
    File::open(path)?
        .take(MAX_SQL_HOOK_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_SQL_HOOK_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "SQL hook exceeds the configured size limit",
        ));
    }
    Ok(bytes)
}

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
