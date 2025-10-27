use thiserror::Error;

#[derive(Debug, Error)]
pub struct InvalidDeterminant(pub u8);
impl std::fmt::Display for InvalidDeterminant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

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
// const HEADER_LEN_BYTES: usize = HeaderV2Part::HeaderLen as usize;

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

#[repr(u8)]
#[non_exhaustive]
#[derive(Debug)]
pub enum SSToken {
    Start = 0,
    NewBlock = 1,
    NewSuperblock = 2,
    SuperblockSeq = 3,
}
impl TryFrom<u8> for SSToken {
    type Error = InvalidDeterminant;

    fn try_from(value: u8) -> std::result::Result<Self, Self::Error> {
        match value {
            0 => Ok(SSToken::Start),
            1 => Ok(SSToken::NewBlock),
            2 => Ok(SSToken::NewSuperblock),
            3 => Ok(SSToken::SuperblockSeq),
            _ => Err(InvalidDeterminant(value)),
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

#[derive(Debug)]
pub struct HeaderBase {
    pub version: u32,
    pub content_crc: u32,
    pub initial_state_size: u32,
    pub identifier: u64,
}

#[derive(Debug)]
pub struct HeaderV2 {
    pub base: HeaderBase,
    pub frame_count: u32,
    pub block_size: u32,
    pub superblock_size: u32,
    pub checkpoint_commit_interval: u8,
    pub checkpoint_commit_threshold: u8,
    pub checkpoint_compression: Compression,
}

#[derive(Debug)]
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
    #[error("Coreless frame read for version 0 not possible")]
    NoCoreRead(),
    #[error("Checkpoint too big {0}")]
    CheckpointTooBig(std::num::TryFromIntError),
    #[error("Invalid frame token {0}")]
    BadFrameToken(u8),
}

type Result<T> = std::result::Result<T, ReplayError>;

pub struct ReplayDecoder<'a, R: std::io::BufRead> {
    rply: &'a mut R,
    pub header: Header,
    pub initial_state: Vec<u8>,
    pub frame_number: usize,
}

impl<R: std::io::BufRead> ReplayDecoder<'_, R> {
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
                frame.checkpoint_raw_bytes.clear();
                frame.checkpoint_uncompressed_raw_bytes.clear();
                frame.checkpoint_uncompressed_unencoded_bytes.clear();
            }
            FrameToken::Checkpoint => {
                frame.checkpoint_compression = Compression::None;
                frame.checkpoint_encoding = Encoding::Raw;
                let cp_size = usize::try_from(rply.read_u64::<LittleEndian>()?)
                    .map_err(ReplayError::CheckpointTooBig)?;
                frame.checkpoint_raw_bytes.resize(cp_size, 0);
                rply.read_exact(frame.checkpoint_raw_bytes.as_mut_slice())?;
            }
            FrameToken::Checkpoint2 => {
                // read a 1 byte compression
                let compression =
                    Compression::try_from(rply.read_u8()?).map_err(ReplayError::Compression)?;
                // read a 1 byte encoding
                let encoding =
                    Encoding::try_from(rply.read_u8()?).map_err(ReplayError::Encoding)?;
                // read a 4 byte uncompressed unencoded size
                let uc_ue_size = rply.read_u32::<LittleEndian>()? as usize;
                // read a 4 byte uncompressed encoded size
                let uc_enc_size = rply.read_u32::<LittleEndian>()? as usize;
                // read a 4 byte compressed encoded size
                let comp_enc_size = rply.read_u32::<LittleEndian>()? as usize;
                // read the compressed encoded data (todo, make a reader instead)
                frame.checkpoint_raw_bytes.resize(comp_enc_size, 0);
                rply.read_exact(frame.checkpoint_raw_bytes.as_mut_slice())?;
                // maybe decompress
                match compression {
                    Compression::None => {}
                    Compression::Zlib => {
                        use flate2::bufread::ZlibDecoder;
                        frame
                            .checkpoint_uncompressed_raw_bytes
                            .resize(uc_enc_size, 0);
                        let mut decoder = ZlibDecoder::new(rply);
                        std::io::copy(
                            &mut decoder,
                            &mut std::io::Cursor::new(
                                frame.checkpoint_uncompressed_raw_bytes.as_mut_slice(),
                            ),
                        )?;
                    }
                    Compression::Zstd => {
                        use zstd::Decoder;
                        frame
                            .checkpoint_uncompressed_raw_bytes
                            .resize(uc_enc_size, 0);
                        let mut decoder = Decoder::with_buffer(rply)?.single_frame();
                        std::io::copy(
                            &mut decoder,
                            &mut std::io::Cursor::new(
                                frame.checkpoint_uncompressed_raw_bytes.as_mut_slice(),
                            ),
                        )?;
                        decoder.finish();
                    }
                }
                // maybe decode
                match encoding {
                    Encoding::Raw => {}
                    Encoding::Statestream => {
                        frame
                            .checkpoint_uncompressed_unencoded_bytes
                            .resize(uc_ue_size, 0);
                        // statestream_decode(frame.checkpoint_decompressed_data().unwrap(), &mut frame.checkpoint_uncompressed_unencoded_bytes);
                    }
                }
            }
        }
        Ok(())
    }
}

/// Creates a [`ReplayDecoder`] for the given buffered readable stream.
///
/// # Errors
/// [`ReplayError::IO`]: Some issue with the read stream, e.g. insufficient length or unexpected end
/// [`ReplayError::Magic`]: Invalid magic number at beginning of file
/// [`ReplayError::Version`]: Version identifier not recognized by parser
/// [`ReplayError::Compression`]: Unsupported compression scheme for checkpoints
pub fn decode<R: std::io::BufRead>(rply: &mut R) -> Result<ReplayDecoder<'_, R>> {
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
        });
    }
    let frame_count = rply.read_u32::<LittleEndian>()?;
    let block_size = rply.read_u32::<LittleEndian>()?;
    let superblock_size = rply.read_u32::<LittleEndian>()?;
    let cp_config = rply.read_u32::<LittleEndian>()?;
    let checkpoint_commit_interval = (cp_config >> 24) as u8;
    let checkpoint_commit_threshold = ((cp_config >> 16) & 0xFF) as u8;
    let checkpoint_compression =
        Compression::try_from(((cp_config >> 8) & 0xFF) as u8).map_err(ReplayError::Compression)?;
    rply.read_exact(initial_state.as_mut_slice())?;
    // TODO: decode if version is 2
    Ok(ReplayDecoder {
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
    })
}
impl Header {
    #[must_use]
    pub fn version(&self) -> u32 {
        match self {
            Header::V0V1(header_base) => header_base.version,
            Header::V2(header_v2) => header_v2.base.version,
        }
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
    checkpoint_raw_bytes: Vec<u8>,
    checkpoint_uncompressed_raw_bytes: Vec<u8>,
    checkpoint_uncompressed_unencoded_bytes: Vec<u8>,
    pub checkpoint_compression: Compression,
    pub checkpoint_encoding: Encoding,
}

impl Frame {
    #[must_use]
    pub fn checkpoint_decompressed_data(&self) -> Option<&[u8]> {
        if self.checkpoint_raw_bytes.is_empty() {
            return None;
        }
        Some(match self.checkpoint_compression {
            Compression::None => self.checkpoint_raw_bytes.as_slice(),
            _ => self.checkpoint_uncompressed_raw_bytes.as_slice(),
        })
    }
    #[must_use]
    pub fn checkpoint_data(&self) -> Option<&[u8]> {
        if self.checkpoint_raw_bytes.is_empty() {
            return None;
        }
        Some(
            match (self.checkpoint_compression, self.checkpoint_encoding) {
                (Compression::None, Encoding::Raw) => self.checkpoint_raw_bytes.as_slice(),
                (_, Encoding::Raw) => self.checkpoint_uncompressed_raw_bytes.as_slice(),
                (_, _) => self.checkpoint_uncompressed_unencoded_bytes.as_slice(),
            },
        )
    }
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            key_events: Vec::default(),
            input_events: Vec::default(),
            checkpoint_raw_bytes: Vec::default(),
            checkpoint_uncompressed_raw_bytes: Vec::default(),
            checkpoint_uncompressed_unencoded_bytes: Vec::default(),
            checkpoint_compression: Compression::None,
            checkpoint_encoding: Encoding::Raw,
        }
    }
}
