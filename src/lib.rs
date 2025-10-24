mod rply;
pub use rply::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v2_header() {
        let mut file = std::io::BufReader::new(std::fs::File::open("examples/bobl.replay").unwrap());
        let header = match rply::read_header(&mut file).unwrap() {
            rply::Header::V0V1(_) => panic!("Version too low"),
            rply::Header::V2(h) => h,
        };
        assert_eq!(header.base.version, 2);
        assert_eq!(header.base.content_crc, 2199475946);
        assert_eq!(header.base.initial_state_size, 2531);
        assert_eq!(header.base.identifier, 1761326589);
        assert_eq!(header.frame_count, 6383);
        assert_eq!(header.block_size, 128);
        assert_eq!(header.superblock_size, 16);
        assert_eq!(header.checkpoint_commit_interval, 4);
        assert_eq!(header.checkpoint_commit_threshold, 2);
        assert_eq!(header.checkpoint_compression, rply::Compression::None);
    }
}
