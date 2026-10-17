//! Q3's packet-local adaptive Huffman stream, separate from its fixed MSG codes.
//! Sibling ranks follow original qcommon/huffman.c; links are bounded indices.
use super::{Encoding, Error, ErrorKind, Reader, Writer};

const NYT: u16 = 256;
const INTERNAL: u16 = 257;
const NONE: u16 = u16::MAX;
const NODES: usize = 513;

#[derive(Clone, Copy)]
struct Node {
    symbol: u16,
    weight: u32,
    parent: u16,
    left: u16,
    right: u16,
    next: u16,
    previous: u16,
    block: u16,
}
impl Node {
    const EMPTY: Self = Self {
        symbol: NYT,
        weight: 0,
        parent: NONE,
        left: NONE,
        right: NONE,
        next: NONE,
        previous: NONE,
        block: NONE,
    };
}

#[derive(Clone, Copy)]
struct Block {
    leader: u16,
    next_free: u16,
}
impl Block {
    const EMPTY: Self = Self {
        leader: NONE,
        next_free: NONE,
    };
}

/// Reusable fixed scratch. Each packet starts with the single NYT node; no
/// connection retains training and neither operation allocates.
pub struct AdaptiveHuffman {
    nodes: [Node; NODES],
    symbols: [u16; 256],
    blocks: [Block; NODES],
    path: [u16; NODES],
    root: u16,
    node_count: u16,
    block_count: u16,
    free_block: u16,
}
impl Default for AdaptiveHuffman {
    fn default() -> Self {
        Self::new()
    }
}
impl AdaptiveHuffman {
    pub const fn new() -> Self {
        Self {
            nodes: [Node::EMPTY; NODES],
            symbols: [NONE; 256],
            blocks: [Block::EMPTY; NODES],
            path: [NONE; NODES],
            root: 0,
            node_count: 1,
            block_count: 0,
            free_block: NONE,
        }
    }

    fn reset(&mut self) {
        self.nodes[0] = Node::EMPTY;
        self.symbols.fill(NONE);
        self.root = 0;
        self.node_count = 1;
        self.block_count = 0;
        self.free_block = NONE;
    }

    /// Include the native big-endian decoded length and final padding byte.
    /// Empty input produces no bytes, as original Huff_Compress does.
    pub fn compress(&mut self, input: &[u8], output: &mut [u8]) -> Result<usize, Error> {
        self.reset();
        let length = u16::try_from(input.len()).map_err(|_| Error {
            byte: 0,
            kind: ErrorKind::Capacity,
        })?;
        if input.is_empty() {
            return Ok(0);
        }
        let mut writer = Writer::new(output, Encoding::Bits);
        writer.write_bits(u32::from(length >> 8), 8)?;
        writer.write_bits(u32::from(length & 255), 8)?;
        for &symbol in input {
            let node = self.symbols[usize::from(symbol)];
            self.write_prefix(if node == NONE { 0 } else { node }, &mut writer)?;
            if node == NONE {
                for shift in (0..8).rev() {
                    writer.write_bits(u32::from(symbol >> shift & 1), 1)?;
                }
            }
            self.add_reference(symbol);
        }
        // Native (bit / 8) + 1 includes an extra byte when exactly aligned.
        let padding = (8 - writer.bit_position() % 8) as u8;
        writer.write_bits(0, padding)?;
        Ok(writer.size())
    }

    /// Decode the declared byte count into caller storage. Oversized output is
    /// rejected before decoding; trailing padding is not part of the message.
    pub fn decompress(&mut self, input: &[u8], output: &mut [u8]) -> Result<usize, Error> {
        self.reset();
        if input.is_empty() {
            return Ok(0);
        }
        let mut reader = Reader::new(input, Encoding::Bits);
        let length = ((reader.read_bits(8)? << 8) | reader.read_bits(8)?) as usize;
        let output = output.get_mut(..length).ok_or(Error {
            byte: 0,
            kind: ErrorKind::Capacity,
        })?;
        for value in output {
            let mut node = self.root;
            while self.nodes[usize::from(node)].symbol == INTERNAL {
                let current = self.nodes[usize::from(node)];
                node = if reader.read_bits(1)? == 0 {
                    current.left
                } else {
                    current.right
                };
            }
            let symbol = if self.nodes[usize::from(node)].symbol == NYT {
                let mut symbol = 0;
                for _ in 0..8 {
                    symbol = (symbol << 1) | reader.read_bits(1)? as u8;
                }
                symbol
            } else {
                self.nodes[usize::from(node)].symbol as u8
            };
            *value = symbol;
            self.add_reference(symbol);
        }
        Ok(length)
    }

    fn write_prefix(&mut self, mut node: u16, writer: &mut Writer<'_>) -> Result<(), Error> {
        let mut length = 0;
        while self.nodes[usize::from(node)].parent != NONE {
            self.path[length] = node;
            length += 1;
            node = self.nodes[usize::from(node)].parent;
        }
        for index in (0..length).rev() {
            let node = self.path[index];
            let parent = self.nodes[usize::from(node)].parent;
            writer.write_bits(u32::from(self.nodes[usize::from(parent)].right == node), 1)?;
        }
        Ok(())
    }

    fn allocate_block(&mut self, leader: u16) -> u16 {
        let block = if self.free_block != NONE {
            let block = self.free_block;
            self.free_block = self.blocks[usize::from(block)].next_free;
            block
        } else {
            let block = self.block_count;
            self.block_count += 1;
            block
        };
        self.blocks[usize::from(block)] = Block {
            leader,
            next_free: NONE,
        };
        block
    }

    fn release_block(&mut self, block: u16) {
        self.blocks[usize::from(block)].next_free = self.free_block;
        self.free_block = block;
    }

    fn add_reference(&mut self, symbol: u8) {
        let existing = self.symbols[usize::from(symbol)];
        if existing != NONE {
            self.increment(existing);
            return;
        }
        // A byte can enter once: the NYT node plus two nodes per byte fit 513.
        let leaf = self.node_count;
        let branch = leaf + 1;
        self.node_count += 2;
        let next = self.nodes[0].next;
        let parent = self.nodes[0].parent;
        let block = if next != NONE && self.nodes[usize::from(next)].weight == 1 {
            self.nodes[usize::from(next)].block
        } else {
            self.allocate_block(branch)
        };
        self.nodes[usize::from(branch)] = Node {
            symbol: INTERNAL,
            weight: 1,
            parent,
            left: 0,
            right: leaf,
            next,
            previous: leaf,
            block,
        };
        self.nodes[usize::from(leaf)] = Node {
            symbol: u16::from(symbol),
            weight: 1,
            parent: branch,
            next: branch,
            previous: 0,
            block,
            ..Node::EMPTY
        };
        if next != NONE {
            self.nodes[usize::from(next)].previous = branch;
        }
        self.nodes[0].next = leaf;
        self.nodes[0].parent = branch;
        if parent == NONE {
            self.root = branch;
        } else if self.nodes[usize::from(parent)].left == 0 {
            self.nodes[usize::from(parent)].left = branch;
        } else {
            self.nodes[usize::from(parent)].right = branch;
        }
        self.symbols[usize::from(symbol)] = leaf;
        self.increment(parent);
    }

    fn increment(&mut self, mut node: u16) {
        let mut length = 0;
        while node != NONE {
            let index = usize::from(node);
            let current = self.nodes[index];
            if current.next != NONE
                && self.nodes[usize::from(current.next)].weight == current.weight
            {
                let leader = self.blocks[usize::from(current.block)].leader;
                if leader != current.parent {
                    self.swap_tree(leader, node);
                }
                self.swap_list(leader, node);
            }
            let current = self.nodes[index];
            if current.previous != NONE
                && self.nodes[usize::from(current.previous)].weight == current.weight
            {
                self.blocks[usize::from(current.block)].leader = current.previous;
            } else {
                self.release_block(current.block);
            }
            self.nodes[index].weight += 1;
            let next = self.nodes[index].next;
            self.nodes[index].block = if next != NONE
                && self.nodes[usize::from(next)].weight == self.nodes[index].weight
            {
                self.nodes[usize::from(next)].block
            } else {
                self.allocate_block(node)
            };
            self.path[length] = node;
            length += 1;
            node = self.nodes[index].parent;
        }
        // Preserve native post-parent rank repair without a recursive stack.
        for index in (0..length).rev() {
            let node = self.path[index];
            let parent = self.nodes[usize::from(node)].parent;
            if parent != NONE && self.nodes[usize::from(node)].previous == parent {
                self.swap_list(node, parent);
                let block = self.nodes[usize::from(node)].block;
                if self.blocks[usize::from(block)].leader == node {
                    self.blocks[usize::from(block)].leader = parent;
                }
            }
        }
    }

    fn swap_tree(&mut self, first: u16, second: u16) {
        let first_parent = self.nodes[usize::from(first)].parent;
        let second_parent = self.nodes[usize::from(second)].parent;
        self.replace_child(first_parent, first, second);
        self.replace_child(second_parent, second, first);
        self.nodes[usize::from(first)].parent = second_parent;
        self.nodes[usize::from(second)].parent = first_parent;
    }

    fn replace_child(&mut self, parent: u16, previous: u16, next: u16) {
        if parent == NONE {
            self.root = next;
        } else if self.nodes[usize::from(parent)].left == previous {
            self.nodes[usize::from(parent)].left = next;
        } else {
            self.nodes[usize::from(parent)].right = next;
        }
    }

    fn swap_list(&mut self, first: u16, second: u16) {
        let first_node = self.nodes[usize::from(first)];
        let second_node = self.nodes[usize::from(second)];
        self.nodes[usize::from(first)].next = second_node.next;
        self.nodes[usize::from(second)].next = first_node.next;
        self.nodes[usize::from(first)].previous = second_node.previous;
        self.nodes[usize::from(second)].previous = first_node.previous;
        if self.nodes[usize::from(first)].next == first {
            self.nodes[usize::from(first)].next = second;
        }
        if self.nodes[usize::from(second)].next == second {
            self.nodes[usize::from(second)].next = first;
        }
        for node in [first, second] {
            let current = self.nodes[usize::from(node)];
            if current.next != NONE {
                self.nodes[usize::from(current.next)].previous = node;
            }
        }
        for node in [first, second] {
            let current = self.nodes[usize::from(node)];
            if current.previous != NONE {
                self.nodes[usize::from(current.previous)].next = node;
            }
        }
    }
}
