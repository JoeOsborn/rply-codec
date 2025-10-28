use nohash_hasher::NoHashHasher;
use smallvec::{SmallVec, smallvec};
use std::{collections::HashMap, hash::BuildHasherDefault};
use xxhash_rust::xxh3::xxh3_64 as xxh;

// struct Addition {
//     when:u64, // Frame on which some objects were added
//     index:u32, // Lowest index added on this frame
// }

pub(crate) struct BlockIndex<
    T: bytemuck::Zeroable + bytemuck::AnyBitPattern + bytemuck::NoUninit + PartialEq,
> {
    index: HashMap<u64, SmallVec<[u32; 4]>, BuildHasherDefault<NoHashHasher<u64>>>,
    objects: Vec<Box<[T]>>,
    hashes: Vec<u64>,
    //additions: Vec<Addition>,
    object_size: usize,
}

#[expect(unused)]
pub(crate) struct Insertion {
    index: u32,
    is_new: bool,
}

fn hash<T: bytemuck::AnyBitPattern + bytemuck::NoUninit>(val: &[T]) -> u64 {
    xxh(bytemuck::cast_slice(val))
}

impl<T: bytemuck::Zeroable + bytemuck::AnyBitPattern + bytemuck::NoUninit + PartialEq>
    BlockIndex<T>
{
    pub fn new(object_size: usize) -> Self {
        let mut index = HashMap::with_capacity_and_hasher(4096, BuildHasherDefault::default());
        let zeros = (vec![T::zeroed(); object_size]).into_boxed_slice();
        let zero_hash = hash(&zeros);
        index.insert(zero_hash, smallvec![0]);
        Self {
            index,
            object_size,
            objects: vec![zeros],
            hashes: vec![zero_hash],
        }
    }
    #[expect(unused)]
    pub fn insert(&mut self, obj: &[T], _frame: u64) -> Insertion {
        assert_eq!(obj.len(), self.object_size);
        let hash = hash(obj);
        match self.index.entry(hash) {
            std::collections::hash_map::Entry::Occupied(mut e) => {
                if let Some(found) = e
                    .get()
                    .iter()
                    .find(|o| obj == &*self.objects[(**o) as usize])
                {
                    Insertion {
                        index: *found,
                        is_new: false,
                    }
                } else {
                    let copy = Box::from(obj);
                    let idx = u32::try_from(self.objects.len()).unwrap();
                    self.objects.push(copy);
                    self.hashes.push(hash);
                    e.get_mut().push(idx);
                    Insertion {
                        index: idx,
                        is_new: true,
                    }
                }
            }
            std::collections::hash_map::Entry::Vacant(e) => {
                let copy = Box::from(obj);
                let idx = u32::try_from(self.objects.len()).unwrap();
                self.objects.push(copy);
                self.hashes.push(hash);
                e.insert(smallvec![idx]);
                Insertion {
                    index: idx,
                    is_new: true,
                }
            }
        }
    }
    pub fn insert_exact(&mut self, idx: u32, obj: Box<[T]>, _frame: u64) -> bool {
        assert_eq!(obj.len(), self.object_size);
        if self.objects.len() != idx as usize {
            return false;
        }
        let hash = hash(&obj);
        self.index.entry(hash).or_default().push(idx);
        self.objects.push(obj);
        self.hashes.push(hash);
        true
    }
    pub fn get(&self, which: u32) -> &[T] {
        &self.objects[which as usize]
    }
    #[expect(unused)]
    pub fn clear(&mut self) {
        self.index.clear();
        self.objects.truncate(1);
        self.hashes.truncate(1);
        self.index.insert(self.hashes[0], smallvec![0]);
    }
    #[expect(unused)]
    pub fn len(&self) -> usize {
        self.objects.len()
    }
    // remove_after, commit?
}
