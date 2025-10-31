use retro_rs::Emulator;
use rply_codec::{Frame, InputData, ReplayError, decode, encode};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let file =
        std::fs::File::open(args.get(1).unwrap_or(&"examples/v0.replay".to_string())).unwrap();
    let outfile =
        std::fs::File::create(args.get(2).unwrap_or(&"examples/v2.replay".to_string())).unwrap();
    let corefile = args
        .get(3)
        .unwrap_or(&"cores/fceumm_libretro".to_string())
        .clone();
    let romfile = args.get(4).unwrap_or(&"roms/demo.nes".to_string()).clone();
    let mut emu = Emulator::create(Path::new(&corefile), Path::new(&romfile));
    let file = std::io::BufReader::new(file);
    let mut outfile = std::io::BufWriter::new(outfile);
    let mut rply = decode(file).unwrap();
    let header = &rply.header;
    println!("Header in: {header:?}");
    if header.version() != 0 {
        println!("Only use this program for v0 replays!");
        std::process::exit(-1);
    }
    assert!(emu.load(&rply.initial_state));
    let mut header_out = header.clone();
    header_out.upgrade();
    let mut encoder = encode(header_out, &rply.initial_state, &mut outfile).unwrap();
    let mut frame = Frame::default();
    rply.read_key_events(&mut frame).unwrap();
    rply.read_end_of_frame(&mut frame).unwrap();
    let frame = Rc::new(RefCell::new(frame));
    let rply = Rc::new(RefCell::new(rply));
    let cb = {
        let frame = Rc::clone(&frame);
        let rply = Rc::clone(&rply);
        Box::new(move |port, device, idx, id| {
            let val = rply.borrow_mut().read_v0_button().unwrap();
            //println!("{port}-{device}-{idx}-{id}: 0x{val:x}");
            frame.borrow_mut().input_events.push(InputData {
                port: u8::try_from(port).unwrap(),
                device: u8::try_from(device).unwrap(),
                idx: u8::try_from(idx).unwrap(),
                id: u16::try_from(id).unwrap(),
                val,
            });
            val
        })
    };
    loop {
        frame.borrow_mut().clear();
        emu.run_with_button_callback(cb.clone());
        match rply.borrow_mut().read_key_events(&mut frame.borrow_mut()) {
            Ok(()) => {}
            Err(ReplayError::IO(e)) => {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    break;
                }
                panic!("{e}");
            }
            Err(e) => panic!("{e}"),
        }
        rply.borrow_mut()
            .read_end_of_frame(&mut frame.borrow_mut())
            .unwrap();
        encoder.write_frame(&frame.borrow()).unwrap();
    }
    encoder.finish().unwrap();
    println!("Header out: {:?}", encoder.header);
}
