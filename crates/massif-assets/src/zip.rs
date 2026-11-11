//! Just enough of the zip format to read a Minecraft jar: the central
//! directory, and entries stored or deflated.

use std::collections::HashMap;
use std::io::Read;

#[derive(Debug, thiserror::Error)]
pub enum ZipError {
    #[error("not a zip file")]
    NotZip,
    #[error("the zip file is damaged: {0}")]
    Damaged(&'static str),
    #[error("entry compressed with method {0}, which is not supported")]
    Method(u16),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    method: u16,
    compressed: u64,
    size: u64,
    header: u64,
}

/// A zip archive held in memory.
pub struct Archive {
    bytes: Vec<u8>,
    entries: HashMap<String, Entry>,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    b.get(at..at + 8).map(|s| u64::from_le_bytes(s.try_into().expect("8 bytes")))
}

impl Archive {
    pub fn open(path: &std::path::Path) -> Result<Self, ZipError> {
        Self::from_bytes(std::fs::read(path)?)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, ZipError> {
        // The end of central directory record is in the last 64 KiB.
        let start = bytes.len().saturating_sub(65_557);
        let eocd = (start..bytes.len().saturating_sub(21))
            .rev()
            .find(|&i| u32_at(&bytes, i) == Some(0x0605_4b50))
            .ok_or(ZipError::NotZip)?;
        let mut count = u16_at(&bytes, eocd + 10).ok_or(ZipError::NotZip)? as u64;
        let mut offset = u32_at(&bytes, eocd + 16).ok_or(ZipError::NotZip)? as u64;
        // Zip64: the real values are in the zip64 end record.
        if (count == 0xffff || offset == 0xffff_ffff) && eocd >= 20 && u32_at(&bytes, eocd - 20) == Some(0x0706_4b50) {
            let record = u64_at(&bytes, eocd - 12).ok_or(ZipError::Damaged("zip64 locator"))? as usize;
            if u32_at(&bytes, record) != Some(0x0606_4b50) {
                return Err(ZipError::Damaged("zip64 record"));
            }
            count = u64_at(&bytes, record + 32).ok_or(ZipError::Damaged("zip64 record"))?;
            offset = u64_at(&bytes, record + 48).ok_or(ZipError::Damaged("zip64 record"))?;
        }
        let mut entries = HashMap::with_capacity(count as usize);
        let mut at = offset as usize;
        for _ in 0..count {
            if u32_at(&bytes, at) != Some(0x0201_4b50) {
                return Err(ZipError::Damaged("central directory"));
            }
            let field = |o: usize| u32_at(&bytes, at + o).ok_or(ZipError::Damaged("central directory"));
            let method = u16_at(&bytes, at + 10).ok_or(ZipError::Damaged("central directory"))?;
            let mut compressed = field(20)? as u64;
            let mut size = field(24)? as u64;
            let name_len = u16_at(&bytes, at + 28).ok_or(ZipError::Damaged("central directory"))? as usize;
            let extra_len = u16_at(&bytes, at + 30).ok_or(ZipError::Damaged("central directory"))? as usize;
            let comment_len = u16_at(&bytes, at + 32).ok_or(ZipError::Damaged("central directory"))? as usize;
            let mut header = field(42)? as u64;
            let name = bytes.get(at + 46..at + 46 + name_len).ok_or(ZipError::Damaged("entry name"))?;
            let name = String::from_utf8_lossy(name).into_owned();
            // Zip64 sizes and offsets live in an extra field.
            let extra = bytes.get(at + 46 + name_len..at + 46 + name_len + extra_len).unwrap_or(&[]);
            let mut e = 0;
            while e + 4 <= extra.len() {
                let (id, len) = (u16_at(extra, e).unwrap_or(0), u16_at(extra, e + 2).unwrap_or(0) as usize);
                if id == 1 {
                    let mut p = e + 4;
                    let mut next = || {
                        let v = u64_at(extra, p);
                        p += 8;
                        v
                    };
                    if size == 0xffff_ffff {
                        size = next().unwrap_or(size);
                    }
                    if compressed == 0xffff_ffff {
                        compressed = next().unwrap_or(compressed);
                    }
                    if header == 0xffff_ffff {
                        header = next().unwrap_or(header);
                    }
                }
                e += 4 + len;
            }
            entries.insert(
                name,
                Entry {
                    method,
                    compressed,
                    size,
                    header,
                },
            );
            at += 46 + name_len + extra_len + comment_len;
        }
        Ok(Self { bytes, entries })
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    /// The bytes of an entry, inflated.
    pub fn read(&self, name: &str) -> Option<Result<Vec<u8>, ZipError>> {
        let e = *self.entries.get(name)?;
        Some(self.read_entry(e))
    }

    fn read_entry(&self, e: Entry) -> Result<Vec<u8>, ZipError> {
        let h = e.header as usize;
        if u32_at(&self.bytes, h) != Some(0x0403_4b50) {
            return Err(ZipError::Damaged("local header"));
        }
        let name_len = u16_at(&self.bytes, h + 26).ok_or(ZipError::Damaged("local header"))? as usize;
        let extra_len = u16_at(&self.bytes, h + 28).ok_or(ZipError::Damaged("local header"))? as usize;
        let data = h + 30 + name_len + extra_len;
        let raw = self
            .bytes
            .get(data..data + e.compressed as usize)
            .ok_or(ZipError::Damaged("entry data"))?;
        match e.method {
            0 => Ok(raw.to_vec()),
            8 => {
                let mut out = Vec::with_capacity(e.size as usize);
                flate2::read::DeflateDecoder::new(raw).read_to_end(&mut out)?;
                Ok(out)
            }
            m => Err(ZipError::Method(m)),
        }
    }
}

/// Zips written by hand, for tests.
#[cfg(test)]
pub mod tests_support {
    use std::io::Write;

    /// A zip of `(name, bytes, deflated)` entries.
    pub fn tiny_zip(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data, deflate) in files {
            let body = if *deflate {
                let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                enc.write_all(data).unwrap();
                enc.finish().unwrap()
            } else {
                data.to_vec()
            };
            let method: u16 = if *deflate { 8 } else { 0 };
            let header = out.len() as u32;
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            out.extend_from_slice(&[20, 0, 0, 0]);
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&[0; 8]); // time, date, crc
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&body);
            central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            central.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&[0; 8]);
            central.extend_from_slice(&(body.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0; 12]);
            central.extend_from_slice(&header.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let offset = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::tiny_zip;
    use super::*;

    #[test]
    fn stored_and_deflated_entries_read_back() {
        let text = b"hello hello hello hello, a jar is a zip";
        let zip = tiny_zip(&[("a.txt", b"stored", false), ("dir/b.txt", text, true)]);
        let a = Archive::from_bytes(zip).unwrap();
        assert_eq!(a.read("a.txt").unwrap().unwrap(), b"stored");
        assert_eq!(a.read("dir/b.txt").unwrap().unwrap(), text);
        assert!(a.read("missing").is_none());
        assert!(matches!(Archive::from_bytes(b"nope".to_vec()), Err(ZipError::NotZip)));
    }
}
