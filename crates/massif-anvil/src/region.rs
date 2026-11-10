//! Anvil region files (`r.X.Z.mca`): 32×32 chunks in 4 KiB sectors.
//!
//! ```text
//! sector 0   1024 × u32 BE: (first sector << 8) | sector count
//! sector 1   1024 × u32 BE: last-modified time, seconds
//! sector 2+  per chunk: u32 BE length · u8 compression (2 = zlib) · data,
//!            padded to a whole number of sectors
//! ```

use flate2::Compression;
use flate2::write::ZlibEncoder;
use std::io::{self, Write};

const SECTOR: usize = 4096;
/// Chunks per region edge.
pub const REGION_CHUNKS: usize = 32;
const COMPRESSION_ZLIB: u8 = 2;

/// Collects the compressed chunks of one region, then writes the file.
pub struct RegionBuilder {
    /// Complete chunk records (length, compression type, data), by
    /// `x + z * 32`.
    chunks: Vec<Option<Vec<u8>>>,
    count: usize,
}

impl Default for RegionBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl RegionBuilder {
    pub fn new() -> Self {
        Self {
            chunks: vec![None; REGION_CHUNKS * REGION_CHUNKS],
            count: 0,
        }
    }

    /// Compress and store the chunk at region-local position `(x, z)`.
    pub fn put(&mut self, x: usize, z: usize, nbt: &[u8], level: u32) -> io::Result<()> {
        assert!(x < REGION_CHUNKS && z < REGION_CHUNKS);
        let mut record = Vec::with_capacity(nbt.len() / 6 + 64);
        record.extend_from_slice(&[0, 0, 0, 0, COMPRESSION_ZLIB]);
        let mut enc = ZlibEncoder::new(record, Compression::new(level));
        enc.write_all(nbt)?;
        let mut record = enc.finish()?;
        let len = (record.len() - 4) as u32; // compression byte + data
        record[..4].copy_from_slice(&len.to_be_bytes());
        let slot = &mut self.chunks[x + z * REGION_CHUNKS];
        if slot.is_none() {
            self.count += 1;
        }
        *slot = Some(record);
        Ok(())
    }

    pub fn chunk_count(&self) -> usize {
        self.count
    }

    /// Write the region file. Returns its size in bytes.
    pub fn write<W: Write>(&self, mut out: W, timestamp: u32) -> io::Result<u64> {
        let mut locations = [0u8; SECTOR];
        let mut timestamps = [0u8; SECTOR];
        let mut sector = 2u32;
        for (i, chunk) in self.chunks.iter().enumerate() {
            let Some(record) = chunk else { continue };
            let sectors = record.len().div_ceil(SECTOR) as u32;
            if sectors > 255 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "a chunk is larger than the 1 MiB a region file can hold",
                ));
            }
            locations[i * 4..i * 4 + 4].copy_from_slice(&((sector << 8) | sectors).to_be_bytes());
            timestamps[i * 4..i * 4 + 4].copy_from_slice(&timestamp.to_be_bytes());
            sector += sectors;
        }
        out.write_all(&locations)?;
        out.write_all(&timestamps)?;
        let padding = [0u8; SECTOR];
        for record in self.chunks.iter().flatten() {
            out.write_all(record)?;
            let rem = record.len() % SECTOR;
            if rem != 0 {
                out.write_all(&padding[..SECTOR - rem])?;
            }
        }
        out.flush()?;
        Ok(sector as u64 * SECTOR as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbt::NbtWriter;
    use std::io::Cursor;

    fn chunk_nbt(tag: i32, filler: usize) -> Vec<u8> {
        let mut w = NbtWriter::new();
        w.int("tag", tag);
        // Incompressible filler so that some chunks span several sectors.
        let mut s = 0x9E37_79B9u32;
        let bytes: Vec<u8> = (0..filler)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                s as u8
            })
            .collect();
        w.byte_array("filler", &bytes);
        w.finish()
    }

    #[test]
    fn regions_are_readable_by_an_independent_reader() {
        let mut region = RegionBuilder::new();
        let chunks = [(0usize, 0usize, 7, 10), (31, 31, 8, 9000), (5, 17, 9, 20_000), (6, 17, 10, 0)];
        for &(x, z, tag, filler) in &chunks {
            region.put(x, z, &chunk_nbt(tag, filler), 3).unwrap();
        }
        assert_eq!(region.chunk_count(), 4);
        // Replacing a chunk does not count twice.
        region.put(6, 17, &chunk_nbt(10, 0), 3).unwrap();
        assert_eq!(region.chunk_count(), 4);

        let mut file = Vec::new();
        let size = region.write(&mut file, 1_700_000_000).unwrap();
        assert_eq!(size, file.len() as u64);
        assert_eq!(file.len() % SECTOR, 0);

        let mut reader = fastanvil::Region::from_stream(Cursor::new(file)).unwrap();
        for &(x, z, tag, filler) in &chunks {
            let data = reader.read_chunk(x, z).unwrap().expect("chunk present");
            assert_eq!(data, chunk_nbt(tag, filler), "chunk ({x}, {z})");
        }
        assert!(reader.read_chunk(1, 1).unwrap().is_none());
    }

    #[test]
    fn an_empty_region_is_just_a_header() {
        let mut file = Vec::new();
        assert_eq!(RegionBuilder::new().write(&mut file, 0).unwrap(), 8192);
        assert!(file.iter().all(|&b| b == 0));
    }
}
