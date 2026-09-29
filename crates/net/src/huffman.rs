//! Adaptive Huffman codec for Quake III messages.
//!
//! Donor provenance: `HuffmanTree`, `createMessageHuffman`,
//! `compressAdaptive`, and `decompressAdaptive` in
//! `src/network/q3/huffman.ts` (a port of id Software's `huffman.c`).
//!
//! Nodes live in an arena instead of a garbage-collected object graph,
//! but the sibling-property updates, block leaders, and training order
//! match the donor exactly, so encoded bit streams are identical.

use std::sync::OnceLock;

use thiserror::Error;

/// Not-yet-transmitted symbol id.
const NYT: u16 = 256;
/// Internal branch id.
const INTERNAL: u16 = 257;

/// `msg_hData` training counts in symbol order.
const MESSAGE_COUNTS: [u32; 256] = [
    250315, 41193, 6292, 7106, 3730, 3750, 6110, 23283, 33317, 6950, 7838, 9714, 9257, 17259, 3949, 1778, 8288, 1604,
    1590, 1663, 1100, 1213, 1238, 1134, 1749, 1059, 1246, 1149, 1273, 4486, 2805, 3472, 21819, 1159, 1670, 1066, 1043,
    1012, 1053, 1070, 1726, 888, 1180, 850, 960, 780, 1752, 3296, 10630, 4514, 5881, 2685, 4650, 3837, 2093, 1867,
    2584, 1949, 1972, 940, 1134, 1788, 1670, 1206, 5719, 6128, 7222, 6654, 3710, 3795, 1492, 1524, 2215, 1140, 1355,
    971, 2180, 1248, 1328, 1195, 1770, 1078, 1264, 1266, 1168, 965, 1155, 1186, 1347, 1228, 1529, 1600, 2617, 2048,
    2546, 3275, 2410, 3585, 2504, 2800, 2675, 6146, 3663, 2840, 14253, 3164, 2221, 1687, 3208, 2739, 3512, 4796, 4091,
    3515, 5288, 4016, 7937, 6031, 5360, 3924, 4892, 3743, 4566, 4807, 5852, 6400, 6225, 8291, 23243, 7838, 7073, 8935,
    5437, 4483, 3641, 5256, 5312, 5328, 5370, 3492, 2458, 1694, 1821, 2121, 1916, 1149, 1516, 1367, 1236, 1029, 1258,
    1104, 1245, 1006, 1149, 1025, 1241, 952, 1287, 997, 1713, 1009, 1187, 879, 1099, 929, 1078, 951, 1656, 930, 1153,
    1030, 1262, 1062, 1214, 1060, 1621, 930, 1106, 912, 1034, 892, 1158, 990, 1175, 850, 1121, 903, 1087, 920, 1144,
    1056, 3462, 2240, 4397, 12136, 7758, 1345, 1307, 3278, 1950, 886, 1023, 1112, 1077, 1042, 1061, 1071, 1484, 1001,
    1096, 915, 1052, 995, 1070, 876, 1111, 851, 1059, 805, 1112, 923, 1103, 817, 1899, 1872, 976, 841, 1127, 956, 1159,
    950, 7791, 954, 1289, 933, 1127, 3207, 1020, 927, 1355, 768, 1040, 745, 952, 805, 1073, 740, 1013, 805, 1008, 796,
    996, 1057, 11457, 13504,
];

/// Error for Huffman coding failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HuffmanError {
    /// A Huffman symbol must be a byte.
    #[error("huffman symbol must be a byte")]
    BadSymbol,
    /// The tree path led nowhere.
    #[error("invalid huffman tree")]
    InvalidTree,
    /// The input ended inside a symbol.
    #[error("truncated huffman symbol")]
    Truncated,
    /// Adaptive payloads are limited to 16-bit lengths.
    #[error("adaptive huffman length exceeds 16 bits")]
    LengthTooLarge,
    /// Negative decompression limit.
    #[error("invalid decompression limit")]
    BadLimit,
}

#[derive(Debug, Clone)]
struct Node {
    symbol: u16,
    weight: u32,
    left: Option<usize>,
    right: Option<usize>,
    parent: Option<usize>,
    next: Option<usize>,
    prev: Option<usize>,
    block_leader: usize,
}

/// Adaptive sibling-property Huffman tree (`HuffmanTree`).
#[derive(Debug, Clone)]
pub struct HuffmanTree {
    nodes: Vec<Node>,
    nyt: usize,
    root: usize,
    symbols: [Option<usize>; 257],
}

impl HuffmanTree {
    /// Create an empty tree holding only the NYT leaf.
    #[must_use]
    pub fn new() -> Self {
        let nyt = Node {
            symbol: NYT,
            weight: 0,
            left: None,
            right: None,
            parent: None,
            next: None,
            prev: None,
            block_leader: 0,
        };
        let mut symbols: [Option<usize>; 257] = [None; 257];
        symbols[NYT as usize] = Some(0);
        Self {
            nodes: vec![nyt],
            nyt: 0,
            root: 0,
            symbols,
        }
    }

    fn push(&mut self, symbol: u16, weight: u32) -> usize {
        let index = self.nodes.len();
        self.nodes.push(Node {
            symbol,
            weight,
            left: None,
            right: None,
            parent: None,
            next: None,
            prev: None,
            block_leader: index,
        });
        index
    }

    /// Train the tree on one symbol occurrence (`addReference`).
    pub fn add_reference(&mut self, symbol: u8) {
        if let Some(existing) = self.symbols[usize::from(symbol)] {
            self.increment(Some(existing));
            return;
        }
        let leaf = self.push(u16::from(symbol), 1);
        let branch = self.push(INTERNAL, 1);
        self.nodes[branch].next = self.nodes[self.nyt].next;
        if let Some(next) = self.nodes[branch].next {
            self.nodes[next].prev = Some(branch);
            if self.nodes[next].weight == 1 {
                self.nodes[branch].block_leader = self.nodes[next].block_leader;
            }
        }
        self.nodes[self.nyt].next = Some(branch);
        self.nodes[branch].prev = Some(self.nyt);
        self.nodes[leaf].next = Some(branch);
        self.nodes[branch].prev = Some(leaf);
        self.nodes[leaf].block_leader = self.nodes[branch].block_leader;
        self.nodes[self.nyt].next = Some(leaf);
        self.nodes[leaf].prev = Some(self.nyt);

        let parent = self.nodes[self.nyt].parent;
        if let Some(parent) = parent {
            if self.nodes[parent].left == Some(self.nyt) {
                self.nodes[parent].left = Some(branch);
            } else {
                self.nodes[parent].right = Some(branch);
            }
        } else {
            self.root = branch;
        }
        self.nodes[branch].right = Some(leaf);
        self.nodes[branch].left = Some(self.nyt);
        self.nodes[branch].parent = parent;
        self.nodes[self.nyt].parent = Some(branch);
        self.nodes[leaf].parent = Some(branch);
        self.symbols[usize::from(symbol)] = Some(leaf);
        self.increment(parent);
    }

    /// Encode one symbol (`encodeSymbol`).
    pub fn encode_symbol(&self, symbol: u8, put_bit: &mut impl FnMut(u8)) {
        if let Some(node) = self.symbols[usize::from(symbol)] {
            self.send(node, put_bit);
        } else {
            self.send(self.nyt, put_bit);
            for shift in (0..8).rev() {
                put_bit((symbol >> shift) & 1);
            }
        }
    }

    /// Decode one tree prefix (`decodePrefix`).
    pub fn decode_prefix(&self, get_bit: &mut impl FnMut() -> u8) -> Result<u16, HuffmanError> {
        let mut node = self.root;
        while self.nodes[node].symbol == INTERNAL {
            let next = if get_bit() == 0 {
                self.nodes[node].left
            } else {
                self.nodes[node].right
            };
            node = next.ok_or(HuffmanError::InvalidTree)?;
        }
        Ok(self.nodes[node].symbol)
    }

    /// Decode one tree prefix with a fallible bit source.
    ///
    /// Returns `Ok(None)` when the path leaves the tree; bit-source
    /// failures propagate unchanged.
    pub fn decode_prefix_checked<E>(&self, get_bit: &mut impl FnMut() -> Result<u8, E>) -> Result<Option<u16>, E> {
        let mut node = self.root;
        while self.nodes[node].symbol == INTERNAL {
            let next = if get_bit()? == 0 {
                self.nodes[node].left
            } else {
                self.nodes[node].right
            };
            let Some(next) = next else {
                return Ok(None);
            };
            node = next;
        }
        Ok(Some(self.nodes[node].symbol))
    }

    /// Decode one symbol with a fallible bit source (`decodeSymbol`).
    pub fn decode_symbol_checked<E>(&self, get_bit: &mut impl FnMut() -> Result<u8, E>) -> Result<Option<u8>, E> {
        let Some(prefix) = self.decode_prefix_checked(get_bit)? else {
            return Ok(None);
        };
        if prefix != NYT {
            return Ok(u8::try_from(prefix).ok());
        }
        let mut symbol: u8 = 0;
        for _ in 0..8 {
            symbol = (symbol << 1) | (get_bit()? & 1);
        }
        Ok(Some(symbol))
    }

    /// Decode one symbol, reading a raw byte after NYT (`decodeSymbol`).
    pub fn decode_symbol(&self, get_bit: &mut impl FnMut() -> u8) -> Result<u8, HuffmanError> {
        let prefix = self.decode_prefix(get_bit)?;
        if prefix != NYT {
            return u8::try_from(prefix).map_err(|_| HuffmanError::InvalidTree);
        }
        let mut symbol: u8 = 0;
        for _ in 0..8 {
            symbol = (symbol << 1) | (get_bit() & 1);
        }
        Ok(symbol)
    }

    fn send(&self, node: usize, put_bit: &mut impl FnMut(u8)) {
        if let Some(parent) = self.nodes[node].parent {
            self.send(parent, put_bit);
            put_bit(u8::from(self.nodes[parent].right == Some(node)));
        }
    }

    fn swap_tree(&mut self, a: usize, b: usize) {
        let ap = self.nodes[a].parent;
        let bp = self.nodes[b].parent;
        match ap {
            None => self.root = b,
            Some(p) if self.nodes[p].left == Some(a) => self.nodes[p].left = Some(b),
            Some(p) => self.nodes[p].right = Some(b),
        }
        match bp {
            None => self.root = a,
            Some(p) if self.nodes[p].left == Some(b) => self.nodes[p].left = Some(a),
            Some(p) => self.nodes[p].right = Some(a),
        }
        self.nodes[a].parent = bp;
        self.nodes[b].parent = ap;
    }

    fn swap_list(&mut self, a: usize, b: usize) {
        let next = self.nodes[a].next;
        self.nodes[a].next = self.nodes[b].next;
        self.nodes[b].next = next;
        let prev = self.nodes[a].prev;
        self.nodes[a].prev = self.nodes[b].prev;
        self.nodes[b].prev = prev;
        if self.nodes[a].next == Some(a) {
            self.nodes[a].next = Some(b);
        }
        if self.nodes[b].next == Some(b) {
            self.nodes[b].next = Some(a);
        }
        if let Some(next) = self.nodes[a].next {
            self.nodes[next].prev = Some(a);
        }
        if let Some(next) = self.nodes[b].next {
            self.nodes[next].prev = Some(b);
        }
        if let Some(prev) = self.nodes[a].prev {
            self.nodes[prev].next = Some(a);
        }
        if let Some(prev) = self.nodes[b].prev {
            self.nodes[prev].next = Some(b);
        }
    }

    fn increment(&mut self, node: Option<usize>) {
        let Some(node) = node else {
            return;
        };
        if let Some(next) = self.nodes[node].next {
            if self.nodes[next].weight == self.nodes[node].weight {
                let leader = self.nodes[node].block_leader;
                if Some(leader) != self.nodes[node].parent {
                    self.swap_tree(leader, node);
                }
                self.swap_list(leader, node);
            }
        }
        if let Some(prev) = self.nodes[node].prev {
            if self.nodes[prev].weight == self.nodes[node].weight {
                // The donor re-points the shared block leader at the
                // predecessor; every block member shares one leader slot.
                set_leader(self, node, prev);
            }
        }
        let weight = self.nodes[node].weight + 1;
        self.nodes[node].weight = weight;
        if let Some(next) = self.nodes[node].next {
            if self.nodes[next].weight == weight {
                self.nodes[node].block_leader = self.nodes[next].block_leader;
            } else {
                self.nodes[node].block_leader = node;
            }
        } else {
            self.nodes[node].block_leader = node;
        }
        if let Some(parent) = self.nodes[node].parent {
            self.increment(Some(parent));
            if self.nodes[node].prev == Some(parent) {
                self.swap_list(node, parent);
                if self.nodes[node].block_leader == node {
                    set_leader(self, node, parent);
                }
            }
        }
    }
}

/// Re-point every member of `node`'s block at `leader`.
///
/// The donor stores one shared `Block` object per weight class; the
/// arena stores the leader index on each member, so leader updates
/// rewrite every member carrying the old leader.
fn set_leader(tree: &mut HuffmanTree, node: usize, leader: usize) {
    let old = tree.nodes[node].block_leader;
    for member in tree.nodes.iter_mut() {
        if member.block_leader == old {
            member.block_leader = leader;
        }
    }
}

impl Default for HuffmanTree {
    fn default() -> Self {
        Self::new()
    }
}

fn trained_message_tree() -> HuffmanTree {
    let mut tree = HuffmanTree::new();
    for (symbol, count) in MESSAGE_COUNTS.iter().enumerate() {
        for _ in 0..*count {
            tree.add_reference(symbol as u8);
        }
    }
    tree
}

static MESSAGE_TREE: OnceLock<HuffmanTree> = OnceLock::new();

/// Shared frozen message codec (`createMessageHuffman`).
pub fn message_tree() -> &'static HuffmanTree {
    MESSAGE_TREE.get_or_init(trained_message_tree)
}

/// Compress a payload with the adaptive codec (`compressAdaptive`).
pub fn compress_adaptive(data: &[u8]) -> Result<Vec<u8>, HuffmanError> {
    if data.is_empty() {
        return Ok(Vec::new());
    }
    if data.len() > 65535 {
        return Err(HuffmanError::LengthTooLarge);
    }
    let mut output = vec![0u8; 2 + data.len() * 33 + 1];
    output[0] = (data.len() >> 8) as u8;
    output[1] = (data.len() & 255) as u8;
    let mut position = 16usize;
    let mut tree = HuffmanTree::new();
    for symbol in data {
        tree.encode_symbol(*symbol, &mut |bit| {
            let offset = position >> 3;
            output[offset] |= bit << (position & 7);
            position += 1;
        });
        tree.add_reference(*symbol);
    }
    output.truncate((position >> 3) + 1);
    Ok(output)
}

/// Decompress an adaptive payload (`decompressAdaptive`).
pub fn decompress_adaptive(data: &[u8], max_length: usize) -> Result<Vec<u8>, HuffmanError> {
    if data.is_empty() {
        return Ok(Vec::new());
    }
    if data.len() < 2 {
        return Err(HuffmanError::Truncated);
    }
    let declared = (usize::from(data[0]) << 8) | usize::from(data[1]);
    let length = declared.min(max_length);
    let mut position = 16usize;
    let mut tree = HuffmanTree::new();
    let mut output = Vec::with_capacity(length);
    for _ in 0..length {
        let mut get_bit = || {
            if position >= data.len() * 8 {
                return Err(HuffmanError::Truncated);
            }
            let value = (data[position >> 3] >> (position & 7)) & 1;
            position += 1;
            Ok(value)
        };
        let symbol = tree
            .decode_symbol_checked(&mut get_bit)?
            .ok_or(HuffmanError::InvalidTree)?;
        output.push(symbol);
        tree.add_reference(symbol);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptive_codec_round_trip() {
        let cases: &[&[u8]] = &[
            b"",
            b"a",
            b"Hello, Quake III Arena! Hello, Quake III Arena!",
            &[0u8, 1, 2, 3, 250, 251, 252, 253, 254, 255],
        ];
        for case in cases {
            let compressed = compress_adaptive(case).unwrap();
            let plain = decompress_adaptive(&compressed, 16384).unwrap();
            assert_eq!(plain, *case);
        }
        assert!(decompress_adaptive(&[0], 16384).is_err());
        assert_eq!(compress_adaptive(&[]).unwrap(), Vec::<u8>::new());
        assert_eq!(decompress_adaptive(&[], 8).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn fresh_tree_sends_nyt_then_raw_byte() {
        let tree = HuffmanTree::new();
        let mut bits = Vec::new();
        tree.encode_symbol(0x41, &mut |bit| bits.push(bit));
        assert_eq!(bits, vec![0, 1, 0, 0, 0, 0, 0, 1]);
        let mut reader = HuffmanTree::new();
        let mut position = 0;
        let symbol = reader.decode_symbol(&mut || {
            let bit = bits[position];
            position += 1;
            bit
        });
        assert_eq!(symbol.unwrap(), 0x41);
        reader.add_reference(0x41);
    }

    #[test]
    fn message_codec_round_trip() {
        let tree = message_tree();
        let payload: Vec<u8> = (0..=255u8).collect();
        let mut bits = Vec::new();
        for symbol in &payload {
            tree.encode_symbol(*symbol, &mut |bit| bits.push(bit));
        }
        let mut position = 0;
        let mut decoded = Vec::new();
        for _ in 0..payload.len() {
            let symbol = tree
                .decode_prefix(&mut || {
                    let bit = bits[position];
                    position += 1;
                    bit
                })
                .unwrap();
            decoded.push(symbol as u8);
        }
        assert_eq!(decoded, payload);
        assert_eq!(position, bits.len());
    }
}
