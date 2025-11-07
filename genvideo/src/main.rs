use ffmpeg_next::util::{mathematics::Rescale, rational::Rational};
use ffmpeg_next::{
    format::context::Output as FFOut,
    software::converter as img_conv,
    util::frame::{Audio as FFAFrame, Video as FFVFrame},
};
use retro_rs::Emulator;
use ringbuf::traits::{Consumer, Observer, RingBuffer};
use rply_codec::{Frame, decode};
use std::{error::Error, path::Path};

#[derive(Debug, Clone, Copy)]
struct ToI32Err();

impl std::fmt::Display for ToI32Err {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Float conversion out of integer bounds or applied to nan"
        )
    }
}
impl Error for ToI32Err {}

trait ToI32 {
    fn to_i32(self) -> Result<i32, ToI32Err>;
}

impl ToI32 for f64 {
    fn to_i32(mut self) -> Result<i32, ToI32Err> {
        self = self.trunc();
        if self.is_infinite()
            || self.is_nan()
            || (self < f64::from(i32::MIN))
            || (f64::from(i32::MAX) < self)
        {
            return Err(ToI32Err());
        }
        Ok(unsafe { self.to_int_unchecked() })
    }
}

struct VideoState {
    out_video_enc: ffmpeg_next::encoder::video::Encoder,
    out_vframe: FFVFrame,
    out_rgbframe: FFVFrame,
    encoded_video: ffmpeg_next::Packet,
    converter: ffmpeg_next::software::scaling::Context,
    emu_time_base: Rational,
    native_pixel_format: bool,
    stride: usize,
}

impl VideoState {
    fn new(
        emu_time_base: Rational,
        aspect_ratio: Rational,
        w: usize,
        h: usize,
        pixel_format: retro_rs::libretro::retro_pixel_format,
        output: &mut FFOut,
    ) -> Self {
        let out_video_codec = ffmpeg_next::encoder::find(ffmpeg_next::codec::Id::H264).unwrap();
        let mut out_video_ctx =
            ffmpeg_next::codec::context::Context::new_with_codec(out_video_codec);
        // out_video_ctx.set_time_base(emu_time_base);
        let mut video_params = ffmpeg_next::codec::Parameters::new();
        unsafe {
            let vps = video_params.as_mut_ptr();
            (*vps).width = i32::try_from(w).unwrap();
            (*vps).height = i32::try_from(h).unwrap();
            (*vps).codec_id = out_video_codec.id().into();
            (*vps).codec_type = ffmpeg_next::ffi::AVMediaType::AVMEDIA_TYPE_VIDEO;
            (*vps).sample_aspect_ratio = aspect_ratio.into();
        };
        out_video_ctx.set_parameters(video_params).unwrap();
        let _out_video = output.add_stream_with(&out_video_ctx).unwrap();
        let encoded_video = ffmpeg_next::Packet::empty();
        // out_video.set_time_base(emu_time_base);
        let mut out_video_enc = out_video_ctx.encoder().video().unwrap();
        out_video_enc.set_format(ffmpeg_next::format::Pixel::YUV420P);
        out_video_enc.set_aspect_ratio(aspect_ratio);
        out_video_enc.set_width(u32::try_from(w).unwrap());
        out_video_enc.set_height(u32::try_from(h).unwrap());
        out_video_enc.set_time_base(emu_time_base);
        let out_video_enc = out_video_enc.open().unwrap();
        let out_vframe = FFVFrame::new(
            out_video_enc.format(),
            out_video_enc.width(),
            out_video_enc.height(),
        );
        let (copy_format, is_native, stride) = match pixel_format {
            retro_rs::libretro::retro_pixel_format::RETRO_PIXEL_FORMAT_0RGB1555 => {
                (ffmpeg_next::format::Pixel::RGB555, true, 2)
            }
            retro_rs::libretro::retro_pixel_format::RETRO_PIXEL_FORMAT_XRGB8888 => {
                (ffmpeg_next::format::Pixel::ZRGB, true, 4)
            }
            retro_rs::libretro::retro_pixel_format::RETRO_PIXEL_FORMAT_RGB565 => {
                (ffmpeg_next::format::Pixel::RGB565, true, 2)
            }
            _other => (ffmpeg_next::format::Pixel::RGB24, false, 3),
        };
        let out_rgbframe = FFVFrame::new(
            copy_format,
            u32::try_from(w).unwrap(),
            u32::try_from(h).unwrap(),
        );

        let converter = img_conv(
            (u32::try_from(w).unwrap(), u32::try_from(h).unwrap()),
            out_rgbframe.format(),
            out_video_enc.format(),
        )
        .unwrap();
        Self {
            out_video_enc,
            out_vframe,
            out_rgbframe,
            encoded_video,
            converter,
            emu_time_base,
            native_pixel_format: is_native,
            stride,
        }
    }
    fn writeout(&mut self, output: &mut FFOut) {
        let output_time_base = output.stream(0).unwrap().time_base();
        while self
            .out_video_enc
            .receive_packet(&mut self.encoded_video)
            .is_ok()
        {
            self.encoded_video.set_stream(0);
            self.encoded_video
                .rescale_ts(self.out_video_enc.time_base(), output_time_base);
            self.encoded_video.write_interleaved(output).unwrap();
        }
    }
    fn send_frame(&mut self, emu: &Emulator, frame_num: u64, output: &mut FFOut) {
        // output one frame of video/audio, set_pts
        // copy video to out_vframe
        if self.native_pixel_format {
            let pitch = emu.framebuffer_pitch();
            let (w, h) = emu.framebuffer_size();
            let stride = self.stride;
            emu.peek_framebuffer(|fb| {
                let data = self.out_rgbframe.data_mut(0);
                for y in 0..h {
                    data[(y * w * stride)..((y + 1) * w * stride)]
                        .copy_from_slice(&fb[(y * pitch)..(y * pitch + w * stride)]);
                }
            })
            .unwrap();
        } else {
            emu.copy_framebuffer_rgb888(self.out_rgbframe.data_mut(0))
                .unwrap();
        }
        self.converter
            .run(&self.out_rgbframe, &mut self.out_vframe)
            .unwrap();
        let frame_num = i64::try_from(frame_num).unwrap();
        let frame_pts = frame_num.rescale(self.emu_time_base, self.out_video_enc.time_base());
        self.out_vframe.set_pts(Some(frame_pts));
        self.out_video_enc.send_frame(&self.out_vframe).unwrap();
        self.writeout(output);
    }
    fn drain(&mut self, output: &mut FFOut) {
        self.out_video_enc.send_eof().unwrap();
        self.writeout(output);
    }
}

struct AudioState {
    out_audio_enc: ffmpeg_next::encoder::audio::Encoder,
    out_aframe: FFAFrame,
    in_aframe: FFAFrame,
    encoded_audio: ffmpeg_next::Packet,
    audio_buf: ringbuf::LocalRb<ringbuf::storage::Heap<i16>>,
    audio_frame_out: i64,
    audio_frame_in: i64,
    resampler: ffmpeg_next::software::resampling::Context,
}

impl AudioState {
    fn new(in_audio_sample_rate: i32, emu_video_frame_rate: i32, output: &mut FFOut) -> Self {
        let out_audio_codec = ffmpeg_next::encoder::find(ffmpeg_next::codec::Id::AAC).unwrap();
        let mut out_audio_ctx =
            ffmpeg_next::codec::context::Context::new_with_codec(out_audio_codec);
        // out_audio_ctx.debug(Debug::all());
        let mut audio_params = ffmpeg_next::codec::Parameters::new();
        unsafe {
            let aps = audio_params.as_mut_ptr();
            (*aps).codec_id = out_audio_codec.id().into();
            (*aps).codec_type = ffmpeg_next::ffi::AVMediaType::AVMEDIA_TYPE_AUDIO;
            (*aps).sample_rate = 48000;
            (*aps).frame_size = 1024;
            (*aps).channels = 2;
        };
        out_audio_ctx.set_parameters(audio_params).unwrap();
        let _out_audio = output.add_stream_with(&out_audio_ctx).unwrap();
        let encoded_audio = ffmpeg_next::Packet::empty();
        let audio_time_base = Rational::new(1, 48000);
        let mut out_audio_enc = out_audio_ctx.encoder().audio().unwrap();
        out_audio_enc.set_channels(2);
        out_audio_enc.set_format(ffmpeg_next::format::Sample::F32(
            ffmpeg_next::format::sample::Type::Planar,
        ));
        out_audio_enc.set_channel_layout(ffmpeg_next::ChannelLayout::STEREO);
        out_audio_enc.set_time_base(audio_time_base);
        out_audio_enc.set_rate(48000);
        let out_audio_enc = out_audio_enc.open().unwrap();
        let mut in_aframe = FFAFrame::new(
            ffmpeg_next::format::Sample::I16(ffmpeg_next::format::sample::Type::Packed),
            704,
            //            dbg!(in_audio_sample_rate / emu_video_frame_rate) as usize,
            ffmpeg_next::ChannelLayout::STEREO,
        );
        in_aframe.set_rate(u32::try_from(in_audio_sample_rate).unwrap());
        let mut out_aframe = FFAFrame::new(
            out_audio_enc.format(),
            out_audio_enc.frame_size() as usize,
            out_audio_enc.channel_layout(),
        );
        out_aframe.set_rate(out_audio_enc.rate());
        let resampler = ffmpeg_next::software::resampler(
            (
                in_aframe.format(),
                in_aframe.channel_layout(),
                in_aframe.rate(),
            ),
            (
                out_aframe.format(),
                out_aframe.channel_layout(),
                out_aframe.rate(),
            ),
        )
        .unwrap();
        println!(
            "Resample from {} to {}",
            in_aframe.rate(),
            out_aframe.rate()
        );
        let audio_buf = ringbuf::LocalRb::new(in_aframe.samples() * 2 * 20);

        Self {
            out_audio_enc,
            out_aframe,
            encoded_audio,
            audio_buf,
            audio_frame_out: 0,
            audio_frame_in: 0,
            resampler,
            in_aframe,
        }
    }
    fn writeout(&mut self, output: &mut FFOut) {
        let output_time_base = output.stream(1).unwrap().time_base();
        while self
            .out_audio_enc
            .receive_packet(&mut self.encoded_audio)
            .is_ok()
        {
            self.encoded_audio.set_stream(1);
            self.encoded_audio
                .rescale_ts(self.out_audio_enc.time_base(), output_time_base);
            self.encoded_audio.write_interleaved(output).unwrap();
        }
    }
    fn resample(&mut self, drain: bool) {
        println!(
            "RESAMPLE {:?} {:?}",
            self.in_aframe.pts(),
            self.out_aframe.pts()
        );
        match dbg!(self.resampler.run(&self.in_aframe, &mut self.out_aframe)) {
            Ok(Some(delay)) if drain => {
                dbg!(delay);
                let null_frame = unsafe { FFAFrame::wrap(std::ptr::null_mut()) };
                while let Ok(Some(delay)) = self.resampler.run(&null_frame, &mut self.out_aframe) {
                    dbg!("2", delay);
                    if !drain {
                        break;
                    }
                }
            }
            Err(e) => println!("Resampler error {e}"),
            _ => {}
        }
    }
    fn send_frames(&mut self, emu: &Emulator, output: &mut FFOut) {
        #[allow(unused_must_use)]
        emu.peek_audio_sample(|samples| {
            self.audio_buf.push_slice_overwrite(samples);
            while self.audio_buf.occupied_len() >= self.in_aframe.samples() * 2 {
                let (_, toconvert, _) = unsafe { self.in_aframe.data_mut(0).align_to_mut::<i16>() };
                assert_eq!(self.audio_buf.pop_slice(toconvert), toconvert.len());
                dbg!(toconvert.len());
                self.in_aframe.set_pts(Some(self.audio_frame_in));
                self.out_aframe.set_pts(Some(self.audio_frame_out));
                self.audio_frame_in += i64::try_from(self.in_aframe.samples()).unwrap();
                self.audio_frame_out += i64::try_from(self.out_aframe.samples()).unwrap();
                self.resample(false);
                dbg!(self.out_aframe.samples());
                self.out_audio_enc.send_frame(&self.out_aframe).unwrap();
            }
        });
        self.writeout(output);
    }
    fn drain(&mut self, output: &mut FFOut) {
        while self.audio_buf.occupied_len() > 0 {
            let (_, toconvert, _) = unsafe { self.in_aframe.data_mut(0).align_to_mut::<i16>() };
            let len = self.audio_buf.pop_slice(toconvert);
            toconvert[len..].fill(0);
            self.resample(true);
            self.out_aframe.set_pts(Some(self.audio_frame_out));
            self.audio_frame_out += i64::try_from(len / 2).unwrap();
            self.out_audio_enc.send_frame(&self.out_aframe).unwrap();
        }
        self.out_audio_enc.send_eof().unwrap();
        self.writeout(output);
    }
}

// bobl example: /home/jcoa2018/.cargo/bin/cargo run --manifest-path /home/jcoa2018/Projects/rply-codec/genvideo/Cargo.toml  --bin genvideo examples/bobl.replay examples/bobl.mp4 cores/fceumm_libretro roms/bobl.nes
// ff3 example: /home/jcoa2018/.cargo/bin/cargo run --manifest-path /home/jcoa2018/Projects/rply-codec/genvideo/Cargo.toml  --bin genvideo examples/ff3v2.replay examples/ff3.mp4 cores/snes9x_libretro roms/ff3.nes

fn main() {
    ffmpeg_next::init().unwrap();
    ffmpeg_next::log::set_level(ffmpeg_next::log::Level::Trace);
    let args: Vec<_> = std::env::args().collect();
    let file =
        std::fs::File::open(args.get(1).unwrap_or(&"examples/ff3v2.replay".to_string())).unwrap();
    let outfile = std::path::PathBuf::from(args.get(2).unwrap_or(&"examples/ff3.mp4".to_string()));
    let corefile = args
        .get(3)
        .unwrap_or(&"cores/snes9x_libretro".to_string())
        .clone();
    let romfile = args.get(4).unwrap_or(&"roms/ff3.sfc".to_string()).clone();
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
    let pixel_format = emu.pixel_format();
    assert!(emu.load(&rply.initial_state));

    let mut output = ffmpeg_next::format::output(&outfile).unwrap();
    let emu_video_framerate = emu.get_video_fps().to_i32().unwrap();
    let emu_time_base = Rational::new(1, emu_video_framerate);
    let audio_sample_rate = emu.get_audio_sample_rate().to_i32().unwrap();
    let aspect_ratio = Rational::from(emu.get_aspect_ratio() as f64);
    let mut video_state =
        VideoState::new(emu_time_base, aspect_ratio, w, h, pixel_format, &mut output);
    let mut audio_state = AudioState::new(audio_sample_rate, emu_video_framerate, &mut output);
    output.write_header().unwrap();
    let video_stream_time_base = output.stream(0).unwrap().time_base();
    let audio_stream_time_base = output.stream(1).unwrap().time_base();
    video_state
        .encoded_video
        .set_time_base(video_stream_time_base);
    audio_state
        .encoded_audio
        .set_time_base(audio_stream_time_base);

    let mut frame = Frame::default();
    while let Ok(()) = rply
        .read_frame(&mut frame)
        .inspect_err(|e| println!("Err: {e}"))
    {
        let buttons = frame_to_buttons(&frame);
        emu.run(buttons);
        video_state.send_frame(&emu, rply.frame_number, &mut output);
        audio_state.send_frames(&emu, &mut output);
        if !frame.checkpoint_bytes.is_empty() {
            assert!(emu.load(&frame.checkpoint_bytes));
        }

        if Some(rply.frame_number) == rply.header.frame_count() {
            break;
        }
    }
    audio_state.drain(&mut output);
    video_state.drain(&mut output);
    output.write_trailer().unwrap();
}

fn frame_to_buttons(frame: &Frame) -> [retro_rs::Buttons; 2] {
    use retro_rs::Buttons;
    let mut buttons = [0_i16; 2];
    for inp in &frame.input_events {
        let port = usize::from(inp.port);
        if port < buttons.len() && inp.device == 1 {
            buttons[port] |= inp.val;
        }
    }
    [Buttons::from(buttons[0]), Buttons::from(buttons[1])]
}
