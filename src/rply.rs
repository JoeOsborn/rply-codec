use std::io::Write;

use crate::{
    InvalidDeterminant,
    clock::{self, Timer},
    statestream,
};
use thiserror::Error;

// #[repr(usize)]
// pub enum HeaderV0V1Part {
//     Magic = 0,
//     Version = 4,
//     CRC = 8,
//     StateSize = 12,
//     Identifier = 16,
//     HeaderLen = 24,
// }
// #[repr(usize)]
// pub enum HeaderV2Part {
//     FrameCount = 24,
//     BlockSize = 28,
//     SuperblockSize = 32,
//     CheckpointConfig = 36,
//     HeaderLen = 40,
// }
// const HEADER_V0V1_LEN_BYTES: usize = HeaderV0V1Part::HeaderLen as usize;
const HEADERV2_LEN_BYTES: usize = 40;

// const VERSION: u32 = 2;
const MAGIC: u32 = 0x4253_5632;

#[repr(u8)]
#[non_exhaustive]
#[derive(Debug)]
pub enum FrameToken {
    Invalid = 0,
    Regular = b'f',
    Checkpoint = b'c',
    Checkpoint2 = b'C',
}
impl From<u8> for FrameToken {
    fn from(value: u8) -> Self {
        match value {
            b'f' => FrameToken::Regular,
            b'c' => FrameToken::Checkpoint,
            b'C' => FrameToken::Checkpoint2,
            _ => FrameToken::Invalid,
        }
    }
}
impl From<FrameToken> for u8 {
    fn from(value: FrameToken) -> Self {
        match value {
            FrameToken::Invalid => 0,
            FrameToken::Regular => b'f',
            FrameToken::Checkpoint => b'c',
            FrameToken::Checkpoint2 => b'C',
        }
    }
}

#[repr(u8)]
#[non_exhaustive]
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Compression {
    None = 0,
    Zlib = 1,
    Zstd = 2,
}

impl TryFrom<u8> for Compression {
    type Error = InvalidDeterminant;

    fn try_from(value: u8) -> std::result::Result<Self, Self::Error> {
        match value {
            0 => Ok(Compression::None),
            1 => Ok(Compression::Zlib),
            2 => Ok(Compression::Zstd),
            _ => Err(InvalidDeterminant(value)),
        }
    }
}

impl From<Compression> for u8 {
    fn from(value: Compression) -> Self {
        match value {
            Compression::None => 0,
            Compression::Zlib => 1,
            Compression::Zstd => 2,
        }
    }
}

#[repr(u8)]
#[non_exhaustive]
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Encoding {
    Raw = 0,
    Statestream = 1,
}

impl TryFrom<u8> for Encoding {
    type Error = InvalidDeterminant;

    fn try_from(value: u8) -> std::result::Result<Self, Self::Error> {
        match value {
            0 => Ok(Encoding::Raw),
            1 => Ok(Encoding::Statestream),
            _ => Err(InvalidDeterminant(value)),
        }
    }
}

impl From<Encoding> for u8 {
    fn from(value: Encoding) -> Self {
        match value {
            Encoding::Raw => 0,
            Encoding::Statestream => 1,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HeaderBase {
    pub version: u32,
    pub content_crc: u32,
    pub initial_state_size: u32,
    pub identifier: u64,
}

#[derive(Debug, Clone)]
pub struct HeaderV2 {
    pub base: HeaderBase,
    pub frame_count: u32,
    pub block_size: u32,
    pub superblock_size: u32,
    pub checkpoint_commit_interval: u8,
    pub checkpoint_commit_threshold: u8,
    pub checkpoint_compression: Compression,
}

#[derive(Debug, Clone)]
pub enum Header {
    V0V1(HeaderBase),
    V2(HeaderV2),
}

#[derive(Error, Debug)]
pub enum ReplayError {
    #[error("Invalid replay magic {0}")]
    Magic(u32),
    #[error("Unsupported version {0}")]
    Version(u32),
    #[error("Unsupported compression scheme {0}")]
    Compression(InvalidDeterminant),
    #[error("Unsupported encoding scheme {0}")]
    Encoding(InvalidDeterminant),
    #[error("I/O Error")]
    IO(#[from] std::io::Error),
    #[error("Too many frames to {0} fit framecount header")]
    TooManyFrames(std::num::TryFromIntError),
    #[error("Coreless frame read for version 0 not possible")]
    NoCoreRead(),
    #[error("Checkpoint too big {0}")]
    CheckpointTooBig(std::num::TryFromIntError),
    #[error("Frame too long {0}")]
    FrameTooLong(std::num::TryFromIntError),
    #[error("Frame has too many key events {0}")]
    TooManyKeyEvents(std::num::TryFromIntError),
    #[error("Frame has too many input events {0}")]
    TooManyInputEvents(std::num::TryFromIntError),
    #[error("Invalid frame token {0}")]
    BadFrameToken(u8),
}

type Result<T> = std::result::Result<T, ReplayError>;

pub struct ReplayDecoder<'a, R: std::io::BufRead> {
    rply: &'a mut R,
    pub header: Header,
    pub initial_state: Vec<u8>,
    pub frame_number: u64,
    ss_state: statestream::Ctx,
}

impl<R: std::io::BufRead> ReplayDecoder<'_, R> {
    /// Creates a [`ReplayDecoder`] for the given buffered readable stream.
    ///
    /// # Errors
    /// [`ReplayError::IO`]: Some issue with the read stream, e.g. insufficient length or unexpected end
    /// [`ReplayError::Magic`]: Invalid magic number at beginning of file
    /// [`ReplayError::Version`]: Version identifier not recognized by parser
    /// [`ReplayError::Compression`]: Unsupported compression scheme for checkpoints
    pub fn new(rply: &mut R) -> Result<ReplayDecoder<'_, R>> {
        use byteorder::{LittleEndian, ReadBytesExt};
        let magic = rply.read_u32::<LittleEndian>()?;
        if magic != MAGIC {
            return Err(ReplayError::Magic(magic));
        }
        let version = rply.read_u32::<LittleEndian>()?;
        if version > 2 {
            return Err(ReplayError::Version(version));
        }
        let content_crc = rply.read_u32::<LittleEndian>()?;
        let initial_state_size = rply.read_u32::<LittleEndian>()?;
        let identifier = rply.read_u64::<LittleEndian>()?;
        let base = HeaderBase {
            version,
            content_crc,
            initial_state_size,
            identifier,
        };
        let mut initial_state = vec![0; initial_state_size as usize];
        if version < 2 {
            rply.read_exact(initial_state.as_mut_slice())?;
            return Ok(ReplayDecoder {
                header: Header::V0V1(base),
                rply,
                initial_state,
                frame_number: 0,
                ss_state: statestream::Ctx::new(1, 1),
            });
        }
        let frame_count = rply.read_u32::<LittleEndian>()?;
        let block_size = rply.read_u32::<LittleEndian>()?;
        let superblock_size = rply.read_u32::<LittleEndian>()?;
        let cp_config = rply.read_u32::<LittleEndian>()?;
        let checkpoint_commit_interval = (cp_config >> 24) as u8;
        let checkpoint_commit_threshold = ((cp_config >> 16) & 0xFF) as u8;
        let checkpoint_compression = Compression::try_from(((cp_config >> 8) & 0xFF) as u8)
            .map_err(ReplayError::Compression)?;
        let mut replay = ReplayDecoder {
            rply,
            initial_state,
            header: Header::V2(HeaderV2 {
                base,
                frame_count,
                block_size,
                superblock_size,
                checkpoint_commit_interval,
                checkpoint_commit_threshold,
                checkpoint_compression,
            }),
            frame_number: 0,
            ss_state: statestream::Ctx::new(block_size, superblock_size),
        };
        if replay.header.version() == 1 {
            replay.rply.read_exact(&mut replay.initial_state)?;
        } else {
            replay.decode_initial_checkpoint()?;
        }
        Ok(replay)
    }

    /// Reads a single frame at the current decoder position.
    /// # Errors
    /// [`ReplayError::IO`]: Unexpected end of stream or other I/O error
    /// [`ReplayError::Compression`]: Unsupported compression scheme
    /// [`ReplayError::Encoding`]: Unsupported encoding scheme
    /// [`ReplayError::BadFrameToken`]: Frame token not recognized or misaligned
    /// [`ReplayError::NoCoreRead`]: Tried to read a frame on a version 0 replay without a loaded core
    /// [`ReplayError::CheckpointTooBig`]: Tried to read a checkpoint bigger than the address space
    #[allow(clippy::too_many_lines)]
    pub fn read_frame(&mut self, frame: &mut Frame) -> Result<()> {
        use byteorder::{LittleEndian, ReadBytesExt};
        let stopwatch = clock::time(Timer::DecodeFrame);
        let vsn = self.header.version();
        let rply = &mut *self.rply;
        if vsn == 0 {
            return Err(ReplayError::NoCoreRead());
        }
        if vsn > 1 {
            /* skip over the backref */
            let _ = rply.read_u32::<LittleEndian>()?;
        }
        let key_count = rply.read_u8()? as usize;
        frame.key_events.resize_with(key_count, Default::default);
        for ki in 0..key_count {
            /*
            down, padding, mod_x2, code_x4, char_x4
             */
            let down = rply.read_u8()?;
            let _ = rply.read_u8()?; // padding
            let modf = rply.read_u16::<LittleEndian>()?;
            let code = rply.read_u32::<LittleEndian>()?;
            let chr = rply.read_u32::<LittleEndian>()?;
            let key_data = KeyData {
                down,
                /* buf[1] is padding */
                modf,
                code,
                chr,
            };
            frame.key_events[ki] = key_data;
        }
        let input_count = rply.read_u16::<LittleEndian>()? as usize;
        frame
            .input_events
            .resize_with(input_count, Default::default);
        for ii in 0..input_count {
            /* port, device, idx, padding, id_x2, value_x2 */
            let port = rply.read_u8()?;
            let device = rply.read_u8()?;
            let idx = rply.read_u8()?;
            let _ = rply.read_u8()?;
            let id = rply.read_u16::<LittleEndian>()?;
            let val = rply.read_i16::<LittleEndian>()?;
            let inp_data = InputData {
                port,
                device,
                idx,
                id,
                val,
            };
            frame.input_events[ii] = inp_data;
        }
        let tok = rply.read_u8()?;
        match FrameToken::from(tok) {
            FrameToken::Invalid => return Err(ReplayError::BadFrameToken(tok)),
            FrameToken::Regular => {
                frame.checkpoint_compression = Compression::None;
                frame.checkpoint_encoding = Encoding::Raw;
                frame.checkpoint_bytes.clear();
            }
            FrameToken::Checkpoint => {
                frame.checkpoint_compression = Compression::None;
                frame.checkpoint_encoding = Encoding::Raw;
                let cp_size = usize::try_from(rply.read_u64::<LittleEndian>()?)
                    .map_err(ReplayError::CheckpointTooBig)?;
                frame.checkpoint_bytes.resize(cp_size, 0);
                rply.read_exact(frame.checkpoint_bytes.as_mut_slice())?;
            }
            FrameToken::Checkpoint2 => {
                self.decode_checkpoint(&mut frame.checkpoint_bytes)?;
            }
        }
        self.frame_number += 1;
        drop(stopwatch);
        Ok(())
    }

    fn decode_initial_checkpoint(&mut self) -> Result<()> {
        let mut initial_state = std::mem::take(&mut self.initial_state);
        self.decode_checkpoint(&mut initial_state)?;
        self.initial_state = initial_state;
        Ok(())
    }

    fn decode_checkpoint(&mut self, checkpoint_bytes: &mut Vec<u8>) -> Result<()> {
        use byteorder::{LittleEndian, ReadBytesExt};
        let stopwatch = clock::time(Timer::DecodeCheckpoint);
        let rply = &mut *self.rply;
        // read a 1 byte compression code
        let compression =
            Compression::try_from(rply.read_u8()?).map_err(ReplayError::Compression)?;
        // read a 1 byte encoding code
        let encoding = Encoding::try_from(rply.read_u8()?).map_err(ReplayError::Encoding)?;
        // read a 4 byte uncompressed unencoded size
        let uc_ue_size = rply.read_u32::<LittleEndian>()? as usize;
        // read a 4 byte uncompressed encoded size
        #[expect(unused)]
        let uc_enc_size = rply.read_u32::<LittleEndian>()? as usize;
        // read a 4 byte compressed encoded size
        #[expect(unused)]
        let comp_enc_size = rply.read_u32::<LittleEndian>()? as usize;
        checkpoint_bytes.resize(uc_ue_size, 0);
        // maybe decompress
        match (compression, encoding) {
            (Compression::None, Encoding::Raw) => {
                rply.read_exact(checkpoint_bytes.as_mut_slice())?;
            }
            (Compression::None, Encoding::Statestream) => {
                let mut ss_decoder =
                    statestream::Decoder::new(rply, &mut self.ss_state, uc_ue_size);
                std::io::copy(
                    &mut ss_decoder,
                    &mut std::io::Cursor::new(checkpoint_bytes.as_mut_slice()),
                )?;
            }
            (Compression::Zlib, Encoding::Raw) => {
                use flate2::bufread::ZlibDecoder;
                let mut decoder = ZlibDecoder::new(rply);
                std::io::copy(
                    &mut decoder,
                    &mut std::io::Cursor::new(checkpoint_bytes.as_mut_slice()),
                )?;
            }
            (Compression::Zlib, Encoding::Statestream) => {
                use flate2::bufread::ZlibDecoder;
                let mut decoder = ZlibDecoder::new(rply);
                let mut ss_decoder =
                    statestream::Decoder::new(&mut decoder, &mut self.ss_state, uc_ue_size);
                std::io::copy(
                    &mut ss_decoder,
                    &mut std::io::Cursor::new(checkpoint_bytes.as_mut_slice()),
                )?;
            }
            (Compression::Zstd, Encoding::Raw) => {
                use zstd::Decoder;
                let mut decoder = Decoder::with_buffer(rply)?.single_frame();
                std::io::copy(
                    &mut decoder,
                    &mut std::io::Cursor::new(checkpoint_bytes.as_mut_slice()),
                )?;
            }
            (Compression::Zstd, Encoding::Statestream) => {
                use zstd::Decoder;
                let mut decoder = Decoder::with_buffer(rply)?.single_frame();
                let mut ss_decoder =
                    statestream::Decoder::new(&mut decoder, &mut self.ss_state, uc_ue_size);
                std::io::copy(
                    &mut ss_decoder,
                    &mut std::io::Cursor::new(checkpoint_bytes.as_mut_slice()),
                )?;
            }
        }
        drop(stopwatch);
        Ok(())
    }
}

/// Creates a [`ReplayDecoder`] for the given buffered readable stream.
///
/// # Errors
/// See [`ReplayDecoder::new`].
pub fn decode<R: std::io::BufRead>(rply: &mut R) -> Result<ReplayDecoder<'_, R>> {
    ReplayDecoder::new(rply)
}

pub struct ReplayEncoder<'a, W: std::io::Write + std::io::Seek> {
    rply: &'a mut W,
    pub header: Header,
    pub frame_number: u64,
    last_pos: u64,
    ss_state: statestream::Ctx,
    finished: bool,
}

impl<'w, W: std::io::Write + std::io::Seek> ReplayEncoder<'w, W> {
    /// Creates a [`ReplayEncoder`] for the given writable and seekable stream.
    ///
    /// # Errors
    /// [`ReplayError::IO`]: Some issue with the write stream, e.g. unexpected end
    /// [`ReplayError::Version`]: Version identifier not supported by writer
    /// [`ReplayError::Compression`]: Unsupported compression scheme for checkpoints
    pub fn new<'s>(
        header: Header,
        initial_state: &'s [u8],
        rply: &'w mut W,
    ) -> Result<ReplayEncoder<'w, W>> {
        if header.version() != 2 {
            return Err(ReplayError::Version(header.version()));
        }
        let ss_state = statestream::Ctx::new(header.block_size(), header.superblock_size());
        let mut replay = ReplayEncoder {
            rply,
            header,
            frame_number: 0,
            last_pos: 0,
            ss_state,
            finished: false,
        };
        replay.write_header()?;
        if !initial_state.is_empty() {
            replay.encode_initial_checkpoint(initial_state)?;
        }
        replay.last_pos = replay.rply.stream_position()?;
        Ok(replay)
    }
    fn write_header(&mut self) -> Result<()> {
        use byteorder::{LittleEndian, WriteBytesExt};
        self.header
            .set_frame_count(u32::try_from(self.frame_number).unwrap_or_default());
        let old_pos = self.rply.stream_position()?;
        self.rply.seek(std::io::SeekFrom::Start(0))?;
        self.rply.write_u32::<LittleEndian>(MAGIC)?;
        self.rply.write_u32::<LittleEndian>(2)?;
        self.rply
            .write_u32::<LittleEndian>(self.header.content_crc())?;
        // state size
        self.rply
            .write_u32::<LittleEndian>(self.header.initial_state_size())?;
        self.rply
            .write_u64::<LittleEndian>(self.header.identifier())?;
        self.rply.write_u32::<LittleEndian>(
            u32::try_from(self.header.frame_count().unwrap())
                .map_err(ReplayError::TooManyFrames)?,
        )?;
        self.rply
            .write_u32::<LittleEndian>(self.header.block_size())?;
        self.rply
            .write_u32::<LittleEndian>(self.header.superblock_size())?;
        let cp_interval = u32::from(self.header.checkpoint_commit_interval());
        let cp_threshold = u32::from(self.header.checkpoint_commit_threshold());
        let cp_compression = u32::from(u8::from(self.header.checkpoint_compression()));
        self.rply.write_u32::<LittleEndian>(
            (cp_interval << 24) | (cp_threshold << 16) | (cp_compression << 8),
        )?;
        self.rply.seek(std::io::SeekFrom::Start(old_pos))?;
        Ok(())
    }
    fn encode_checkpoint(&mut self, checkpoint: &[u8], frame: u64) -> Result<()> {
        use byteorder::{LittleEndian, WriteBytesExt};
        let stopwatch = clock::time(Timer::EncodeCheckpoint);
        let compression = self.header.checkpoint_compression();
        let encoding = Encoding::Statestream;
        self.rply.write_u8(u8::from(compression))?;
        self.rply.write_u8(u8::from(encoding))?;
        // write unencoded uncompressed size
        let full_size = u32::try_from(checkpoint.len()).map_err(ReplayError::CheckpointTooBig)?;
        self.rply.write_u32::<LittleEndian>(full_size)?;
        let size_pos = self.rply.stream_position()?;
        // can't yet write encoded uncompressed size, just write zeros for now
        // write encoded compressed size
        self.rply.write_u32::<LittleEndian>(0)?;
        // write encoded compressed bytes
        self.rply.write_u32::<LittleEndian>(0)?;
        let (encoded_size, compressed_size) = match (compression, encoding) {
            (Compression::None, Encoding::Raw) => {
                self.rply.write_all(checkpoint)?;
                (full_size, full_size)
            }
            (Compression::None, Encoding::Statestream) => {
                let encoder = statestream::Encoder::new(&mut self.rply, &mut self.ss_state);
                let encoded_size = encoder.encode_checkpoint(checkpoint, frame)?;
                (encoded_size, encoded_size)
            }
            (Compression::Zlib, Encoding::Raw) => {
                use flate2::write::ZlibEncoder;
                let here_pos = self.rply.stream_position()?;
                let mut encoder = ZlibEncoder::new(&mut self.rply, flate2::Compression::default());
                let encoded_size = full_size;
                encoder.write_all(checkpoint)?;
                encoder.finish()?;
                let compressed_size = u32::try_from(self.rply.stream_position()? - here_pos)
                    .map_err(ReplayError::CheckpointTooBig)?;
                (encoded_size, compressed_size)
            }
            (Compression::Zlib, Encoding::Statestream) => {
                use flate2::write::ZlibEncoder;
                let here_pos = self.rply.stream_position()?;
                let mut compressor =
                    ZlibEncoder::new(&mut self.rply, flate2::Compression::default());
                let encoder = statestream::Encoder::new(&mut compressor, &mut self.ss_state);
                let encoded_size = encoder.encode_checkpoint(checkpoint, frame)?;
                compressor.finish()?;
                let compressed_size = u32::try_from(self.rply.stream_position()? - here_pos)
                    .map_err(ReplayError::CheckpointTooBig)?;
                (encoded_size, compressed_size)
            }
            (Compression::Zstd, Encoding::Raw) => {
                let here_pos = self.rply.stream_position()?;
                let mut encoder = zstd::Encoder::new(&mut self.rply, 16)?;
                encoder.write_all(checkpoint)?;
                encoder.finish()?;
                let encoded_size = full_size;
                let compressed_size = u32::try_from(self.rply.stream_position()? - here_pos)
                    .map_err(ReplayError::CheckpointTooBig)?;
                (encoded_size, compressed_size)
            }
            (Compression::Zstd, Encoding::Statestream) => {
                let here_pos = self.rply.stream_position()?;
                let mut compressor = zstd::Encoder::new(&mut self.rply, 16)?;
                let encoder = statestream::Encoder::new(&mut compressor, &mut self.ss_state);
                let encoded_size = encoder.encode_checkpoint(checkpoint, frame)?;
                compressor.finish()?;
                let compressed_size = u32::try_from(self.rply.stream_position()? - here_pos)
                    .map_err(ReplayError::CheckpointTooBig)?;
                (encoded_size, compressed_size)
            }
        };
        let end_pos = self.rply.stream_position()?;
        self.rply.seek(std::io::SeekFrom::Start(size_pos))?;
        // write encoded compressed size
        self.rply.write_u32::<LittleEndian>(encoded_size)?;
        // write encoded compressed bytes
        self.rply.write_u32::<LittleEndian>(compressed_size)?;
        self.rply.seek(std::io::SeekFrom::Start(end_pos))?;
        drop(stopwatch);
        Ok(())
    }
    fn encode_initial_checkpoint(&mut self, checkpoint: &[u8]) -> Result<()> {
        self.rply
            .seek(std::io::SeekFrom::Start(HEADERV2_LEN_BYTES as u64))?;
        self.encode_checkpoint(checkpoint, 0)?;
        let encoded_size = self.rply.stream_position()? - HEADERV2_LEN_BYTES as u64;
        self.header.set_initial_state_size(
            u32::try_from(encoded_size).map_err(ReplayError::CheckpointTooBig)?,
        );
        // Have to rewrite header to account for initial state size
        self.write_header()?;
        self.last_pos = self.rply.stream_position()?;
        Ok(())
    }

    /// Writes a single frame at the current encoder position.
    /// # Errors
    /// [`ReplayError::FrameTooLong`]: Frame encoded to more than 2^32 bytes, backrefs invalid
    /// [`ReplayError::TooManyKeyEvents`]: More key events than allowed by spec
    /// [`ReplayError::TooManyInputEvents`]: More input events than allowed by spec
    /// [`ReplayError::CheckpointTooBig`]: Checkpoint data takes up more than 2^32 bytes
    pub fn write_frame(&mut self, frame: &Frame) -> Result<()> {
        use byteorder::{LittleEndian, WriteBytesExt};
        let stopwatch = clock::time(Timer::EncodeFrame);
        let start_pos = self.rply.stream_position()?;
        self.rply.write_u32::<LittleEndian>(
            u32::try_from(start_pos - self.last_pos).map_err(ReplayError::FrameTooLong)?,
        )?;
        self.rply.write_u8(
            u8::try_from(frame.key_events.len()).map_err(ReplayError::TooManyKeyEvents)?,
        )?;
        for evt in &frame.key_events {
            self.rply.write_u8(evt.down)?;
            self.rply.write_u8(0)?; // padding
            self.rply.write_u16::<LittleEndian>(evt.modf)?;
            self.rply.write_u32::<LittleEndian>(evt.code)?;
            self.rply.write_u32::<LittleEndian>(evt.chr)?;
        }
        self.rply.write_u16::<LittleEndian>(
            u16::try_from(frame.input_events.len()).map_err(ReplayError::TooManyInputEvents)?,
        )?;
        for evt in &frame.input_events {
            self.rply.write_u8(evt.port)?;
            self.rply.write_u8(evt.device)?;
            self.rply.write_u8(evt.idx)?;
            self.rply.write_u8(0)?; // padding
            self.rply.write_u16::<LittleEndian>(evt.id)?;
            self.rply.write_i16::<LittleEndian>(evt.val)?;
        }
        if frame.checkpoint_bytes.is_empty() {
            self.rply.write_u8(u8::from(FrameToken::Regular))?;
        } else {
            self.rply.write_u8(u8::from(FrameToken::Checkpoint2))?;
            self.encode_checkpoint(&frame.checkpoint_bytes, self.frame_number)?;
        }
        self.frame_number += 1;
        self.last_pos = start_pos;
        drop(stopwatch);
        Ok(())
    }
    /// Finishes the encoding, writing the header in the process
    /// # Errors
    /// [`ReplayError::IO`]: Underlying writer fails to write header
    pub fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.write_header()?;
        self.finished = true;
        Ok(())
    }
}

impl<W: std::io::Write + std::io::Seek> Drop for ReplayEncoder<'_, W> {
    fn drop(&mut self) {
        self.finish().unwrap();
    }
}

/// Creates a [`ReplayEncoder`] for the given writable & seekable stream.
///
/// # Errors
/// See [`ReplayEncoder::new`].
pub fn encode<'w, W: std::io::Write + std::io::Seek>(
    header: Header,
    initial_state: &[u8],
    rply: &'w mut W,
) -> Result<ReplayEncoder<'w, W>> {
    ReplayEncoder::new(header, initial_state, rply)
}

impl Header {
    fn base(&self) -> &HeaderBase {
        match self {
            Header::V0V1(header_base) => header_base,
            Header::V2(header_v2) => &header_v2.base,
        }
    }
    fn base_mut(&mut self) -> &mut HeaderBase {
        match self {
            Header::V0V1(header_base) => header_base,
            Header::V2(header_v2) => &mut header_v2.base,
        }
    }
    #[must_use]
    pub fn version(&self) -> u32 {
        self.base().version
    }
    #[must_use]
    pub fn content_crc(&self) -> u32 {
        self.base().content_crc
    }
    pub fn set_content_crc(&mut self, crc: u32) {
        self.base_mut().content_crc = crc;
    }
    #[must_use]
    pub fn identifier(&self) -> u64 {
        self.base().identifier
    }
    pub fn set_identifier(&mut self, id: u64) {
        self.base_mut().identifier = id;
    }
    #[must_use]
    pub fn initial_state_size(&self) -> u32 {
        self.base().initial_state_size
    }
    pub fn set_initial_state_size(&mut self, sz: u32) {
        self.base_mut().initial_state_size = sz;
    }
    #[must_use]
    pub fn frame_count(&self) -> Option<u64> {
        match self {
            Header::V0V1(_) => None,
            Header::V2(header_v2) => Some(u64::from(header_v2.frame_count)),
        }
    }
    pub fn set_frame_count(&mut self, frames: u32) {
        self.upgrade().frame_count = frames;
    }
    pub fn upgrade(&mut self) -> &mut HeaderV2 {
        if let Header::V0V1(base) = self {
            *self = Header::V2(HeaderV2 {
                base: base.clone(),
                frame_count: 0,
                block_size: 0,
                superblock_size: 0,
                checkpoint_commit_interval: 0,
                checkpoint_commit_threshold: 0,
                checkpoint_compression: Compression::None,
            });
        }
        let Header::V2(v2) = self else { unreachable!() };
        v2
    }
    #[must_use]
    pub fn block_size(&self) -> u32 {
        match self {
            Header::V0V1(_) => 0,
            Header::V2(header_v2) => header_v2.block_size,
        }
    }
    pub fn set_block_size(&mut self, sz: u32) {
        let v2 = self.upgrade();
        v2.block_size = sz;
    }
    #[must_use]
    pub fn superblock_size(&self) -> u32 {
        match self {
            Header::V0V1(_) => 0,
            Header::V2(header_v2) => header_v2.superblock_size,
        }
    }
    pub fn set_superblock_size(&mut self, sz: u32) {
        let v2 = self.upgrade();
        v2.superblock_size = sz;
    }
    #[must_use]
    pub fn checkpoint_commit_interval(&self) -> u8 {
        match self {
            Header::V0V1(_) => 0,
            Header::V2(header_v2) => header_v2.checkpoint_commit_interval,
        }
    }
    #[must_use]
    pub fn checkpoint_commit_threshold(&self) -> u8 {
        match self {
            Header::V0V1(_) => 0,
            Header::V2(header_v2) => header_v2.checkpoint_commit_threshold,
        }
    }
    pub fn set_checkpoint_commit_settings(&mut self, interval: u8, threshold: u8) {
        let v2 = self.upgrade();
        v2.checkpoint_commit_interval = interval;
        v2.checkpoint_commit_threshold = threshold;
    }
    #[must_use]
    pub fn checkpoint_compression(&self) -> Compression {
        match self {
            Header::V0V1(_) => Compression::None,
            Header::V2(header_v2) => header_v2.checkpoint_compression,
        }
    }
    pub fn set_checkpoint_compression(&mut self, compression: Compression) {
        let v2 = self.upgrade();
        v2.checkpoint_compression = compression;
    }
}
#[derive(Debug, Default)]
pub struct KeyData {
    pub down: u8,
    pub modf: u16,
    pub code: u32,
    pub chr: u32,
}
#[derive(Debug, Default)]
pub struct InputData {
    pub port: u8,
    pub device: u8,
    pub idx: u8,
    pub id: u16,
    pub val: i16,
}

#[derive(Debug)]
pub struct Frame {
    pub key_events: Vec<KeyData>,
    pub input_events: Vec<InputData>,
    pub checkpoint_bytes: Vec<u8>,
    pub checkpoint_compression: Compression,
    pub checkpoint_encoding: Encoding,
}

impl Frame {
    #[must_use]
    pub fn inputs(&self) -> String {
        use std::fmt::Write;
        let mut output = String::new();
        for i in 0..self.input_events.len() {
            let evt = &self.input_events[i];
            write!(output, "{:03}:{:016b}", evt.id, evt.val).unwrap();
            if i + 1 < self.input_events.len() {
                write!(output, "--").unwrap();
            }
        }
        output
    }
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            key_events: Vec::default(),
            input_events: Vec::default(),
            checkpoint_bytes: Vec::default(),
            checkpoint_compression: Compression::None,
            checkpoint_encoding: Encoding::Raw,
        }
    }
}
