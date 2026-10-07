//! UnityFS asset bundles: a header, a (usually compressed) table of blocks
//! and files, then the compressed blocks. Decompressed, the blocks form one
//! byte stream that the files ("nodes") are slices of.

use crate::reader::Reader;
use crate::{Error, ErrorKind, Result};

/// Header flag bits.
const COMPRESSION_MASK: u32 = 0x3F;
const BLOCKS_INFO_AT_END: u32 = 0x80;
const BLOCK_INFO_NEEDS_PADDING: u32 = 0x200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    None,
    Lzma,
    Lz4,
    Lz4Hc,
}

impl Compression {
    fn from_flags(flags: u32) -> Option<Self> {
        match flags & COMPRESSION_MASK {
            0 => Some(Self::None),
            1 => Some(Self::Lzma),
            2 => Some(Self::Lz4),
            3 => Some(Self::Lz4Hc),
            _ => None,
        }
    }
}

/// One file inside a bundle: a serialized file (`CAB-…`) or a resource
/// blob (`CAB-….resS`, `.resource`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleNode {
    pub path: String,
    pub offset: u64,
    pub size: u64,
    pub flags: u32,
}

impl BundleNode {
    /// Flag bit 2 marks serialized files.
    pub fn is_serialized_file(&self) -> bool {
        self.flags & 4 != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockInfo {
    pub uncompressed_size: u32,
    pub compressed_size: u32,
    pub flags: u16,
}

#[derive(Debug)]
pub struct Bundle {
    pub format: u32,
    pub player_version: String,
    pub unity_version: String,
    pub flags: u32,
    pub blocks: Vec<BlockInfo>,
    pub nodes: Vec<BundleNode>,
    data: Vec<u8>,
}

/// Refuses to allocate more than this for one bundle's decompressed data.
const MAX_UNCOMPRESSED: u64 = 4 << 30;

fn decompress(compression: Compression, src: &[u8], size: usize, at: usize) -> Result<Vec<u8>> {
    let fail = |message: String| Error {
        offset: at,
        kind: ErrorKind::Decompress(message),
    };
    match compression {
        Compression::None => {
            if src.len() != size {
                return Err(fail(format!(
                    "stored block is {} bytes, expected {size}",
                    src.len()
                )));
            }
            Ok(src.to_vec())
        }
        Compression::Lz4 | Compression::Lz4Hc => {
            let mut out = vec![0; size];
            let n =
                lz4_flex::block::decompress_into(src, &mut out).map_err(|e| fail(e.to_string()))?;
            if n != size {
                return Err(fail(format!("LZ4 produced {n} bytes, expected {size}")));
            }
            Ok(out)
        }
        Compression::Lzma => Err(Error {
            offset: at,
            kind: ErrorKind::Unsupported("LZMA-compressed bundle data".into()),
        }),
    }
}

/// A bundle's header and directory: everything before the data blocks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleDirectory {
    pub format: u32,
    pub player_version: String,
    pub unity_version: String,
    pub flags: u32,
    pub blocks: Vec<BlockInfo>,
    pub nodes: Vec<BundleNode>,
    /// Where the first data block starts in the file.
    pub data_start: usize,
}

impl BundleDirectory {
    /// Reads the header and directory. For bundles whose directory follows
    /// the header (all of Subnautica's), a prefix of the file is enough.
    pub fn parse(bytes: &[u8]) -> Result<BundleDirectory> {
        let mut r = Reader::new(bytes, true);
        let signature = r.cstr()?;
        if signature != "UnityFS" {
            return Err(Error {
                offset: 0,
                kind: ErrorKind::BadMagic(signature),
            });
        }
        let format = r.u32()?;
        let player_version = r.cstr()?;
        let unity_version = r.cstr()?;
        let _size = r.i64()?;
        let compressed_info = r.u32()? as usize;
        let uncompressed_info = r.u32()? as usize;
        let flags = r.u32()?;
        if !(6..=8).contains(&format) {
            return Err(r.error(ErrorKind::Unsupported(format!("UnityFS format {format}"))));
        }
        if format >= 7 {
            r.align(16)?;
        }

        let info_at = if flags & BLOCKS_INFO_AT_END != 0 {
            bytes
                .len()
                .checked_sub(compressed_info)
                .ok_or_else(|| r.error(ErrorKind::Invalid("blocks info larger than file".into())))?
        } else {
            r.pos()
        };
        let mut info_reader = Reader::new(bytes, true);
        info_reader.seek(info_at)?;
        let compressed = info_reader.bytes(compressed_info)?;
        if flags & BLOCKS_INFO_AT_END == 0 {
            r.seek(info_at + compressed_info)?;
        }
        let compression = Compression::from_flags(flags)
            .ok_or_else(|| r.error(ErrorKind::Unsupported(format!("compression {flags:#x}"))))?;
        let info = decompress(compression, compressed, uncompressed_info, info_at)?;

        let mut ir = Reader::new(&info, true);
        let _hash: [u8; 16] = ir.array()?;
        let block_count = ir.count(10)?;
        let mut blocks = Vec::with_capacity(block_count);
        for _ in 0..block_count {
            blocks.push(BlockInfo {
                uncompressed_size: ir.u32()?,
                compressed_size: ir.u32()?,
                flags: ir.u16()?,
            });
        }
        let node_count = ir.count(21)?;
        let mut nodes = Vec::with_capacity(node_count);
        for _ in 0..node_count {
            let offset = ir.i64()?;
            let size = ir.i64()?;
            let node_flags = ir.u32()?;
            let path = ir.cstr()?;
            if offset < 0 || size < 0 {
                return Err(ir.error(ErrorKind::Invalid(format!(
                    "node {path} has negative extent"
                ))));
            }
            nodes.push(BundleNode {
                path,
                offset: offset as u64,
                size: size as u64,
                flags: node_flags,
            });
        }

        if flags & BLOCK_INFO_NEEDS_PADDING != 0 {
            r.align(16)?;
        }
        Ok(BundleDirectory {
            format,
            player_version,
            unity_version,
            flags,
            blocks,
            nodes,
            data_start: r.pos(),
        })
    }
}

impl Bundle {
    pub fn parse(bytes: &[u8]) -> Result<Bundle> {
        let dir = BundleDirectory::parse(bytes)?;
        let mut r = Reader::new(bytes, true);
        r.seek(dir.data_start)?;
        let BundleDirectory {
            format,
            player_version,
            unity_version,
            flags,
            blocks,
            nodes,
            ..
        } = dir;
        let total: u64 = blocks.iter().map(|b| u64::from(b.uncompressed_size)).sum();
        if total > MAX_UNCOMPRESSED {
            return Err(r.error(ErrorKind::Invalid(format!("{total} bytes uncompressed"))));
        }
        let mut data = Vec::with_capacity(total as usize);
        for block in &blocks {
            let at = r.pos();
            let src = r.bytes(block.compressed_size as usize)?;
            let kind = Compression::from_flags(u32::from(block.flags)).ok_or_else(|| {
                r.error(ErrorKind::Unsupported(format!(
                    "block compression {:#x}",
                    block.flags
                )))
            })?;
            data.extend(decompress(kind, src, block.uncompressed_size as usize, at)?);
        }
        for node in &nodes {
            if node.offset.saturating_add(node.size) > data.len() as u64 {
                return Err(Error {
                    offset: 0,
                    kind: ErrorKind::Invalid(format!("node {} runs past the data", node.path)),
                });
            }
        }
        Ok(Bundle {
            format,
            player_version,
            unity_version,
            flags,
            blocks,
            nodes,
            data,
        })
    }

    /// The bytes of one node (bounds were checked in `parse`).
    /// Size of the decompressed data held in memory.
    pub fn data_len(&self) -> usize {
        self.data.len()
    }

    pub fn node_data(&self, node: &BundleNode) -> &[u8] {
        &self.data[node.offset as usize..(node.offset + node.size) as usize]
    }
}

/// Builds a bundle in memory. Only for tests and tools: lets tests run on
/// synthetic data instead of game files.
pub fn write_bundle(nodes: &[(&str, u32, &[u8])], unity_version: &str, lz4: bool) -> Vec<u8> {
    let be32 = |v: u32| v.to_be_bytes();
    let mut data = Vec::new();
    let mut node_table = Vec::new();
    for (path, flags, bytes) in nodes {
        node_table.extend((data.len() as i64).to_be_bytes());
        node_table.extend((bytes.len() as i64).to_be_bytes());
        node_table.extend(be32(*flags));
        node_table.extend(path.as_bytes());
        node_table.push(0);
        data.extend_from_slice(bytes);
    }
    let (block, block_flags) = if lz4 {
        (lz4_flex::block::compress(&data), 2u16)
    } else {
        (data.clone(), 0)
    };
    let mut info = vec![0u8; 16];
    info.extend(1i32.to_be_bytes());
    info.extend(be32(data.len() as u32));
    info.extend(be32(block.len() as u32));
    info.extend(block_flags.to_be_bytes());
    info.extend((nodes.len() as i32).to_be_bytes());
    info.extend(node_table);
    let info_compressed = if lz4 {
        lz4_flex::block::compress(&info)
    } else {
        info.clone()
    };

    let mut out = Vec::new();
    out.extend(b"UnityFS\0");
    out.extend(be32(7));
    out.extend(b"5.x.x\0");
    out.extend(unity_version.as_bytes());
    out.push(0);
    out.extend(0i64.to_be_bytes()); // total size, not checked by readers
    out.extend(be32(info_compressed.len() as u32));
    out.extend(be32(info.len() as u32));
    out.extend(be32(if lz4 { 0x42 } else { 0x40 }));
    while out.len() % 16 != 0 {
        out.push(0);
    }
    out.extend(info_compressed);
    out.extend(block);
    out
}
