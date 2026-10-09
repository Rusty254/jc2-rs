//! Name hashing used throughout Just Cause 2.
//!
//! The engine identifies every file and every property name by a 32-bit hash:
//! Bob Jenkins' lookup3 `hashlittle` with seed 0 over the raw ASCII bytes.
//! Archive tables (`.tab`) key entries by the hash of the **file name only**
//! (no directory), which is why the `dropzone\` override folder is flat.

/// Bob Jenkins' lookup3 `hashlittle`, little-endian variant.
pub fn hashlittle(data: &[u8], seed: u32) -> u32 {
    let len = data.len();
    let mut a = 0xdead_beef_u32.wrapping_add(len as u32).wrapping_add(seed);
    let mut b = a;
    let mut c = a;

    let (blocks, _) = data.as_chunks::<12>();
    let mut remaining = len;
    for k in blocks {
        if remaining <= 12 {
            // lookup3 stops the main loop when 12 or fewer bytes are left
            break;
        }
        a = a.wrapping_add(u32::from_le_bytes([k[0], k[1], k[2], k[3]]));
        b = b.wrapping_add(u32::from_le_bytes([k[4], k[5], k[6], k[7]]));
        c = c.wrapping_add(u32::from_le_bytes([k[8], k[9], k[10], k[11]]));
        // mix(a,b,c)
        a = a.wrapping_sub(c);
        a ^= c.rotate_left(4);
        c = c.wrapping_add(b);
        b = b.wrapping_sub(a);
        b ^= a.rotate_left(6);
        a = a.wrapping_add(c);
        c = c.wrapping_sub(b);
        c ^= b.rotate_left(8);
        b = b.wrapping_add(a);
        a = a.wrapping_sub(c);
        a ^= c.rotate_left(16);
        c = c.wrapping_add(b);
        b = b.wrapping_sub(a);
        b ^= a.rotate_left(19);
        a = a.wrapping_add(c);
        c = c.wrapping_sub(b);
        c ^= b.rotate_left(4);
        b = b.wrapping_add(a);
        remaining -= 12;
    }
    if remaining == 0 {
        return c;
    }
    let tail = &data[len - remaining..];
    let mut t = [0u8; 12];
    t[..tail.len()].copy_from_slice(tail);
    a = a.wrapping_add(u32::from_le_bytes([t[0], t[1], t[2], t[3]]));
    b = b.wrapping_add(u32::from_le_bytes([t[4], t[5], t[6], t[7]]));
    c = c.wrapping_add(u32::from_le_bytes([t[8], t[9], t[10], t[11]]));
    // final(a,b,c)
    c ^= b;
    c = c.wrapping_sub(b.rotate_left(14));
    a ^= c;
    a = a.wrapping_sub(c.rotate_left(11));
    b ^= a;
    b = b.wrapping_sub(a.rotate_left(25));
    c ^= b;
    c = c.wrapping_sub(b.rotate_left(16));
    a ^= c;
    a = a.wrapping_sub(c.rotate_left(4));
    b ^= a;
    b = b.wrapping_sub(a.rotate_left(14));
    c ^= b;
    c = c.wrapping_sub(b.rotate_left(24));
    c
}

/// Hash of an arbitrary string (property names, etc.).
pub fn hash_str(s: &str) -> u32 {
    hashlittle(s.as_bytes(), 0)
}

/// Strip any directory part (either separator) and return the file name.
pub fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// The key the archive tables use for a file: hash of its bare file name.
pub fn name_hash(path: &str) -> u32 {
    hash_str(file_name(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup3_reference_vectors() {
        // From Bob Jenkins' lookup3.c self-test.
        assert_eq!(
            hashlittle(b"Four score and seven years ago", 0),
            0x1777_0551
        );
        assert_eq!(
            hashlittle(b"Four score and seven years ago", 1),
            0xcd62_8161
        );
        assert_eq!(hashlittle(b"", 0), 0xdead_beef);
    }

    #[test]
    fn exact_multiples_of_twelve() {
        // 12 and 24 byte inputs exercise the "stop the loop at <= 12 left" rule.
        let s12 = b"abcdefghijkl";
        let s24 = b"abcdefghijklmnopqrstuvwx";
        assert_eq!(hashlittle(s12, 0), reference(s12, 0));
        assert_eq!(hashlittle(s24, 0), reference(s24, 0));
        assert_eq!(
            hashlittle(b"abcdefghijklm", 0),
            reference(b"abcdefghijklm", 0)
        );
    }

    #[test]
    fn archive_key_is_file_name_only() {
        assert_eq!(name_hash("gui\\credits.blz"), hash_str("credits.blz"));
        assert_eq!(
            name_hash("exported/vehicles/lave/lave.v046_buscoach.ee"),
            hash_str("lave.v046_buscoach.ee")
        );
    }

    /// Straightforward transcription of lookup3.c used to cross-check the fast path.
    fn reference(key: &[u8], seed: u32) -> u32 {
        let mut length = key.len();
        let mut a: u32 = 0xdeadbeefu32.wrapping_add(length as u32).wrapping_add(seed);
        let (mut b, mut c) = (a, a);
        let mut k = key;
        while length > 12 {
            a = a.wrapping_add(u32::from_le_bytes([k[0], k[1], k[2], k[3]]));
            b = b.wrapping_add(u32::from_le_bytes([k[4], k[5], k[6], k[7]]));
            c = c.wrapping_add(u32::from_le_bytes([k[8], k[9], k[10], k[11]]));
            a = a.wrapping_sub(c);
            a ^= c.rotate_left(4);
            c = c.wrapping_add(b);
            b = b.wrapping_sub(a);
            b ^= a.rotate_left(6);
            a = a.wrapping_add(c);
            c = c.wrapping_sub(b);
            c ^= b.rotate_left(8);
            b = b.wrapping_add(a);
            a = a.wrapping_sub(c);
            a ^= c.rotate_left(16);
            c = c.wrapping_add(b);
            b = b.wrapping_sub(a);
            b ^= a.rotate_left(19);
            a = a.wrapping_add(c);
            c = c.wrapping_sub(b);
            c ^= b.rotate_left(4);
            b = b.wrapping_add(a);
            length -= 12;
            k = &k[12..];
        }
        if length == 0 {
            return c;
        }
        let mut t = [0u8; 12];
        t[..length].copy_from_slice(&k[..length]);
        a = a.wrapping_add(u32::from_le_bytes([t[0], t[1], t[2], t[3]]));
        b = b.wrapping_add(u32::from_le_bytes([t[4], t[5], t[6], t[7]]));
        c = c.wrapping_add(u32::from_le_bytes([t[8], t[9], t[10], t[11]]));
        c ^= b;
        c = c.wrapping_sub(b.rotate_left(14));
        a ^= c;
        a = a.wrapping_sub(c.rotate_left(11));
        b ^= a;
        b = b.wrapping_sub(a.rotate_left(25));
        c ^= b;
        c = c.wrapping_sub(b.rotate_left(16));
        a ^= c;
        a = a.wrapping_sub(c.rotate_left(4));
        b ^= a;
        b = b.wrapping_sub(a.rotate_left(14));
        c ^= b;
        c = c.wrapping_sub(b.rotate_left(24));
        c
    }
}
