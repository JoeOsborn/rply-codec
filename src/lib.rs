mod rply;

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v2_header() {
        let mut file = std::fs::File::open("examples/bobl.replay").unwrap();
        let header = match rply::read_header(&mut file).unwrap() {
            rply::Header::V0V1(_) => panic!("Version too low"),
            rply::Header::V2(h) => h,
        };
        // content_crc: 2199475946,
        // initial_state_size: 2531,
        // identifier: 1761326589,
        assert_eq!(header.base.version, 2);
        assert_eq!(header.base.content_crc, 2199475946);
        assert_eq!(header.base.initial_state_size, 2531);
        assert_eq!(header.base.identifier, 1761326589);
    }
}
