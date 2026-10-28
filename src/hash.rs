const POLY: u64 = 0x01A1561E0005800C;

const fn make_table() -> [u64; 256] {
    let mut table = [0u64; 256];
    let mut i = 1;
    while i < 256 {
        table[i] = (table[i >> 1] << 1) ^ if i & 1 == 1 { POLY } else { 0 };
        i += 1;
    }
    table
}

static TABLE: [u64; 256] = make_table();

pub fn crc64(data: &[u8]) -> u64 {
    let mut h = u64::MAX;
    for &b in data {
        h = (h >> 8) ^ TABLE[((h ^ b as u64) & 0xFF) as usize];
    }
    !h
}

#[cfg(test)]
mod tests {
    use super::crc64;

    #[test]
    fn known_name_hashes() {
        assert_eq!(crc64(b"ExperienceValue"), 0xF302EB5E1DAD1D78);
        assert_eq!(crc64(b"plinth_animations"), 0xC527EC788C8CB923);
        assert_eq!(crc64(b"DifficultyLevelDamageScales"), 0x106A1176BCFF31E1);
        assert_eq!(crc64(b"HitPointMultiplier"), 0x94B7CB5DE6765903);
        assert_eq!(crc64(b"base/brute"), 0xCA795FB820B9CD59);
    }
}
