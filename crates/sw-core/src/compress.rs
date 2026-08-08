//! `.Z` (Unix compress / LZW) decompression.
//!
//! Implements the ncompress variant used by SGI distributions: a 3-byte
//! header (`1F 9D` magic plus a flag byte), LSB-first bit packing, block
//! mode with `CLEAR` codes, and code width transitions from 9 up to
//! `maxbits` bits.
use crate::error::{Error, Result};

/// The two magic bytes starting a `.Z` stream.
pub const MAGIC: [u8; 2] = [0x1F, 0x9D];

/// Whether `data` starts with the `.Z` magic.
pub fn is_compressed(data: &[u8]) -> bool {
    data.len() >= 3 && data[..2] == MAGIC
}

/// Decompresses a complete `.Z` stream.
///
/// # Errors
///
/// Returns [`Error::Compress`] if the stream is not a valid `.Z` stream or
/// uses an unsupported bit width.
pub fn decompress(data: &[u8]) -> Result<Vec<u8>> {
    let corrupt = || Error::Compress {
        message: "corrupt .Z stream".to_string(),
    };

    if !is_compressed(data) {
        return Err(Error::Compress {
            message: "not a .Z stream".to_string(),
        });
    }

    let flags = data[2];
    let max_bits = (flags & 0x1F) as u32;
    let block_mode = flags & 0x80 != 0;
    if !(9..=16).contains(&max_bits) {
        return Err(Error::Compress {
            message: format!("unsupported .Z maxbits {max_bits}"),
        });
    }

    const CLEAR_CODE: u32 = 256;
    const FIRST_CODE: u32 = 257;
    let max_max_code = 1u32 << max_bits;

    let mut n_bits: u32 = 9;
    let mut max_code: u32 = (1 << n_bits) - 1;
    let mut free_ent: u32 = if block_mode { FIRST_CODE } else { 256 };
    let mut clear_pending = false;

    let mut reader = CodeReader::new(&data[3..]);

    let mut prefix = vec![0u32; max_max_code as usize];
    let mut suffix = vec![0u8; max_max_code as usize];
    let mut stack = vec![0u8; max_max_code as usize];
    for (i, byte) in suffix.iter_mut().take(256).enumerate() {
        *byte = i as u8;
    }

    let mut output = Vec::new();
    let mut old_code: Option<u32> = None;
    let mut fin_char = 0u8;

    while let Some(code) = reader.next_code(
        max_bits,
        max_max_code,
        &mut n_bits,
        &mut max_code,
        free_ent,
        &mut clear_pending,
    ) {
        if block_mode && code == CLEAR_CODE {
            clear_pending = true;
            free_ent = FIRST_CODE;
            old_code = None;
            continue;
        }

        let Some(old) = old_code else {
            if code > 255 {
                return Err(corrupt());
            }
            fin_char = code as u8;
            output.push(fin_char);
            old_code = Some(code);
            continue;
        };

        let in_code = code;
        let mut code = code;
        let mut stack_top = 0usize;

        if code >= free_ent {
            if code != free_ent || stack_top >= stack.len() {
                return Err(corrupt());
            }
            stack[stack_top] = fin_char;
            stack_top += 1;
            code = old;
        }

        while code >= 256 {
            if code >= free_ent || stack_top >= stack.len() {
                return Err(corrupt());
            }
            stack[stack_top] = suffix[code as usize];
            stack_top += 1;
            code = prefix[code as usize];
        }

        fin_char = (code & 0xFF) as u8;
        if stack_top >= stack.len() {
            return Err(corrupt());
        }
        stack[stack_top] = fin_char;
        stack_top += 1;

        while stack_top > 0 {
            stack_top -= 1;
            output.push(stack[stack_top]);
        }

        if free_ent < max_max_code {
            prefix[free_ent as usize] = old;
            suffix[free_ent as usize] = fin_char;
            free_ent += 1;
        }

        old_code = Some(in_code);
    }

    Ok(output)
}

/// LSB-first code reader honoring ncompress block alignment.
///
/// Codes are read in chunks of `n_bits` bytes; a chunk holds
/// `8 * n_bits - (n_bits - 1)` usable bits, which realigns the stream at
/// every code-width transition.
struct CodeReader<'a> {
    data: &'a [u8],
    pos: usize,
    chunk: &'a [u8],
    offset: usize,
    size: usize,
}

impl<'a> CodeReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        CodeReader {
            data,
            pos: 0,
            chunk: &[],
            offset: 0,
            size: 0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn next_code(
        &mut self,
        max_bits: u32,
        max_max_code: u32,
        n_bits: &mut u32,
        max_code: &mut u32,
        free_ent: u32,
        clear_pending: &mut bool,
    ) -> Option<u32> {
        if *clear_pending || self.offset >= self.size || free_ent > *max_code {
            if free_ent > *max_code {
                *n_bits += 1;
                *max_code = if *n_bits == max_bits {
                    max_max_code
                } else {
                    (1 << *n_bits) - 1
                };
            }
            if *clear_pending {
                *n_bits = 9;
                *max_code = (1 << *n_bits) - 1;
                *clear_pending = false;
            }

            let remain = self.data.len() - self.pos;
            if remain == 0 {
                return None;
            }
            let chunk_bytes = (*n_bits as usize).min(remain);
            self.chunk = &self.data[self.pos..self.pos + chunk_bytes];
            self.pos += chunk_bytes;
            self.offset = 0;
            let usable = chunk_bytes * 8;
            if usable < (*n_bits as usize) - 1 {
                return None;
            }
            self.size = usable - ((*n_bits as usize) - 1);
        }

        if self.offset >= self.size {
            return None;
        }

        let n = *n_bits as usize;
        let start_bit = self.offset;
        let end_byte = (start_bit + n - 1) >> 3;
        if end_byte >= self.chunk.len() {
            return None;
        }

        let bit_offset = start_bit & 7;
        let bytes = &self.chunk[(start_bit >> 3)..=end_byte];
        let mut byte_iter = bytes.iter();

        let mut code = (*byte_iter.next().unwrap_or(&0) >> bit_offset) as u32;
        let mut bits_left = n.saturating_sub(8 - bit_offset);
        let mut shift = 8 - bit_offset;

        while bits_left >= 8 {
            code |= (*byte_iter.next().unwrap_or(&0) as u32) << shift;
            shift += 8;
            bits_left -= 8;
        }
        if bits_left > 0 {
            let mask = (1u32 << bits_left) - 1;
            code |= ((*byte_iter.next().unwrap_or(&0) as u32) & mask) << shift;
        }

        self.offset += n;
        Some(code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_magic() {
        assert!(is_compressed(&[0x1F, 0x9D, 0x90]));
        assert!(!is_compressed(&[0x1F, 0x8B, 0x08]));
        assert!(!is_compressed(&[0x1F]));
    }

    /// Real `.Z` record (`/.cshrc`, IRIX 5.3 eoe1.sw.unix) and its
    /// decompressed content.
    #[test]
    fn decodes_real_record() {
        let compressed = include_bytes!("../tests/data/cshrc.z");
        let expected = include_bytes!("../tests/data/cshrc.out");
        let decoded = decompress(compressed).unwrap();
        assert_eq!(decoded, expected);
    }

    #[test]
    fn rejects_garbage() {
        assert!(decompress(b"hello world").is_err());
        assert!(decompress(&[0x1F, 0x9D, 0x05, 0x00]).is_err());
    }
}
