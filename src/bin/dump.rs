use std::io::Seek;

use rply_codec::*;

pub fn main() {
    let args: Vec<_> = std::env::args().collect();
    let file =
        std::fs::File::open(args.get(1).unwrap_or(&"examples/bobl.replay".to_string())).unwrap();
    let mut file = std::io::BufReader::new(file);
    let header = read_header(&mut file).unwrap();
    println!("{:?}", header);
    let initial_size = match &header {
        Header::V0V1(header_base) => header_base.initial_state_size,
        Header::V2(header_v2) => header_v2.base.initial_state_size,
    };
    file.seek_relative(initial_size as i64).unwrap();
    let mut frame = Frame::default();
    read_frame(&mut file, &header, &mut frame).unwrap();
    println!("{:?}", frame);
}
