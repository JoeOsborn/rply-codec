use thiserror::Error;

#[derive(Debug, Error)]
pub struct InvalidDeterminant(pub u8);
impl std::fmt::Display for InvalidDeterminant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[repr(usize)]
pub enum HeaderV0V1Part {
    Magic = 0,
    Version = 4,
    CRC = 8,
    StateSize = 12,
    Identifier = 16,
    HeaderLen = 24,
}
#[repr(usize)]
enum HeaderV2Part {
    FrameCount = 24,
    BlockSize = 28,
    SuperblockSize = 32,
    CheckpointConfig = 36,
    HeaderLen = 40,
}
const HEADER_V0V1_LEN_BYTES: usize = HeaderV0V1Part::HeaderLen as usize;
const HEADER_LEN_BYTES: usize = HeaderV2Part::HeaderLen as usize;

const VERSION: u32 = 2;
const MAGIC: u32 = 0x42535632;

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
    Compression(#[from] InvalidDeterminant),
    #[error("I/O Error")]
    IO(#[from] std::io::Error),
    #[error("Coreless frame read for version 0 not possible")]
    NoCoreRead(),
    #[error("Invalid frame token {0}")]
    BadFrameToken(u8),
}

type Result<T> = std::result::Result<T, ReplayError>;

pub fn read_header(rply: &mut impl std::io::BufRead) -> Result<Header> {
    let mut bytes = [0; HEADER_LEN_BYTES];
    rply.read_exact(&mut bytes[0..HEADER_V0V1_LEN_BYTES])?;
    // These unwraps are safe because if I can take e.g. a slice of length 4, I already have a 4-byte value.
    // And I know I can take those slices because read_exact read exactly 24 bytes.
    let magic = u32::from_le_bytes(
        <[u8; 4]>::try_from(
            &bytes[(HeaderV0V1Part::Magic as usize)..(HeaderV0V1Part::Magic as usize + 4)],
        )
        .unwrap(),
    );
    if magic != MAGIC {
        return Err(ReplayError::Magic(magic));
    }
    let version = u32::from_le_bytes(
        <[u8; 4]>::try_from(
            &bytes[(HeaderV0V1Part::Version as usize)..(HeaderV0V1Part::Version as usize + 4)],
        )
        .unwrap(),
    );
    if version > 2 {
        return Err(ReplayError::Version(version));
    }
    let content_crc = u32::from_le_bytes(
        <[u8; 4]>::try_from(
            &bytes[(HeaderV0V1Part::CRC as usize)..(HeaderV0V1Part::CRC as usize + 4)],
        )
        .unwrap(),
    );
    let initial_state_size = u32::from_le_bytes(
        <[u8; 4]>::try_from(
            &bytes[(HeaderV0V1Part::StateSize as usize)..(HeaderV0V1Part::StateSize as usize + 4)],
        )
        .unwrap(),
    );
    let identifier = u64::from_le_bytes(
        <[u8; 8]>::try_from(
            &bytes
                [(HeaderV0V1Part::Identifier as usize)..(HeaderV0V1Part::Identifier as usize + 8)],
        )
        .unwrap(),
    );
    let base = HeaderBase {
        version,
        content_crc,
        initial_state_size,
        identifier,
    };
    if version < 2 {
        return Ok(Header::V0V1(base));
    }
    rply.read_exact(&mut bytes[HEADER_V0V1_LEN_BYTES..HEADER_LEN_BYTES])?;
    let frame_count = u32::from_le_bytes(
        <[u8; 4]>::try_from(
            &bytes[(HeaderV2Part::FrameCount as usize)..(HeaderV2Part::FrameCount as usize + 4)],
        )
        .unwrap(),
    );
    let block_size = u32::from_le_bytes(
        <[u8; 4]>::try_from(
            &bytes[(HeaderV2Part::BlockSize as usize)..(HeaderV2Part::BlockSize as usize + 4)],
        )
        .unwrap(),
    );
    let superblock_size = u32::from_le_bytes(
        <[u8; 4]>::try_from(
            &bytes[(HeaderV2Part::SuperblockSize as usize)
                ..(HeaderV2Part::SuperblockSize as usize + 4)],
        )
        .unwrap(),
    );
    let cp_config = u32::from_le_bytes(
        <[u8; 4]>::try_from(
            &bytes[(HeaderV2Part::CheckpointConfig as usize)
                ..(HeaderV2Part::CheckpointConfig as usize + 4)],
        )
        .unwrap(),
    );
    let checkpoint_commit_interval = (cp_config >> 24) as u8;
    let checkpoint_commit_threshold = ((cp_config >> 16) & 0xFF) as u8;
    let checkpoint_compression = Compression::try_from(((cp_config >> 8) & 0xFF) as u8)?;
    Ok(Header::V2(HeaderV2 {
        base,
        frame_count,
        block_size,
        superblock_size,
        checkpoint_commit_interval,
        checkpoint_commit_threshold,
        checkpoint_compression,
    }))
}
impl Header {
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
    pub fn checkpoint_decompressed_data(&self) -> Option<&[u8]> {
        if self.checkpoint_raw_bytes.is_empty() {
            return None;
        }
        Some(match self.checkpoint_compression {
            Compression::None => self.checkpoint_raw_bytes.as_slice(),
            _ => self.checkpoint_uncompressed_raw_bytes.as_slice(),
        })
    }
    pub fn checkpoint_data(&self) -> Option<&[u8]> {
        if self.checkpoint_raw_bytes.is_empty() {
            return None;
        }
        Some(match (self.checkpoint_compression, self.checkpoint_encoding) {
            (Compression::None, Encoding::Raw) => self.checkpoint_raw_bytes.as_slice(),
            (_, Encoding::Raw) => self.checkpoint_uncompressed_raw_bytes.as_slice(),
            (_, _) => self.checkpoint_uncompressed_unencoded_bytes.as_slice()
        })
    }
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            key_events: Default::default(),
            input_events: Default::default(),
            checkpoint_raw_bytes: Default::default(),
            checkpoint_uncompressed_raw_bytes: Default::default(),
            checkpoint_uncompressed_unencoded_bytes: Default::default(),
            checkpoint_compression: Compression::None,
            checkpoint_encoding: Encoding::Raw,
        }
    }
}
/* TODO instead of Header use ReplayContext and have it include the statestream indices if needed */
pub fn read_frame(rply: &mut impl std::io::BufRead, header: &Header, frame: &mut Frame) -> Result<()> {
    let vsn = header.version();
    if vsn == 0 {
        return Err(ReplayError::NoCoreRead());
    }
    let mut buf: [u8; 16] = [0; 16];
    if vsn > 1 {
        /* skip over the backref */
        rply.read_exact(&mut buf)?;
    }
    rply.read_exact(&mut buf[0..1])?;
    let key_count = buf[0] as usize;
    frame.key_events.resize_with(key_count, Default::default);
    for ki in 0..key_count {
        rply.read_exact(&mut buf[0..12])?;
        /*
        down, padding, mod_x2, code_x4, char_x4
         */
        let key_data = KeyData {
            down: buf[0],
            /* buf[1] is padding */
            modf: u16::from_le_bytes(<[u8; 2]>::try_from(&buf[2..4]).unwrap()),
            code: u32::from_le_bytes(<[u8; 4]>::try_from(&buf[4..8]).unwrap()),
            chr: u32::from_le_bytes(<[u8; 4]>::try_from(&buf[8..12]).unwrap()),
        };
        frame.key_events[ki] = key_data;
    }
    let input_count = u16::from_le_bytes(<[u8; 2]>::try_from(&buf[0..2]).unwrap()) as usize;
    frame
        .input_events
        .resize_with(input_count, Default::default);
    for ii in 0..input_count {
        rply.read_exact(&mut buf[0..8])?;
        /* port, device, idx, padding, id_x2, value_x2 */
        let inp_data = InputData {
            port: buf[0],
            device: buf[1],
            idx: buf[2],
            /* buf[3] is padding */
            id: u16::from_le_bytes(<[u8; 2]>::try_from(&buf[4..6]).unwrap()),
            val: i16::from_le_bytes(<[u8; 2]>::try_from(&buf[6..8]).unwrap()),
        };
        frame.input_events[ii] = inp_data;
    }
    rply.read_exact(&mut buf[0..1])?;
    match FrameToken::from(buf[0]) {
        FrameToken::Invalid => return Err(ReplayError::BadFrameToken(buf[0])),
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
            rply.read_exact(&mut buf[0..8])?;
            let cp_size = usize::try_from(u64::from_le_bytes(<[u8; 8]>::try_from(&buf[0..8]).unwrap())).unwrap();
            frame.checkpoint_raw_bytes.resize(cp_size, 0);
            rply.read_exact(frame.checkpoint_raw_bytes.as_mut_slice())?;
        }
        FrameToken::Checkpoint2 => {
            rply.read_exact(&mut buf[0..14])?;
            // read a 1 byte compression
            let compression = Compression::try_from(buf[0])?;
            // read a 1 byte encoding
            let encoding = Encoding::try_from(buf[1])?;
            // read a 4 byte uncompressed unencoded size
            let uc_ue_size = u32::from_le_bytes(<[u8; 4]>::try_from(&buf[2..6]).unwrap()) as usize;
            // read a 4 byte uncompressed encoded size
            let uc_enc_size = u32::from_le_bytes(<[u8; 4]>::try_from(&buf[6..10]).unwrap()) as usize;
            // read a 4 byte compressed encoded size
            let comp_enc_size = u32::from_le_bytes(<[u8; 4]>::try_from(&buf[10..14]).unwrap()) as usize;
            // read the compressed encoded data
            frame.checkpoint_raw_bytes.resize(comp_enc_size, 0);
            rply.read_exact(frame.checkpoint_raw_bytes.as_mut_slice())?;
            // maybe decompress
            match compression {
                Compression::None => {},
                Compression::Zlib => {
                    use flate2::bufread::ZlibDecoder;
                    frame.checkpoint_uncompressed_raw_bytes.resize(uc_enc_size, 0);
                    let mut decoder = ZlibDecoder::new(rply);
                    std::io::copy(&mut decoder, &mut std::io::Cursor::new(frame.checkpoint_uncompressed_raw_bytes.as_mut_slice()))?;
                },
                Compression::Zstd => {
                    use zstd::Decoder;
                    frame.checkpoint_uncompressed_raw_bytes.resize(uc_enc_size, 0);
                    let mut decoder = Decoder::with_buffer(rply)?.single_frame();
                    std::io::copy(&mut decoder, &mut std::io::Cursor::new(frame.checkpoint_uncompressed_raw_bytes.as_mut_slice()))?;
                    decoder.finish();
                },
            };
            // maybe decode
            match encoding {
                Encoding::Raw => {},
                Encoding::Statestream => {
                    frame.checkpoint_uncompressed_unencoded_bytes.resize(uc_ue_size, 0);
                    // statestream_decode(frame.checkpoint_decompressed_data().unwrap(), &mut frame.checkpoint_uncompressed_unencoded_bytes);
                },
            }
        }
    }
    Ok(())
}
