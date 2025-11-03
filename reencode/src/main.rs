use rply_codec::{Counter, Frame, Timer, counts, decode, encode, stats};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let file =
        std::fs::File::open(args.get(1).unwrap_or(&"examples/bobl.replay".to_string())).unwrap();
    let outfile = std::fs::File::create(
        args.get(2)
            .unwrap_or(&"examples/bobl_smallblocks.replay".to_string()),
    )
    .unwrap();
    let file = std::io::BufReader::new(file);
    let mut outfile = std::io::BufWriter::new(outfile);
    let mut rply = decode(file).unwrap();
    let header = &rply.header;
    println!("{header:?}");
    if header.version() == 0 {
        println!("Can't upgrade v0 replays with reencode, upgrade to v1 first using upgrade0");
        std::process::exit(-1);
    }
    let mut header_out = header.clone();
    header_out.upgrade();
    header_out.set_block_size(128);
    header_out.set_superblock_size(128);
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

        // TODO run libretro core here, maybe serialize into frame.checkpoint_bytes
        // TODO maybe get screenshot or add to video

        //frame.drop_checkpoint();
        out.write_frame(&frame).unwrap();
        if Some(rply.frame_number) == rply.header.frame_count() {
            break;
        }
    }
    out.finish().unwrap();
    assert_eq!(out.frame_number, rply.frame_number);
    assert_eq!(out.header.frame_count(), rply.header.frame_count());
    assert_eq!(out.header.frame_count(), Some(out.frame_number));
    for timer in [
        Timer::DecodeFrame,
        Timer::DecodeCheckpoint,
        Timer::DecodeStatestream,
        Timer::EncodeFrame,
        Timer::EncodeCheckpoint,
        Timer::EncodeStatestream,
    ] {
        let times = stats(timer);
        #[allow(clippy::cast_precision_loss)]
        let avg_time = (times.micros as f64 / times.count as f64) / 1000.0;
        println!("{timer:?}: {} ({avg_time:.8}ms avg)", times.count,);
    }
    for counter in [
        Counter::DecSkippedSuperblocks,
        Counter::DecSkippedBlocks,
        Counter::EncReusedBlocks,
        Counter::EncReusedSuperblocks,
        Counter::EncSkippedBlocks,
        Counter::EncMemCmps,
        Counter::EncHashes,
        Counter::EncTotalBlocks,
        Counter::EncTotalSuperblocks,
        Counter::EncTotalKBsIn,
        Counter::EncTotalKBsOut,
    ] {
        println!("{counter:?}: {}", counts(counter));
    }
}
