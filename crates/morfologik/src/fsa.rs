//! Reader for Morfologik's CFSA2 finite-state automaton format (magic `\fsa`,
//! version byte `0xC6`). This is the on-disk format LanguageTool ships its
//! `.dict` files in (both the spelling dictionaries and the POS dictionaries).
//!
//! Ported directly from the reference `morfologik.fsa.CFSA2` Java class so the
//! byte-level arc decoding is bug-for-bug identical. Arc offsets are indices
//! into `arcs`, the byte slice that follows the header + flags + label map.

/// This arc's target node immediately follows it (no explicit address stored).
const BIT_TARGET_NEXT: u8 = 1 << 7;
/// This arc is the last outgoing arc of its node.
const BIT_LAST_ARC: u8 = 1 << 6;
/// A word ends at this arc.
const BIT_FINAL_ARC: u8 = 1 << 5;
/// Low 5 bits of the flags byte hold an index into the label lookup table
/// (0 = the label byte is stored inline right after the flags byte).
const LABEL_INDEX_MASK: u8 = (1 << 5) - 1;

/// `NUMBERS` FSA flag: nodes are prefixed with a vint perfect-hash number.
const FLAG_NUMBERS: u16 = 1 << 8;

#[derive(Debug)]
pub enum FsaError {
    BadMagic,
    UnsupportedVersion(u8),
    Truncated,
}

impl std::fmt::Display for FsaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FsaError::BadMagic => write!(f, "not an FSA file (bad \\fsa magic)"),
            FsaError::UnsupportedVersion(v) => {
                write!(f, "unsupported FSA version 0x{v:02x} (only CFSA2 0xc6 supported)")
            }
            FsaError::Truncated => write!(f, "truncated FSA file"),
        }
    }
}

impl std::error::Error for FsaError {}

pub struct Cfsa2 {
    arcs: Vec<u8>,
    label_mapping: Vec<u8>,
    has_numbers: bool,
    root: usize,
}

impl Cfsa2 {
    /// Parse a CFSA2 FSA from the full bytes of a `.dict` file.
    pub fn parse(bytes: &[u8]) -> Result<Cfsa2, FsaError> {
        if bytes.len() < 5 {
            return Err(FsaError::Truncated);
        }
        if &bytes[0..4] != b"\\fsa" {
            return Err(FsaError::BadMagic);
        }
        let version = bytes[4];
        if version != 0xc6 {
            return Err(FsaError::UnsupportedVersion(version));
        }
        // Header (post magic+version): u16 big-endian flags, u8 label-map size,
        // then the label map, then the arcs.
        if bytes.len() < 8 {
            return Err(FsaError::Truncated);
        }
        let flag_bits = u16::from_be_bytes([bytes[5], bytes[6]]);
        let has_numbers = (flag_bits & FLAG_NUMBERS) != 0;
        let label_mapping_size = bytes[7] as usize;
        let map_start = 8;
        let arcs_start = map_start + label_mapping_size;
        if bytes.len() < arcs_start {
            return Err(FsaError::Truncated);
        }
        let label_mapping = bytes[map_start..arcs_start].to_vec();
        let arcs = bytes[arcs_start..].to_vec();

        let mut fsa = Cfsa2 {
            arcs,
            label_mapping,
            has_numbers,
            root: 0,
        };
        fsa.root = fsa.get_destination_node_offset(fsa.get_first_arc(0));
        Ok(fsa)
    }

    #[inline]
    pub fn root_node(&self) -> usize {
        self.root
    }

    #[inline]
    fn is_arc_last(&self, arc: usize) -> bool {
        (self.arcs[arc] & BIT_LAST_ARC) != 0
    }

    #[inline]
    fn is_next_set(&self, arc: usize) -> bool {
        (self.arcs[arc] & BIT_TARGET_NEXT) != 0
    }

    #[inline]
    pub fn is_arc_final(&self, arc: usize) -> bool {
        (self.arcs[arc] & BIT_FINAL_ARC) != 0
    }

    /// A terminal arc points at no node (it only marks word end).
    #[inline]
    pub fn is_arc_terminal(&self, arc: usize) -> bool {
        self.get_destination_node_offset(arc) == 0
    }

    #[inline]
    pub fn get_arc_label(&self, arc: usize) -> u8 {
        let index = (self.arcs[arc] & LABEL_INDEX_MASK) as usize;
        if index > 0 {
            self.label_mapping[index]
        } else {
            self.arcs[arc + 1]
        }
    }

    #[inline]
    pub fn get_first_arc(&self, node: usize) -> usize {
        if self.has_numbers {
            self.skip_vint(node)
        } else {
            node
        }
    }

    /// Next outgoing arc of the same node, or 0 if `arc` was the last.
    #[inline]
    pub fn get_next_arc(&self, arc: usize) -> usize {
        if self.is_arc_last(arc) {
            0
        } else {
            self.skip_arc(arc)
        }
    }

    /// Follow `arc` to the first arc of its destination node. Caller must ensure
    /// the arc is not terminal.
    #[inline]
    pub fn get_end_node(&self, arc: usize) -> usize {
        self.get_destination_node_offset(arc)
    }

    /// The outgoing arc of `node` labelled `label`, or 0 if none.
    pub fn get_arc(&self, node: usize, label: u8) -> usize {
        let mut arc = self.get_first_arc(node);
        while arc != 0 {
            if self.get_arc_label(arc) == label {
                return arc;
            }
            arc = self.get_next_arc(arc);
        }
        0
    }

    fn get_destination_node_offset(&self, arc: usize) -> usize {
        if self.is_next_set(arc) {
            // Target is the node right after this node's last arc.
            let mut a = arc;
            while !self.is_arc_last(a) {
                a = self.get_next_arc(a);
            }
            self.skip_arc(a)
        } else {
            let off = arc + if (self.arcs[arc] & LABEL_INDEX_MASK) == 0 { 2 } else { 1 };
            read_vint(&self.arcs, off)
        }
    }

    fn skip_arc(&self, mut offset: usize) -> usize {
        let flag = self.arcs[offset];
        offset += 1;
        if (flag & LABEL_INDEX_MASK) == 0 {
            offset += 1; // inline label byte
        }
        if (flag & BIT_TARGET_NEXT) == 0 {
            offset = self.skip_vint(offset); // explicit target address vint
        }
        offset
    }

    fn skip_vint(&self, mut offset: usize) -> usize {
        while (self.arcs[offset] as i8) < 0 {
            offset += 1;
        }
        offset + 1
    }

    /// Enumerate every byte sequence accepted by the automaton, invoking `emit`
    /// for each. Order is depth-first by ascending arc order (not sorted).
    pub fn for_each_sequence<F: FnMut(&[u8])>(&self, mut emit: F) {
        let mut buf: Vec<u8> = Vec::with_capacity(64);
        self.walk(self.root, &mut buf, &mut emit);
    }

    fn walk<F: FnMut(&[u8])>(&self, node: usize, buf: &mut Vec<u8>, emit: &mut F) {
        let mut arc = self.get_first_arc(node);
        while arc != 0 {
            buf.push(self.get_arc_label(arc));
            if self.is_arc_final(arc) {
                emit(buf);
            }
            if !self.is_arc_terminal(arc) {
                let next = self.get_end_node(arc);
                self.walk(next, buf, emit);
            }
            buf.pop();
            arc = self.get_next_arc(arc);
        }
    }
}

/// Variable-length integer decode, matching `CFSA2.readVInt`.
fn read_vint(array: &[u8], mut offset: usize) -> usize {
    let mut b = array[offset];
    let mut value = (b & 0x7f) as usize;
    let mut shift = 7;
    while (b as i8) < 0 {
        offset += 1;
        b = array[offset];
        value |= ((b & 0x7f) as usize) << shift;
        shift += 7;
    }
    value
}
