use rply_codec::{Frame, decode};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let file =
        std::fs::File::open(args.get(1).unwrap_or(&"examples/bobl.replay".to_string())).unwrap();
    let mut file = std::io::BufReader::new(file);
    let mut rply = decode(&mut file).unwrap();
    let header = &rply.header;
    println!("{header:?}");
    let mut frame = Frame::default();
    rply.read_frame(&mut frame).unwrap();
    println!("{frame:?}");
}
