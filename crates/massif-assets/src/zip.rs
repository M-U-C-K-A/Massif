//! Just enough of the zip format to read a Minecraft jar: the central
//! directory, and entries stored or deflated.
//!
//! A jar on disk is read where it lies, an entry at a time: a mod's jar can
//! be a quarter of a gigabyte (Conquest Reforged's is 222 MB) of which a
//! catalogue reads a part.

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

/// Where an archive's bytes are.
enum Data {
    Memory(Vec<u8>),
    File(std::fs::File),
}

impl Data {
    fn len(&self) -> Result<u64, ZipError> {
        Ok(match self {
            Data::Memory(b) => b.len() as u64,
            Data::File(f) => f.metadata()?.len(),
        })
    }

    /// `len` bytes from `at`, from any thread.
    fn read_at(&self, at: u64, len: usize) -> Result<Vec<u8>, ZipError> {
        match self {
            Data::Memory(b) => {
                let at = usize::try_from(at).map_err(|_| ZipError::Damaged("offset"))?;
                b.get(at..at.checked_add(len).ok_or(ZipError::Damaged("offset"))?).map(<[u8]>::to_vec).ok_or(ZipError::Damaged("offset"))
            }
            Data::File(f) => {
                let mut out = vec![0u8; len];
                let mut done = 0;
                while done < len {
                    #[cfg(unix)]
                    let n = std::os::unix::fs::FileExt::read_at(f, &mut out[done..], at + done as u64)?;
                    #[cfg(windows)]
                    let n = std::os::windows::fs::FileExt::seek_read(f, &mut out[done..], at + done as u64)?;
                    if n == 0 {
                        return Err(ZipError::Damaged("entry data"));
                    }
                    done += n;
                }
                Ok(out)
            }
        }
    }
}

/// A zip archive, in memory or read from its file as it goes; or a folder
/// read as one (a resource pack left unzipped, KubeJS's `kubejs/assets`).
pub struct Archive {
    data: Data,
    entries: HashMap<String, Entry>,
    /// For a folder: its files, by their names in it.
    folder: Option<HashMap<String, std::path::PathBuf>>,
    /// Files made in memory over the others (the block states and models
    /// of the blocks KubeJS's scripts make), by their names.
    extra: HashMap<String, Vec<u8>>,
}

/// The most files a folder read as an archive may list.
const MAX_FOLDER_FILES: usize = 1 << 20;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    b.get(at..at + 8).map(|s| u64::from_le_bytes(s.try_into().expect("8 bytes")))
}

/// The central directory of an archive: its bytes and how many entries it
/// says it has.
fn directory(data: &Data) -> Result<(Vec<u8>, u64), ZipError> {
    let len = data.len()?;
    // The end of central directory record is in the last 64 KiB.
    let tail_len = len.min(65_557);
    let tail = data.read_at(len - tail_len, tail_len as usize)?;
    let eocd = (0..tail.len().saturating_sub(21)).rev().find(|&i| u32_at(&tail, i) == Some(0x0605_4b50)).ok_or(ZipError::NotZip)?;
    let mut count = u16_at(&tail, eocd + 10).ok_or(ZipError::NotZip)? as u64;
    let mut size = u32_at(&tail, eocd + 12).ok_or(ZipError::NotZip)? as u64;
    let mut offset = u32_at(&tail, eocd + 16).ok_or(ZipError::NotZip)? as u64;
    // Zip64: the real values are in the zip64 end record.
    let at = len - tail_len + eocd as u64;
    if (count == 0xffff || size == 0xffff_ffff || offset == 0xffff_ffff) && at >= 20 {
        let locator = data.read_at(at - 20, 20)?;
        if u32_at(&locator, 0) == Some(0x0706_4b50) {
            let record_at = u64_at(&locator, 8).ok_or(ZipError::Damaged("zip64 locator"))?;
            let record = data.read_at(record_at, 56)?;
            if u32_at(&record, 0) != Some(0x0606_4b50) {
                return Err(ZipError::Damaged("zip64 record"));
            }
            count = u64_at(&record, 32).ok_or(ZipError::Damaged("zip64 record"))?;
            size = u64_at(&record, 40).ok_or(ZipError::Damaged("zip64 record"))?;
            offset = u64_at(&record, 48).ok_or(ZipError::Damaged("zip64 record"))?;
        }
    }
    if offset.checked_add(size).is_none_or(|end| end > len) || size > 1 << 31 {
        return Err(ZipError::Damaged("central directory"));
    }
    Ok((data.read_at(offset, size as usize)?, count))
}

/// The entries of a central directory, with their names.
fn entries(dir: &[u8], count: u64) -> Result<Vec<(String, Entry)>, ZipError> {
    let mut out = Vec::with_capacity(count.min(1 << 20) as usize);
    let mut at = 0;
    for _ in 0..count {
        if u32_at(dir, at) != Some(0x0201_4b50) {
            return Err(ZipError::Damaged("central directory"));
        }
        let field = |o: usize| u32_at(dir, at + o).ok_or(ZipError::Damaged("central directory"));
        let short = |o: usize| u16_at(dir, at + o).ok_or(ZipError::Damaged("central directory"));
        let method = short(10)?;
        let mut compressed = field(20)? as u64;
        let mut size = field(24)? as u64;
        let (name_len, extra_len, comment_len) = (short(28)? as usize, short(30)? as usize, short(32)? as usize);
        let mut header = field(42)? as u64;
        let name = dir.get(at + 46..at + 46 + name_len).ok_or(ZipError::Damaged("entry name"))?;
        let name = String::from_utf8_lossy(name).into_owned();
        // Zip64 sizes and offsets live in an extra field.
        let extra = dir.get(at + 46 + name_len..at + 46 + name_len + extra_len).unwrap_or(&[]);
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
        out.push((name, Entry { method, compressed, size, header }));
        at += 46 + name_len + extra_len + comment_len;
    }
    Ok(out)
}

impl Archive {
    /// An archive on disk, read as it goes: only its directory is read now.
    pub fn open(path: &std::path::Path) -> Result<Self, ZipError> {
        Self::with(Data::File(std::fs::File::open(path)?))
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, ZipError> {
        Self::with(Data::Memory(bytes))
    }

    fn with(data: Data) -> Result<Self, ZipError> {
        let (dir, count) = directory(&data)?;
        Ok(Self { entries: entries(&dir, count)?.into_iter().collect(), data, folder: None, extra: HashMap::new() })
    }

    /// A folder read as an archive: the files under its subfolders `under`
    /// (`assets`), named from the folder with `/` between their parts, as a
    /// zip's entries are. Nothing is read until asked for; links to other
    /// folders are not followed.
    pub fn folder(root: &std::path::Path, under: &[&str]) -> Result<Self, ZipError> {
        let mut files = HashMap::new();
        let mut stack: Vec<std::path::PathBuf> = under.iter().map(|u| root.join(u)).filter(|d| d.is_dir()).collect();
        if stack.is_empty() {
            return Err(ZipError::NotZip);
        }
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir)?.flatten() {
                let Ok(kind) = e.file_type() else { continue };
                let path = e.path();
                if kind.is_dir() {
                    stack.push(path);
                } else if kind.is_file()
                    && let Ok(rel) = path.strip_prefix(root)
                {
                    let name = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
                    files.insert(name, path);
                    if files.len() > MAX_FOLDER_FILES {
                        return Err(ZipError::Damaged("too many files"));
                    }
                }
            }
        }
        Ok(Self { data: Data::Memory(Vec::new()), entries: HashMap::new(), folder: Some(files), extra: HashMap::new() })
    }

    /// The names of a zip file's entries, reading only its directory: to
    /// look into a large jar without reading all of it.
    pub fn list(path: &std::path::Path) -> Result<Vec<String>, ZipError> {
        let data = Data::File(std::fs::File::open(path)?);
        let (dir, count) = directory(&data)?;
        Ok(entries(&dir, count)?.into_iter().map(|(n, _)| n).collect())
    }

    /// An archive of files made in memory alone.
    pub fn of_files(files: Vec<(String, Vec<u8>)>) -> Self {
        Self { data: Data::Memory(Vec::new()), entries: HashMap::new(), folder: Some(HashMap::new()), extra: files.into_iter().collect() }
    }

    /// The archive with files made in memory added, over its own of the
    /// same names.
    pub fn with_files(mut self, files: Vec<(String, Vec<u8>)>) -> Self {
        self.extra.extend(files);
        self
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        let extra = &self.extra;
        self.entries
            .keys()
            .chain(self.folder.iter().flat_map(HashMap::keys))
            .filter(move |n| !extra.contains_key(*n))
            .chain(extra.keys())
            .map(String::as_str)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.extra.contains_key(name) || self.entries.contains_key(name) || self.folder.as_ref().is_some_and(|f| f.contains_key(name))
    }

    /// The bytes of an entry, inflated.
    pub fn read(&self, name: &str) -> Option<Result<Vec<u8>, ZipError>> {
        if let Some(bytes) = self.extra.get(name) {
            return Some(Ok(bytes.clone()));
        }
        if let Some(files) = &self.folder {
            return files.get(name).map(|p| std::fs::read(p).map_err(ZipError::from));
        }
        let e = *self.entries.get(name)?;
        Some(self.read_entry(e))
    }

    fn read_entry(&self, e: Entry) -> Result<Vec<u8>, ZipError> {
        let local = self.data.read_at(e.header, 30)?;
        if u32_at(&local, 0) != Some(0x0403_4b50) {
            return Err(ZipError::Damaged("local header"));
        }
        let name_len = u16_at(&local, 26).ok_or(ZipError::Damaged("local header"))? as u64;
        let extra_len = u16_at(&local, 28).ok_or(ZipError::Damaged("local header"))? as u64;
        if e.compressed > 1 << 31 || e.size > 1 << 31 {
            return Err(ZipError::Damaged("entry size"));
        }
        let raw = self.data.read_at(e.header + 30 + name_len + extra_len, e.compressed as usize)?;
        match e.method {
            0 => Ok(raw),
            8 => {
                let mut out = Vec::with_capacity(e.size as usize);
                flate2::read::DeflateDecoder::new(&raw[..]).read_to_end(&mut out)?;
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
        // Its directory alone, from the file.
        let path = std::env::temp_dir().join(format!("massif-zip-list-{}.zip", std::process::id()));
        std::fs::write(&path, tiny_zip(&[("a.txt", b"stored", false), ("dir/b.txt", text, true)])).unwrap();
        assert_eq!(Archive::list(&path).unwrap(), ["a.txt", "dir/b.txt"]);
        std::fs::write(&path, b"nope").unwrap();
        assert!(Archive::list(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
