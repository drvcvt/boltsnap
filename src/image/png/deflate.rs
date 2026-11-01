//! The deflate a filtered screenshot band needs and no more, after
//! fdeflate: one dynamic block of literals and runs of zeros (a literal zero
//! repeated at distance 1), which uniform areas become once filtered. The
//! band is scanned eight bytes at a time into spans, the Huffman codes come
//! from the band's own histogram, and literals are written four at a time.
//! Zeros only count as a run where they fill or end an eight-byte block;
//! a few zeros amid other bytes stay literals.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// The order the code length code lengths are sent in (RFC 1951, 3.2.7).
const CODE_LENGTH_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];
const END_OF_BLOCK: usize = 256;
/// Zeros in a row from which a run beats literals: one literal zero and a
/// copy of at least three.
const MIN_RUN: usize = 4;
const MAX_LENGTH: usize = 258;

/// Part of a band: bytes sent as literals, or a run of zeros.
enum Span {
    Literals(std::ops::Range<usize>),
    Zeros(usize),
}

/// `data` as one dynamic block. A band that is not `last` ends with an
/// empty stored block, so the next band starts on a byte boundary of the
/// same stream.
pub(super) fn deflate(data: &[u8], last: bool, bits: &mut Bits) {
    let spans = spans(data);
    let mut literals = ByteCounts::default();
    let mut freqs = [0_u32; 286];
    for span in &spans {
        match span {
            Span::Literals(range) => literals.add(&data[range.clone()]),
            Span::Zeros(count) => run_pieces(*count, |piece| match piece {
                Piece::Zero => freqs[0] += 1,
                Piece::Copy(length) => freqs[usize::from(LENGTHS[length].0)] += 1,
            }),
        }
    }
    literals.into(&mut freqs);
    freqs[END_OF_BLOCK] = 1;
    let lengths = code_lengths(&freqs, 15);
    let codes = codes(&lengths);
    // Code and length side by side, one lookup a symbol.
    let mut table = [(0_u64, 0_u32); 286];
    for (entry, (&code, &length)) in table.iter_mut().zip(codes.iter().zip(&lengths)) {
        *entry = (u64::from(code), u32::from(length));
    }
    let code = |symbol: usize| table[symbol];

    bits.put(u64::from(last), 1);
    bits.put(2, 2);
    write_code_lengths(&lengths, bits);
    for span in &spans {
        match span {
            Span::Literals(range) => {
                let mut quads = data[range.clone()].chunks_exact(4);
                for quad in &mut quads {
                    let (mut value, mut count) = (0, 0);
                    for &byte in quad {
                        let (c, n) = code(usize::from(byte));
                        value |= c << count;
                        count += n;
                    }
                    bits.put(value, count);
                }
                for &byte in quads.remainder() {
                    let (c, n) = code(usize::from(byte));
                    bits.put(c, n);
                }
            }
            Span::Zeros(count) => run_pieces(*count, |piece| match piece {
                Piece::Zero => {
                    let (c, n) = code(0);
                    bits.put(c, n);
                }
                Piece::Copy(length) => {
                    let (symbol, extra_bits, extra) = LENGTHS[length];
                    let (c, n) = code(usize::from(symbol));
                    // Length code, its extra bits, then the one distance
                    // code: 0 in one bit.
                    bits.put(c | u64::from(extra) << n, n + u32::from(extra_bits) + 1);
                }
            }),
        }
    }
    let (c, n) = code(END_OF_BLOCK);
    bits.put(c, n);
    if !last {
        // Empty stored block: header, pad to the byte, LEN 0, NLEN !0.
        bits.put(0, 3);
        bits.align();
        bits.out.extend_from_slice(&[0, 0, 0xff, 0xff]);
    }
}

/// `data` as literal stretches and runs of at least `MIN_RUN` zeros, found
/// eight bytes at a time: a run is the zeros ending one block, any blocks of
/// nothing but zeros, and the zeros starting the next.
fn spans(data: &[u8]) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut literal_start = 0;
    // The zeros seen so far that may become a run: where they start, how many.
    let (mut run_start, mut run) = (0, 0);
    let mut close = |spans: &mut Vec<Span>, start: usize, count: usize| {
        if count >= MIN_RUN {
            if literal_start < start {
                spans.push(Span::Literals(literal_start..start));
            }
            spans.push(Span::Zeros(count));
            literal_start = start + count;
        }
    };
    let mut blocks = data.chunks_exact(8);
    for (index, block) in (&mut blocks).enumerate() {
        let word = u64::from_le_bytes(block.try_into().expect("eight bytes"));
        if word == 0 {
            if run == 0 {
                run_start = index * 8;
            }
            run += 8;
            continue;
        }
        if run > 0 {
            // Little-endian: the block's first bytes are the low ones.
            close(
                &mut spans,
                run_start,
                run + word.trailing_zeros() as usize / 8,
            );
        }
        run = word.leading_zeros() as usize / 8;
        run_start = index * 8 + 8 - run;
    }
    if run > 0 {
        let tail = blocks.remainder().iter().take_while(|&&b| b == 0).count();
        close(&mut spans, run_start, run + tail);
    }
    if literal_start < data.len() {
        spans.push(Span::Literals(literal_start..data.len()));
    }
    spans
}

/// Byte counts in four tables taken in turn, so a byte repeated does not
/// wait on its own count.
struct ByteCounts([[u32; 256]; 4]);

impl Default for ByteCounts {
    fn default() -> Self {
        Self([[0; 256]; 4])
    }
}

impl ByteCounts {
    fn add(&mut self, bytes: &[u8]) {
        let mut quads = bytes.chunks_exact(4);
        for quad in &mut quads {
            for (table, &byte) in self.0.iter_mut().zip(quad) {
                table[usize::from(byte)] += 1;
            }
        }
        for &byte in quads.remainder() {
            self.0[0][usize::from(byte)] += 1;
        }
    }

    /// Adds the counts to the first 256 symbols of `freqs`.
    fn into(self, freqs: &mut [u32; 286]) {
        for table in &self.0 {
            for (freq, &count) in freqs.iter_mut().zip(table) {
                *freq += count;
            }
        }
    }
}

enum Piece {
    Zero,
    /// A copy of the byte before, this many times.
    Copy(usize),
}

/// How `count` zeros are sent: a literal zero, copies of at most
/// `MAX_LENGTH`, and literal zeros for a rest too short to copy.
fn run_pieces(count: usize, mut each: impl FnMut(Piece)) {
    each(Piece::Zero);
    let mut rest = count - 1;
    while rest >= MAX_LENGTH {
        each(Piece::Copy(MAX_LENGTH));
        rest -= MAX_LENGTH;
    }
    if rest >= 3 {
        each(Piece::Copy(rest));
    } else {
        (0..rest).for_each(|_| each(Piece::Zero));
    }
}

/// The dynamic block header after BFINAL and BTYPE: the literal/length code
/// lengths and the one distance code, as code length symbols.
fn write_code_lengths(lengths: &[u8], bits: &mut Bits) {
    let literal_count = 257.max(lengths.iter().rposition(|&l| l > 0).map_or(0, |p| p + 1));
    let mut all = lengths[..literal_count].to_vec();
    // Only distance 1 is used: one code of one bit.
    all.push(1);
    let runs = rle(&all);
    let mut freqs = [0_u32; 19];
    for &(symbol, _) in &runs {
        freqs[usize::from(symbol)] += 1;
    }
    let code_length_lengths = code_lengths(&freqs, 7);
    let code_length_codes = codes(&code_length_lengths);
    let sent = 4.max(
        CODE_LENGTH_ORDER
            .iter()
            .rposition(|&s| code_length_lengths[s] > 0)
            .map_or(0, |p| p + 1),
    );
    bits.put((literal_count - 257) as u64, 5);
    bits.put(0, 5);
    bits.put((sent - 4) as u64, 4);
    for &symbol in &CODE_LENGTH_ORDER[..sent] {
        bits.put(u64::from(code_length_lengths[symbol]), 3);
    }
    for &(symbol, extra) in &runs {
        let s = usize::from(symbol);
        bits.put(
            u64::from(code_length_codes[s]),
            u32::from(code_length_lengths[s]),
        );
        let extra_bits = match symbol {
            16 => 2,
            17 => 3,
            18 => 7,
            _ => 0,
        };
        bits.put(u64::from(extra), extra_bits);
    }
}

/// Per copy length 3 to 258: its length symbol, extra bit count and value.
const LENGTHS: [(u16, u8, u8); 259] = length_table();

const fn length_table() -> [(u16, u8, u8); 259] {
    const BASE: [u16; 29] = [
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115,
        131, 163, 195, 227, 258,
    ];
    const EXTRA: [u8; 29] = [
        0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
    ];
    let mut table = [(0, 0, 0); 259];
    let mut code = 0;
    let mut length = 3;
    while length <= 258 {
        while code + 1 < 29 && BASE[code + 1] as usize <= length {
            code += 1;
        }
        table[length] = (
            257 + code as u16,
            EXTRA[code],
            (length - BASE[code] as usize) as u8,
        );
        length += 1;
    }
    table
}

/// Huffman code lengths for `freqs`, none longer than `limit`: frequencies
/// are halved until the tree is shallow enough.
fn code_lengths(freqs: &[u32], limit: u8) -> Vec<u8> {
    let mut freqs = freqs.to_vec();
    loop {
        let lengths = huffman(&freqs);
        if lengths.iter().all(|&l| l <= limit) {
            return lengths;
        }
        for freq in freqs.iter_mut().filter(|f| **f > 0) {
            *freq = (*freq / 2).max(1);
        }
    }
}

fn huffman(freqs: &[u32]) -> Vec<u8> {
    let mut lengths = vec![0; freqs.len()];
    let used: Vec<usize> = (0..freqs.len()).filter(|&s| freqs[s] > 0).collect();
    if let [only] = used[..] {
        lengths[only] = 1;
    }
    if used.len() < 2 {
        return lengths;
    }
    // Leaves first, then each merged node; a parent always comes later.
    let mut parent = vec![0; used.len() * 2 - 1];
    let mut heap: BinaryHeap<_> = used
        .iter()
        .enumerate()
        .map(|(node, &s)| Reverse((u64::from(freqs[s]), node)))
        .collect();
    let mut next = used.len();
    while let (Some(Reverse((a_freq, a))), Some(Reverse((b_freq, b)))) = (heap.pop(), heap.pop()) {
        parent[a] = next;
        parent[b] = next;
        heap.push(Reverse((a_freq + b_freq, next)));
        next += 1;
    }
    let mut depth = vec![0_u32; next];
    for node in (0..next - 1).rev() {
        depth[node] = depth[parent[node]] + 1;
    }
    for (node, &s) in used.iter().enumerate() {
        lengths[s] = depth[node].min(255) as u8;
    }
    lengths
}

/// Canonical codes for `lengths`, bit-reversed for the LSB-first stream.
fn codes(lengths: &[u8]) -> Vec<u16> {
    let mut count = [0_u16; 16];
    for &l in lengths.iter().filter(|&&l| l > 0) {
        count[usize::from(l)] += 1;
    }
    let mut next = [0_u16; 16];
    let mut code = 0;
    for bits in 1..16 {
        code = (code + count[bits - 1]) << 1;
        next[bits] = code;
    }
    lengths
        .iter()
        .map(|&l| {
            if l == 0 {
                return 0;
            }
            let code = next[usize::from(l)];
            next[usize::from(l)] += 1;
            code.reverse_bits() >> (16 - l)
        })
        .collect()
}

/// Code lengths as code length symbols: 16 repeats the last length 3 to 6
/// times, 17 and 18 stand for 3 to 10 and 11 to 138 zeros.
fn rle(lengths: &[u8]) -> Vec<(u8, u8)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lengths.len() {
        let length = lengths[i];
        let run = lengths[i..].iter().take_while(|&&l| l == length).count();
        let mut left = run;
        if length == 0 {
            while left >= 11 {
                let take = left.min(138);
                out.push((18, (take - 11) as u8));
                left -= take;
            }
            if left >= 3 {
                out.push((17, (left - 3) as u8));
                left = 0;
            }
        } else {
            out.push((length, 0));
            left -= 1;
            while left >= 3 {
                let take = left.min(6);
                out.push((16, (take - 3) as u8));
                left -= take;
            }
        }
        out.extend(std::iter::repeat_n((length, 0), left));
        i += run;
    }
    out
}

/// An LSB-first bit stream appended to `out`, 64 bits at a time.
pub(super) struct Bits {
    pub(super) out: Vec<u8>,
    held: u64,
    count: u32,
}

impl Bits {
    pub(super) fn new(out: Vec<u8>) -> Self {
        Self {
            out,
            held: 0,
            count: 0,
        }
    }

    /// `value`'s low `count` bits, up to 64.
    fn put(&mut self, value: u64, count: u32) {
        self.held |= value.checked_shl(self.count).unwrap_or(0);
        let total = self.count + count;
        if total < 64 {
            self.count = total;
            return;
        }
        self.out.extend_from_slice(&self.held.to_le_bytes());
        // What did not fit is the top of `value`.
        self.held = value.checked_shr(64 - self.count).unwrap_or(0);
        self.count = total - 64;
    }

    /// Pads with zero bits to the next byte and writes out what is held.
    fn align(&mut self) {
        let bytes = self.count.div_ceil(8) as usize;
        self.out
            .extend_from_slice(&self.held.to_le_bytes()[..bytes]);
        self.held = 0;
        self.count = 0;
    }

    pub(super) fn finish(mut self) -> Vec<u8> {
        self.align();
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(data: &[u8]) -> Vec<(usize, usize, bool)> {
        spans(data)
            .iter()
            .map(|span| match span {
                Span::Literals(range) => (range.start, range.len(), false),
                Span::Zeros(count) => (0, *count, true),
            })
            .collect()
    }

    #[test]
    fn zeros_across_blocks_become_runs_and_zeros_amid_bytes_stay_literals() {
        // Zeros from byte 5 through 24 span three blocks; the four at the
        // very end sit in the short tail after a non-zero byte.
        let mut data = vec![5, 0, 0, 0, 7];
        data.extend([0; 20]);
        data.extend([9, 0, 0, 0, 0]);
        assert_eq!(shape(&data), [(0, 5, false), (0, 20, true), (25, 5, false)]);
        // Five zeros inside one block, none at its end: literals.
        assert_eq!(shape(&[1, 0, 0, 0, 0, 0, 2, 3]), [(0, 8, false)]);
        assert_eq!(shape(&[0; 16]), [(0, 16, true)]);
    }

    #[test]
    fn runs_split_into_copies_of_at_most_258() {
        let mut pieces = Vec::new();
        run_pieces(1 + 258 + 2, |piece| {
            pieces.push(match piece {
                Piece::Zero => 0,
                Piece::Copy(length) => length,
            })
        });
        assert_eq!(pieces, [0, 258, 0, 0]);
    }
}
