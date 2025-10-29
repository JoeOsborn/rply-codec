use rply_codec::{Frame, decode, encode};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let file =
        std::fs::File::open(args.get(1).unwrap_or(&"examples/bobl.replay".to_string())).unwrap();
    let outfile = std::fs::File::create(
        args.get(2)
            .unwrap_or(&"examples/bobl_smallblocks.replay".to_string()),
    )
    .unwrap();
    let mut file = std::io::BufReader::new(file);
    let mut outfile = std::io::BufWriter::new(outfile);
    let mut rply = decode(&mut file).unwrap();
    let header = &rply.header;
    println!("{header:?}");
    let mut header_out = header.clone();
    header_out.set_block_size(64);
    let mut out = encode(header_out, &rply.initial_state, &mut outfile).unwrap();
    let mut frame = Frame::default();
    while let Ok(()) = rply
        .read_frame(&mut frame)
        .inspect_err(|e| println!("Err: {e}"))
    {
        println!(
            " {}{:08} {}",
            if frame.checkpoint_bytes.is_empty() {
                " "
            } else {
                "*"
            },
            rply.frame_number,
            frame.inputs(),
        );
        out.write_frame(&frame).unwrap();
        if Some(rply.frame_number) == rply.header.frame_count() {
            println!("Done!");
            break;
        }
    }
    out.finish().unwrap();
    assert_eq!(out.frame_number, rply.frame_number);
    assert_eq!(out.header.frame_count(), rply.header.frame_count());
    assert_eq!(out.header.frame_count(), Some(out.frame_number));
}
