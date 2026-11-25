//! Region files (`r.X.Z.mca`): 32×32 chunks, each a compressed NBT record
//! in sectors of 4 KiB, found through the 4 KiB table at the head of the
//! file. A chunk too large for the file (over 1 MiB) lies beside it in
//! `c.X.Z.mcc`, its record saying so with the high bit of its compression.

use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// A region file found in a folder, by its region coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionFile {
    pub x: i32,
    pub z: i32,
    pub path: PathBuf,
}

/// The region coordinates of a file named `r.X.Z.mca`.
pub fn region_coords(path: &Path) -> Option<(i32, i32)> {
    let name = path.file_name()?.to_str()?;
    let rest = name.strip_prefix("r.")?.strip_suffix(".mca")?;
    let (x, z) = rest.split_once('.')?;
    Some((x.parse().ok()?, z.parse().ok()?))
}

/// The region files of a folder (macOS's `._` shadows left out), sorted.
pub fn region_files(dir: &Path) -> Vec<RegionFile> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<RegionFile> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("._")))
        .filter_map(|p| region_coords(&p).map(|(x, z)| RegionFile { x, z, path: p }))
        .collect();
    out.sort_by_key(|r| (r.z, r.x));
    out
}

/// Which of the 1024 chunks a region file holds, read from its table
/// alone (`(z * 32 + x)`).
pub fn presence(path: &Path) -> io::Result<Box<[bool; 1024]>> {
    let mut head = [0u8; 4096];
    let mut file = std::fs::File::open(path)?;
    let mut read = 0;
    while read < head.len() {
        let n = file.read(&mut head[read..])?;
        if n == 0 {
            break;
        }
        read += n;
    }
    let mut out = Box::new([false; 1024]);
    if read < 4096 {
        return Ok(out);
    }
    for (i, p) in out.iter_mut().enumerate() {
        let e = &head[i * 4..i * 4 + 4];
        *p = e[3] != 0 && (e[0] as u32) << 16 | (e[1] as u32) << 8 | e[2] as u32 >= 2;
    }
    Ok(out)
}

/// A region file read into memory.
pub struct Region {
    path: PathBuf,
    data: Vec<u8>,
    rx: i32,
    rz: i32,
}

impl Region {
    pub fn open(file: &RegionFile) -> io::Result<Region> {
        let data = std::fs::read(&file.path)?;
        Ok(Region { path: file.path.clone(), data, rx: file.x, rz: file.z })
    }

    /// Whether chunk `(x, z)` (within the region, 0..32) is in the file.
    pub fn has(&self, x: usize, z: usize) -> bool {
        self.location(x, z).is_some()
    }

    fn location(&self, x: usize, z: usize) -> Option<(usize, usize)> {
        let i = (z * 32 + x) * 4;
        let e = self.data.get(i..i + 4)?;
        let offset = ((e[0] as usize) << 16 | (e[1] as usize) << 8 | e[2] as usize) * 4096;
        let sectors = e[3] as usize;
        (sectors > 0 && offset >= 8192).then_some((offset, sectors))
    }

    /// The NBT of chunk `(x, z)`, uncompressed; `None` where it is not in
    /// the file.
    pub fn chunk(&self, x: usize, z: usize) -> io::Result<Option<Vec<u8>>> {
        let Some((offset, _)) = self.location(x, z) else { return Ok(None) };
        let head = self.data.get(offset..offset + 5).ok_or_else(|| io::Error::other("chunk past the end of the file"))?;
        let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        let kind = head[4];
        if kind & 0x80 != 0 {
            let name = format!("c.{}.{}.mcc", self.rx * 32 + x as i32, self.rz * 32 + z as i32);
            let outside = std::fs::read(self.path.with_file_name(name))?;
            return decompress(kind & 0x7f, &outside).map(Some);
        }
        if len == 0 {
            return Ok(None);
        }
        let body = self.data.get(offset + 5..offset + 4 + len).ok_or_else(|| io::Error::other("chunk cut short"))?;
        decompress(kind, body).map(Some)
    }
}

/// A chunk's record, uncompressed: 1 gzip, 2 zlib, 3 none, 4 LZ4 (from
/// 24w04a).
pub fn decompress(kind: u8, body: &[u8]) -> io::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(body.len() * 4);
    match kind {
        1 => {
            flate2::read::GzDecoder::new(body).read_to_end(&mut out)?;
        }
        2 => {
            flate2::read::ZlibDecoder::new(body).read_to_end(&mut out)?;
        }
        3 => out.extend_from_slice(body),
        4 => {
            lz4_java_wrc::Lz4BlockInput::new(body).read_to_end(&mut out)?;
        }
        k => return Err(io::Error::other(format!("unknown chunk compression {k}"))),
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_names() {
        assert_eq!(region_coords(Path::new("/w/region/r.-1.3.mca")), Some((-1, 3)));
        assert_eq!(region_coords(Path::new("r.0.0.mcr")), None);
        assert_eq!(region_coords(Path::new("c.0.0.mcc")), None);
    }

    #[test]
    fn reads_what_the_exporter_writes() {
        let mut b = massif_anvil::region::RegionBuilder::new();
        b.put(3, 5, b"hello nbt", 6).unwrap();
        let mut out = Vec::new();
        b.write(&mut out, 0).unwrap();
        let dir = std::env::temp_dir().join(format!("massif-region-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("r.2.-1.mca");
        std::fs::write(&path, &out).unwrap();
        let file = RegionFile { x: 2, z: -1, path: path.clone() };
        let r = Region::open(&file).unwrap();
        assert!(r.has(3, 5) && !r.has(5, 3));
        assert_eq!(r.chunk(3, 5).unwrap().unwrap(), b"hello nbt");
        assert!(presence(&path).unwrap()[5 * 32 + 3]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
