//! Minimal pure-Rust LZ4 Frame format decompressor for Rapfi `.bin.lz4` weights.
//!
//! Implements the standard LZ4 Frame specification (magic `0x184D2204`, blocks
//! with/without block checksums, independent/dependent blocks up to 4 MiB,
//! uncompressed blocks, optional content size / dict-id / content checksum).

use std::io;

const LZ4F_MAGIC: u32 = 0x184D_2204;
const LZ4F_SKIPPABLE_MIN: u32 = 0x184D_2A50;
const LZ4F_SKIPPABLE_MAX: u32 = 0x184D_2A5F;

fn read_u32_le(input: &[u8], pos: &mut usize) -> io::Result<u32> {
    if *pos + 4 > input.len() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "truncated lz4 u32",
        ));
    }
    let val = u32::from_le_bytes([
        input[*pos],
        input[*pos + 1],
        input[*pos + 2],
        input[*pos + 3],
    ]);
    *pos += 4;
    Ok(val)
}

fn read_u64_le(input: &[u8], pos: &mut usize) -> io::Result<u64> {
    if *pos + 8 > input.len() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "truncated lz4 u64",
        ));
    }
    let val = u64::from_le_bytes([
        input[*pos],
        input[*pos + 1],
        input[*pos + 2],
        input[*pos + 3],
        input[*pos + 4],
        input[*pos + 5],
        input[*pos + 6],
        input[*pos + 7],
    ]);
    *pos += 8;
    Ok(val)
}

/// Decompress one LZ4 raw block into `out`.
///
/// `out` may already contain preceding decoded bytes (used as history for
/// dependent blocks).
fn decompress_block(src: &[u8], out: &mut Vec<u8>) -> io::Result<()> {
    let mut i = 0_usize;
    let len = src.len();
    while i < len {
        let token = src[i];
        i += 1;

        // Literal length
        let mut lit_len = (token >> 4) as usize;
        if lit_len == 15 {
            loop {
                if i >= len {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "lz4 truncated literal length",
                    ));
                }
                let b = src[i] as usize;
                i += 1;
                lit_len += b;
                if b != 255 {
                    break;
                }
            }
        }
        if i + lit_len > len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "lz4 truncated literals",
            ));
        }
        out.extend_from_slice(&src[i..i + lit_len]);
        i += lit_len;

        // End of block: last sequence has no match part.
        if i >= len {
            break;
        }

        // Match offset (u16 LE, >= 1)
        if i + 2 > len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "lz4 truncated match offset",
            ));
        }
        let offset = u16::from_le_bytes([src[i], src[i + 1]]) as usize;
        i += 2;
        if offset == 0 || offset > out.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("lz4 invalid match offset {offset} at out_len {}", out.len()),
            ));
        }

        // Match length (+ 4 minmatch)
        let mut match_len = ((token & 0x0F) as usize) + 4;
        if (token & 0x0F) == 15 {
            loop {
                if i >= len {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "lz4 truncated match length",
                    ));
                }
                let b = src[i] as usize;
                i += 1;
                match_len += b;
                if b != 255 {
                    break;
                }
            }
        }

        let start = out.len() - offset;
        if offset >= match_len {
            out.reserve(match_len);
            // Non-overlapping slice copy
            let dst_start = out.len();
            out.resize(dst_start + match_len, 0);
            out.copy_within(start..start + match_len, dst_start);
        } else {
            out.reserve(match_len);
            for k in 0..match_len {
                let b = out[start + k];
                out.push(b);
            }
        }
    }
    Ok(())
}

/// Decompress an LZ4 Frame byte buffer into a `Vec<u8>`.
pub fn decompress_lz4_frame(input: &[u8]) -> io::Result<Vec<u8>> {
    let mut pos = 0_usize;
    let mut out = Vec::new();

    while pos < input.len() {
        let magic = read_u32_le(input, &mut pos)?;
        if (LZ4F_SKIPPABLE_MIN..=LZ4F_SKIPPABLE_MAX).contains(&magic) {
            let skip_len = read_u32_le(input, &mut pos)? as usize;
            if pos + skip_len > input.len() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "truncated lz4 skippable frame",
                ));
            }
            pos += skip_len;
            continue;
        }
        if magic != LZ4F_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid lz4 frame magic: 0x{magic:08x}"),
            ));
        }
        if pos + 2 > input.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "truncated lz4 frame header",
            ));
        }
        let flg = input[pos];
        let _bd = input[pos + 1];
        pos += 2;

        let version = (flg >> 6) & 0x03;
        if version != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported lz4 frame version {version}"),
            ));
        }
        let has_block_checksum = (flg & 0x10) != 0;
        let has_content_size = (flg & 0x08) != 0;
        let has_content_checksum = (flg & 0x04) != 0;
        let has_dict_id = (flg & 0x01) != 0;

        if has_content_size {
            let content_size = read_u64_le(input, &mut pos)? as usize;
            // Reserve up to 256 MiB sanely to avoid OOM on corrupt headers.
            out.reserve(content_size.min(256 * 1024 * 1024));
        }
        if has_dict_id {
            let _dict_id = read_u32_le(input, &mut pos)?;
        }
        // Header checksum byte
        if pos >= input.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "truncated lz4 header checksum",
            ));
        }
        pos += 1;

        // Data blocks
        loop {
            let block_header = read_u32_le(input, &mut pos)?;
            if block_header == 0 {
                break;
            }
            let uncompressed = (block_header & 0x8000_0000) != 0;
            let block_size = (block_header & 0x7FFF_FFFF) as usize;
            if pos + block_size > input.len() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "truncated lz4 block data",
                ));
            }
            let block_data = &input[pos..pos + block_size];
            pos += block_size;
            if uncompressed {
                out.extend_from_slice(block_data);
            } else {
                decompress_block(block_data, &mut out)?;
            }
            if has_block_checksum {
                let _checksum = read_u32_le(input, &mut pos)?;
            }
        }

        if has_content_checksum {
            let _content_checksum = read_u32_le(input, &mut pos)?;
        }
    }

    Ok(out)
}
