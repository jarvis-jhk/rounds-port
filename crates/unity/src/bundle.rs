//! Unity asset bundles (`UnityFS`, as written by Unity 2022.3 Addressables)
//! and the SerializedFiles inside them, which embed their own type trees.
//!
//! Container layout (big-endian header):
//!
//! * `"UnityFS\0"`, `u32` format version (6..=8), Unity version and revision
//!   (C strings), `i64` total size, `u32` compressed and uncompressed size of
//!   the block info, `u32` flags (bits 0-5 compression of the block info,
//!   `0x80` block info at the end of the file, `0x200` data padded to 16);
//! * format >= 7: align to 16 before the block info;
//! * block info (compressed per the flags): 16-byte hash, `i32` block count,
//!   per block `u32` uncompressed size, `u32` compressed size, `u16` flags
//!   (bits 0-5 compression), then `i32` node count, per node `i64` offset,
//!   `i64` size, `u32` flags, path (C string);
//! * the data blocks; their concatenated decompressed bytes hold the nodes.
//!
//! Compression: 0 none, 2/3 LZ4 / LZ4HC (block format). LZMA (1) is not
//! supported (the game's bundles use LZ4).

use std::sync::Arc;

use crate::bytes::Reader;
use crate::serialized::SerializedFile;
use crate::typetree::{RefTypes, TypeTree};
use crate::value::Value;
use crate::{Error, Result};

/// One file inside a bundle.
#[derive(Clone, Debug)]
pub struct BundleEntry {
    /// Path inside the bundle (e.g. `CAB-<hash>`, `CAB-<hash>.resS`).
    pub path: String,
    /// Node flags (4 = SerializedFile).
    pub flags: u32,
    /// Decompressed bytes.
    pub data: Vec<u8>,
}

/// Decompresses one LZ4 block (no frame header) of known output size.
pub fn lz4_block(src: &[u8], out_len: usize) -> Result<Vec<u8>> {
    let bad = || Error::Format("corrupt LZ4 block".into());
    let mut out = Vec::with_capacity(out_len);
    let mut i = 0;
    while i < src.len() {
        let token = src[i];
        i += 1;
        let mut lit = (token >> 4) as usize;
        if lit == 15 {
            loop {
                let b = *src.get(i).ok_or_else(bad)?;
                i += 1;
                lit += b as usize;
                if b != 255 {
                    break;
                }
            }
        }
        out.extend_from_slice(src.get(i..i + lit).ok_or_else(bad)?);
        i += lit;
        if i >= src.len() {
            break; // the last sequence has literals only
        }
        let off = u16::from_le_bytes([src[i], *src.get(i + 1).ok_or_else(bad)?]) as usize;
        i += 2;
        let mut len = (token & 15) as usize;
        if len == 15 {
            loop {
                let b = *src.get(i).ok_or_else(bad)?;
                i += 1;
                len += b as usize;
                if b != 255 {
                    break;
                }
            }
        }
        len += 4;
        if off == 0 || off > out.len() {
            return Err(bad());
        }
        let start = out.len() - off;
        for k in 0..len {
            let b = out[start + k];
            out.push(b);
        }
        if out.len() > out_len {
            return Err(bad());
        }
    }
    if out.len() != out_len {
        return Err(Error::Format(format!(
            "LZ4 block: {} bytes, expected {out_len}",
            out.len()
        )));
    }
    Ok(out)
}

fn decompress(kind: u32, src: &[u8], out_len: usize) -> Result<Vec<u8>> {
    match kind {
        0 => Ok(src.to_vec()),
        2 | 3 => lz4_block(src, out_len),
        k => Err(Error::Unsupported(format!("bundle compression {k}"))),
    }
}

/// Reads a `UnityFS` bundle into its files.
pub fn read_bundle(data: &[u8]) -> Result<Vec<BundleEntry>> {
    let mut r = Reader::new(data);
    r.set_big_endian(true);
    let sig = r.cstr()?;
    if sig != "UnityFS" {
        return Err(Error::Unsupported(format!("bundle signature {sig:?}")));
    }
    let version = r.u32()?;
    let _unity = r.cstr()?;
    let _revision = r.cstr()?;
    let _size = r.i64()?;
    let info_c = r.u32()? as usize;
    let info_u = r.u32()? as usize;
    let flags = r.u32()?;
    if version >= 7 {
        let p = r.pos().next_multiple_of(16);
        r.seek(p)?;
    }
    let info_raw = if flags & 0x80 != 0 {
        data.get(data.len().saturating_sub(info_c)..)
            .ok_or_else(|| Error::Format("bundle block info".into()))?
    } else {
        r.take(info_c)?
    };
    let info = decompress(flags & 0x3f, info_raw, info_u)?;
    if flags & 0x200 != 0 {
        let p = r.pos().next_multiple_of(16);
        r.seek(p)?;
    }
    let mut ir = Reader::new(&info);
    ir.set_big_endian(true);
    ir.take(16)?;
    let nb = ir.len_prefix(10)?;
    let mut blocks = Vec::with_capacity(nb);
    for _ in 0..nb {
        let u = ir.u32()? as usize;
        let c = ir.u32()? as usize;
        let f = ir.u16()? as u32;
        blocks.push((u, c, f));
    }
    let mut stream = Vec::with_capacity(blocks.iter().map(|b| b.0).sum());
    for (u, c, f) in blocks {
        stream.extend_from_slice(&decompress(f & 0x3f, r.take(c)?, u)?);
    }
    let nn = ir.len_prefix(21)?;
    let mut out = Vec::with_capacity(nn);
    for _ in 0..nn {
        let off = ir.i64()? as usize;
        let size = ir.i64()? as usize;
        let flags = ir.u32()?;
        let path = ir.cstr()?;
        let bytes = stream
            .get(off..off + size)
            .ok_or_else(|| Error::Format(format!("bundle node {path} out of range")))?;
        out.push(BundleEntry {
            path,
            flags,
            data: bytes.to_vec(),
        });
    }
    Ok(out)
}

/// An object read from a SerializedFile with embedded type trees.
#[derive(Clone, Debug)]
pub struct EmbeddedObject {
    /// Path id.
    pub path_id: i64,
    /// Unity class id (114 = MonoBehaviour).
    pub class_id: i32,
    /// The decoded fields (header included).
    pub value: Value,
}

struct EmbeddedRefs(Vec<(String, String, String, Arc<TypeTree>)>);

impl RefTypes for EmbeddedRefs {
    fn ref_type(&self, assembly: &str, namespace: &str, class: &str) -> Option<&TypeTree> {
        self.0
            .iter()
            .find(|(a, n, c, _)| a == assembly && n == namespace && c == class)
            .map(|(_, _, _, t)| &**t)
    }
}

/// Reads every object of a SerializedFile that embeds its type trees
/// (asset bundles). Objects whose type has no tree are skipped; objects
/// that fail to decode are returned as errors in the second list.
pub fn read_embedded(data: &[u8]) -> Result<(Vec<EmbeddedObject>, Vec<String>)> {
    let sf = SerializedFile::parse(data)?;
    if !sf.has_type_trees {
        return Err(Error::Unsupported("file has no embedded type trees".into()));
    }
    let mut trees: Vec<Option<TypeTree>> = Vec::with_capacity(sf.types.len());
    for t in &sf.types {
        trees.push(match &t.nodes {
            Some(n) if !n.is_empty() => Some(TypeTree::new(n.clone())?),
            _ => None,
        });
    }
    let mut refs = Vec::new();
    for t in &sf.ref_types {
        if let Some(n) = t.nodes.as_ref().filter(|n| !n.is_empty()) {
            refs.push((
                t.assembly.clone(),
                t.namespace.clone(),
                t.class_name.clone(),
                Arc::new(TypeTree::new(n.clone())?),
            ));
        }
    }
    let refs = EmbeddedRefs(refs);
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for o in &sf.objects {
        let Some(tree) = trees.get(o.type_index as usize).and_then(|t| t.as_ref()) else {
            continue;
        };
        let bytes = &data[o.byte_start as usize..][..o.byte_size as usize];
        let mut r = Reader::new(bytes);
        r.set_big_endian(sf.big_endian);
        match tree.read_with(&mut r, &refs) {
            Ok(value) => out.push(EmbeddedObject {
                path_id: o.path_id,
                class_id: o.class_id,
                value,
            }),
            Err(e) => errors.push(format!("object {}: {e}", o.path_id)),
        }
    }
    Ok((out, errors))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lz4_literals_and_matches() {
        // "abcabcabcX": literals "abc", match offset 3 length 6, literal "X".
        let src = [0x32, b'a', b'b', b'c', 3, 0, 0x10, b'X'];
        assert_eq!(lz4_block(&src, 10).unwrap(), b"abcabcabcX");
        assert!(lz4_block(&src, 9).is_err());
        assert!(lz4_block(&[0x0f, 9, 0], 4).is_err());
    }
}
