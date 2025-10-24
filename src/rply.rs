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
    Magic = 0,
    Version = 4,
    CRC = 8,
    StateSize = 12,
    Identifier = 16,
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

#[repr(u8)]
#[non_exhaustive]
#[derive(Debug)]
pub enum SSToken {
    Start = 0,
    NewBlock = 1,
    NewSuperblock = 2,
    SuperblockSeq = 3,
}

#[repr(u8)]
#[non_exhaustive]
#[derive(Debug)]
pub enum Compression {
    None = 0,
    Zlib = 1,
    Zstd = 2,
}

impl TryFrom<u8> for Compression {}

#[repr(u8)]
#[non_exhaustive]
#[derive(Debug)]
pub enum Encoding {
    Raw = 0,
    Statestream = 1,
}

impl TryFrom<u8> for Encoding {}

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

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ReplayError {
    #[error("Invalid replay magic {0}")]
    Magic(u32),
    #[error("Unsupported version {0}")]
    Version(u32),
    #[error("Unsupported compression scheme {0}")]
    Compression(u8),
    #[error("I/O Error")]
    IO(#[from] std::io::Error),
}

type Result<T> = std::result::Result<T, ReplayError>;

pub fn read_header(rply: &mut impl std::io::Read) -> Result<Header> {
    let mut bytes = vec![0; HEADER_LEN_BYTES];
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
    rply.read_exact(&mut bytes[HEADER_V0V1_LEN_BYTES..HEADER_LEN_BYTES]);
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
