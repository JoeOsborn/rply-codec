mod blockindex;
use blockindex::BlockIndex;

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
    finished: bool,
}

impl<'r, 'c, R: std::io::Read> Decoder<'r, 'c, R> {
    pub(crate) fn new(reader: &'r mut R, ctx: &'c mut Ctx) -> Self {
        Self {
            reader,
            ctx,
            finished: false,
        }
    }
}

impl<'r, 'c, R: std::io::Read> std::io::Read for Decoder<'r, 'c, R> {
    /* a slightly degenerate read implementation in that it will keep
     * calling read on the inner reader until a complete checkpoint is
     * read, then return 0 for subsequent reads */
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        use rmp::decode as r;
        if self.finished {
            return Ok(0);
        }

        let sz = 0;
        todo!();
        Ok(sz)
    }
}
