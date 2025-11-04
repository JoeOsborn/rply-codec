use ffmpeg_next::{
    codec::Debug,
    util::frame::{Audio as FFAFrame, Video as FFVFrame},
};
use retro_rs::Emulator;
use rply_codec::{Frame, decode};
use std::path::Path;

fn copy_audio(samples: &[i16], frame: &mut FFAFrame) {
    println!(
        "Copy {} samples into {}",
        samples.len() / 2,
        frame.samples()
    );
    for (i, pair) in samples.chunks_exact(2).enumerate() {
        let [l, r] = pair else {
            unreachable!();
        };
        frame.plane_mut(0)[i] = f32::from(*l) / 65535.0;
        frame.plane_mut(1)[i] = f32::from(*r) / 65535.0;
    }
}
fn copy_video(fb: &[u8], frame: &mut FFVFrame) {
    let w = i32::try_from(frame.width()).unwrap();
    let h = i32::try_from(frame.height()).unwrap();
    let fmt: ffmpeg_next::ffi::AVPixelFormat = frame.format().into();
    unsafe {
        let frame_ptr = frame.as_mut_ptr();
        let ret = ffmpeg_next::ffi::av_image_fill_arrays(
            (*frame_ptr).data.as_mut_ptr(),
            (*frame_ptr).linesize.as_mut_ptr(),
            fb.as_ptr(),
            fmt,
            w,
            h,
            1,
        );
        assert!(ret > 0, "av_image_fill_arrays returned {ret}");
    }
}

fn main() {
    ffmpeg_next::init().unwrap();
    ffmpeg_next::log::set_level(ffmpeg_next::log::Level::Trace);
    let args: Vec<_> = std::env::args().collect();
    let file =
        std::fs::File::open(args.get(1).unwrap_or(&"examples/bobl.replay".to_string())).unwrap();
    let outfile = std::path::PathBuf::from(args.get(2).unwrap_or(&"examples/bobl.mp4".to_string()));
    let corefile = args
        .get(3)
        .unwrap_or(&"cores/fceumm_libretro".to_string())
        .clone();
    let romfile = args.get(4).unwrap_or(&"roms/bobl.nes".to_string()).clone();
    let mut emu = Emulator::create(Path::new(&corefile), Path::new(&romfile));
    let file = std::io::BufReader::new(file);
    let mut rply = decode(file).unwrap();
    let header = &rply.header;
    println!("Header in: {header:?}");
    if header.version() == 0 {
        println!("Only use this program for v1+ replays!");
        std::process::exit(-1);
    }
    // run emu a tick to make sure we have right frame sizes, etc
    emu.run([retro_rs::Buttons::default(); 2]);

    let (w, h) = emu.framebuffer_size();

    let mut output = ffmpeg_next::format::output(&outfile).unwrap();
    let emu_time_base = ffmpeg_next::util::rational::Rational::new(1, emu.get_video_fps() as i32);
    let out_video_codec = ffmpeg_next::encoder::find(ffmpeg_next::codec::Id::H264).unwrap();
    let mut out_video_ctx = ffmpeg_next::codec::context::Context::new_with_codec(out_video_codec);
    let mut video_params = ffmpeg_next::codec::Parameters::new();
    unsafe {
        let vps = video_params.as_mut_ptr();
        (*vps).width = i32::try_from(w).unwrap();
        (*vps).height = i32::try_from(h).unwrap();
        (*vps).codec_id = out_video_codec.id().into();
        (*vps).framerate = ffmpeg_next::util::rational::Rational::from(emu.get_video_fps()).into();
    };
    out_video_ctx.set_parameters(video_params).unwrap();
    out_video_ctx.set_time_base(emu_time_base);
    out_video_ctx.debug(Debug::all());
    dbg!(out_video_ctx.codec().unwrap().id());
    let mut out_video = output.add_stream_with(&out_video_ctx).unwrap();
    out_video.set_time_base(emu_time_base);
    let video_time_base = out_video.time_base();
    let mut out_video_enc = out_video_ctx.encoder().video().unwrap();
    out_video_enc.debug(Debug::all());
    out_video_enc.set_format(ffmpeg_next::format::Pixel::YUV420P);
    out_video_enc.set_width(u32::try_from(w).unwrap());
    out_video_enc.set_height(u32::try_from(h).unwrap());
    out_video_enc.set_time_base(video_time_base);
    let mut out_video_enc = out_video_enc.open().unwrap();
    let out_audio_codec = ffmpeg_next::encoder::find(ffmpeg_next::codec::Id::AAC).unwrap();
    let mut out_audio_ctx = ffmpeg_next::codec::context::Context::new_with_codec(out_audio_codec);
    out_audio_ctx.debug(Debug::all());
    let mut audio_params = ffmpeg_next::codec::Parameters::new();
    unsafe {
        let aps = audio_params.as_mut_ptr();
        (*aps).codec_id = out_audio_codec.id().into();
        (*aps).sample_rate = emu.get_audio_sample_rate() as i32;
        (*aps).channels = 2;
    };
    out_audio_ctx.set_parameters(audio_params).unwrap();
    out_audio_ctx.set_time_base(ffmpeg_next::util::rational::Rational::new(
        1,
        emu.get_audio_sample_rate() as i32,
    ));
    let mut out_audio = output.add_stream_with(&out_audio_ctx).unwrap();
    out_audio.set_time_base(ffmpeg_next::util::rational::Rational::new(
        1,
        emu.get_audio_sample_rate() as i32,
    ));
    let audio_time_base = out_audio.time_base();
    dbg!(audio_time_base);
    let mut out_audio_enc = out_audio_ctx.encoder().audio().unwrap();
    out_audio_enc.set_channels(2);
    out_audio_enc.set_format(ffmpeg_next::format::Sample::F32(
        ffmpeg_next::format::sample::Type::Planar,
    ));
    out_audio_enc.set_channel_layout(ffmpeg_next::ChannelLayout::STEREO);
    out_audio_enc.set_time_base(audio_time_base);
    out_audio_enc.set_rate(emu.get_audio_sample_rate() as i32);
    let mut out_audio_enc = out_audio_enc.open().unwrap();
    let mut out_vframe = FFVFrame::new(
        out_video_enc.format(),
        out_video_enc.width(),
        out_video_enc.height(),
    );
    let mut out_aframe = FFAFrame::new(
        out_audio_enc.format(),
        out_audio_enc.frame_size() as usize,
        out_audio_enc.channel_layout(),
    );
    dbg!(
        out_audio_enc.channels(),
        out_audio_enc.channel_layout(),
        out_audio_enc.format()
    );
    dbg!(out_aframe.samples());
    dbg!(out_aframe.data(0).len(), out_aframe.data(1).len());
    output.write_header().unwrap();
    assert!(emu.load(&rply.initial_state));
    let mut frame = Frame::default();
    let mut fb = vec![0_u8; w * h * 3];

    while let Ok(()) = rply
        .read_frame(&mut frame)
        .inspect_err(|e| println!("Err: {e}"))
    {
        use ffmpeg_next::util::mathematics::Rescale;
        //println!("FRAME");
        let buttons = frame_to_buttons(&frame);
        emu.run(buttons);
        emu.copy_framebuffer_rgb888(&mut fb).unwrap();
        // output one frame of video/audio, set_pts
        // copy video to out_vframe
        copy_video(&fb, &mut out_vframe);
        let frame_num = i64::try_from(rply.frame_number).unwrap();
        out_vframe.set_pts(Some(
            frame_num.rescale(emu_time_base, out_video_enc.time_base()),
        ));
        out_video_enc.send_frame(&out_vframe).unwrap();
        // copy audio to out_aframe, set_pts
        // maybe in a loop?
        #[allow(unused_must_use)]
        emu.peek_audio_sample(|samples| {
            copy_audio(samples, &mut out_aframe);
        });
        out_aframe.set_pts(Some(
            frame_num.rescale(emu_time_base, out_audio_enc.time_base()),
        ));
        println!(
            "vtime {} atime {}",
            out_vframe.pts().unwrap() as f64 / 60.0,
            out_aframe.pts().unwrap() as f64 / 48000.0
        );
        out_audio_enc.send_frame(&out_aframe).unwrap();
        let mut encoded = ffmpeg_next::Packet::empty();
        while out_video_enc.receive_packet(&mut encoded).is_ok() {
            encoded.set_stream(0);
            encoded.rescale_ts(out_video_enc.time_base(), video_time_base);
            encoded.write_interleaved(&mut output).unwrap();
        }
        while out_audio_enc.receive_packet(&mut encoded).is_ok() {
            encoded.set_stream(1);
            encoded.rescale_ts(out_audio_enc.time_base(), audio_time_base);
            encoded.write_interleaved(&mut output).unwrap();
        }
        if !frame.checkpoint_bytes.is_empty() {
            assert!(emu.load(&frame.checkpoint_bytes));
        }
        if Some(rply.frame_number) == rply.header.frame_count() {
            break;
        }
    }
    out_video_enc.send_eof().unwrap();
    out_audio_enc.send_eof().unwrap();
    output.write_trailer().unwrap();
}

fn frame_to_buttons(frame: &Frame) -> [retro_rs::Buttons; 2] {
    use retro_rs::Buttons;
    let mut buttons = [Buttons::default(); 2];
    for inp in &frame.input_events {
        let port = usize::from(inp.port);
        if port < buttons.len() && inp.device == 1 {
            buttons[port] = Buttons::from(inp.val);
        }
    }
    buttons
}
