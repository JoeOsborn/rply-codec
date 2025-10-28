mod blockindex;
use crate::InvalidDeterminant;
use blockindex::BlockIndex;
use std::io::Write;

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

impl From<SSToken> for u8 {
    fn from(value: SSToken) -> Self {
        match value {
            SSToken::Start => 0,
            SSToken::NewBlock => 1,
            SSToken::NewSuperblock => 2,
            SSToken::SuperblockSeq => 3,
        }
    }
}

pub(crate) struct Ctx {
    block_size: u32,
    superblock_size: u32,
    last_state: Vec<u8>,
    last_superseq: Vec<u32>,
    block_index: BlockIndex<u8>,
    superblock_index: BlockIndex<u32>,
}

impl Ctx {
    pub fn new(block_size: u32, superblock_size: u32) -> Self {
        Self {
            block_size,
            superblock_size,
            last_state: vec![],
            last_superseq: vec![],
            block_index: BlockIndex::new(block_size as usize),
            superblock_index: BlockIndex::new(superblock_size as usize),
        }
    }
}

pub(crate) struct Decoder<'r, 'c, R: std::io::Read> {
    reader: &'r mut R,
    ctx: &'c mut Ctx,
    state_size: usize,
    finished: bool,
    readout_cursor: usize,
}

impl<'r, 'c, R: std::io::Read> Decoder<'r, 'c, R> {
    pub(crate) fn new(reader: &'r mut R, ctx: &'c mut Ctx, state_size: usize) -> Self {
        Self {
            reader,
            ctx,
            finished: false,
            readout_cursor: 0,
            state_size,
        }
    }
    fn readout(&mut self, mut buf: &mut [u8]) -> std::io::Result<usize> {
        match buf.write(&self.ctx.last_state[self.readout_cursor..]) {
            Err(e) => Err(e),
            Ok(sz) => {
                self.readout_cursor += sz;
                Ok(sz)
            }
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseState {
    WaitForStart,
    WaitForSuperblockSeq,
    Finished,
}

#[derive(thiserror::Error, Debug)]
enum SSError {
    #[error("Invalid token {0}")]
    InvalidToken(#[from] InvalidDeterminant),
    #[error("Too many start tokens in stream")]
    TooManyStarts(),
    #[error("Unexpected {1:?} during {0:?}")]
    ParseError(ParseState, SSToken),
    #[error("Block {0} is the wrong size")]
    BlockWrongSize(u32),
    #[error("Superblock {0} is the wrong size")]
    SuperblockWrongSize(u32),
    #[error("Couldn't insert block at {1} on frame {0}")]
    BadBlockInsert(u64, u32),
    #[error("Couldn't insert superblock at {1} on frame {0}")]
    BadSuperblockInsert(u64, u32),
}

impl<R: std::io::Read> std::io::Read for Decoder<'_, '_, R> {
    /* a slightly degenerate read implementation in that it will keep
     * calling read on the inner reader until a complete checkpoint is
     * read, then return 0 for subsequent reads */
    fn read(&mut self, outbuf: &mut [u8]) -> std::io::Result<usize> {
        use ParseState as State;
        use rmp::decode as r;
        if self.finished {
            if self.readout_cursor == self.state_size {
                return Ok(0);
            }
            return self.readout(outbuf);
        }
        let mut frame = 0;
        let mut state = State::WaitForStart;
        let mut buf = vec![0_u8; self.ctx.block_size as usize];
        let mut superblock = vec![0_u32; self.ctx.superblock_size as usize];
        loop {
            let tok: u8 = r::read_int(self.reader).map_err(std::io::Error::other)?;
            match (
                state,
                SSToken::try_from(tok)
                    .map_err(|e| std::io::Error::other(SSError::InvalidToken(e)))?,
            ) {
                (State::WaitForStart, SSToken::Start) => {
                    frame = r::read_int(self.reader).map_err(std::io::Error::other)?;
                    state = State::WaitForSuperblockSeq;
                }
                (_, SSToken::Start) => return Err(std::io::Error::other(SSError::TooManyStarts())),
                (State::WaitForSuperblockSeq, SSToken::NewBlock) => {
                    let idx = r::read_int(self.reader).map_err(std::io::Error::other)?;
                    let bin_len = r::read_bin_len(self.reader).map_err(std::io::Error::other)?;
                    if bin_len != self.ctx.block_size {
                        return Err(std::io::Error::other(SSError::BlockWrongSize(bin_len)));
                    }
                    self.reader.read_exact(&mut buf)?;
                    if !self
                        .ctx
                        .block_index
                        .insert_exact(idx, Box::from(buf.clone()), frame)
                    {
                        return Err(std::io::Error::other(SSError::BadBlockInsert(frame, idx)));
                    }
                }
                (State::WaitForSuperblockSeq, SSToken::NewSuperblock) => {
                    let idx = r::read_int(self.reader).map_err(std::io::Error::other)?;
                    let arr_len = r::read_array_len(self.reader).map_err(std::io::Error::other)?;
                    if arr_len != self.ctx.superblock_size {
                        return Err(std::io::Error::other(SSError::SuperblockWrongSize(arr_len)));
                    }
                    for superblock_elt in &mut superblock {
                        *superblock_elt =
                            r::read_int(self.reader).map_err(std::io::Error::other)?;
                    }
                    if !self.ctx.superblock_index.insert_exact(
                        idx,
                        Box::from(superblock.clone()),
                        frame,
                    ) {
                        return Err(std::io::Error::other(SSError::BadSuperblockInsert(
                            frame, idx,
                        )));
                    }
                }
                (State::WaitForSuperblockSeq, SSToken::SuperblockSeq) => {
                    let arr_len =
                        r::read_array_len(self.reader).map_err(std::io::Error::other)? as usize;
                    let block_byte_size = self.ctx.block_size as usize;
                    let superblock_byte_size = self.ctx.superblock_size as usize * block_byte_size;
                    let mut superseq = vec![0; arr_len];
                    self.ctx.last_state.resize(self.state_size, 0);
                    for (superblock_i, superseq_sblk) in superseq.iter_mut().enumerate() {
                        let superblock_idx =
                            r::read_int(self.reader).map_err(std::io::Error::other)?;
                        *superseq_sblk = superblock_idx;
                        let superblock_data = self.ctx.superblock_index.get(superblock_idx);
                        for (block_i, block_id) in superblock_data.iter().copied().enumerate() {
                            let block_start = (superblock_i * superblock_byte_size
                                + block_i * block_byte_size)
                                .min(self.state_size);
                            let block_end = (block_start + block_byte_size).min(self.state_size);
                            let block_bytes = self.ctx.block_index.get(block_id);
                            if block_end <= block_start {
                                // This can happen in the last superblock if it was padded with extra blocks
                                break;
                            }
                            self.ctx.last_state[block_start..block_end]
                                .copy_from_slice(&block_bytes[0..(block_end - block_start)]);
                        }
                    }
                    self.ctx.last_superseq = superseq;
                    state = State::Finished;
                    self.finished = true;
                    break;
                }
                (s, tok) => return Err(std::io::Error::other(SSError::ParseError(s, tok))),
            }
        }
        assert_eq!(state, State::Finished);
        self.readout(outbuf)
    }
}

pub(crate) struct Encoder<'w, 'c, W: std::io::Write> {
    writer: &'w mut W,
    ctx: &'c mut Ctx,
}

impl<'w, 'c, W: std::io::Write> Encoder<'w, 'c, W> {
    pub(crate) fn new(writer: &'w mut W, ctx: &'c mut Ctx) -> Self {
        Self { writer, ctx }
    }
    pub fn encode_checkpoint(mut self, checkpoint: &[u8], frame: u64) -> std::io::Result<u32> {
        use rmp::encode as r;
        r::write_uint(&mut self.writer, u64::from(u8::from(SSToken::Start)))?;
        r::write_uint(&mut self.writer, frame)?;
        todo!();

        Ok(0)
    }
}
