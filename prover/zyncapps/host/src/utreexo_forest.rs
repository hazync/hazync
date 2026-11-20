//! A compact full Utreexo forest for the tip mode's production witness source (docs/libzync/UTREEXO_ZAPP.md §10.5.1):
//! the same accumulator as rustreexo 0.7.0's `MemForest` (the crate the step guest and Floresta run), held in one arena
//! of 44-byte nodes addressed by u32 index instead of `Rc` nodes, with a leaf index keyed by 8 bytes of the leaf hash.
//!
//! Every operation is a line-for-line port of `rustreexo::mem_forest::MemForest` 0.7.0 (`add_single`, `del` /
//! `del_single`, `recompute_hashes`, `get_pos`, `grab_node`, `prove`), and the position arithmetic below is copied
//! verbatim from rustreexo 0.7.0 `src/util/mod.rs` (`pub(crate)` there; MIT OR Apache-2.0, Calvin Kim and Davidson
//! Souza). Node identity is the arena index where MemForest compares node hashes: the same thing while leaf hashes are
//! unique, which they are on Bitcoin (a leaf commits to its creating block's hash and its outpoint) and which `add`
//! enforces (a repeated live leaf is refused, never silently re-mapped).
//!
//! Nothing here is trusted by any proof: a proof it builds is checked natively by `Stump::modify` before proving and
//! again in the guest. The differential tests (zapps/utreexo/tip-step, `forest_*`) compare it with MemForest itself.
use rustreexo::node_hash::{AccumulatorHash, BitcoinNodeHash};
use rustreexo::proof::Proof;
use std::collections::HashMap;
use std::io::{Read, Write};

pub type H = [u8; 32];
const NIL: u32 = u32::MAX;
const MAX_FOREST_ROWS: u8 = 63;

#[derive(Clone, Copy)]
struct Node {
    hash: H,
    parent: u32,
    left: u32,
    right: u32,
}

/// A slot while walking down (MemForest's `Option<Rc<Node>>` over a root that may be an Empty branch).
#[derive(Clone, Copy, PartialEq)]
enum Slot {
    None,
    /// An Empty root: MemForest's childless Branch with `Hash::empty()`
    Empty,
    At(u32),
}

pub struct ArenaForest {
    nodes: Vec<Node>,
    free: Vec<u32>,
    /// NIL = an Empty root
    roots: Vec<u32>,
    pub leaves: u64,
    /// leaf hash prefix (8 bytes, little endian) -> leaf node; a second live leaf with the same prefix goes to `more`
    map: HashMap<u64, u32>,
    more: HashMap<H, u32>,
}

fn prefix(h: &H) -> u64 {
    u64::from_le_bytes(h[..8].try_into().unwrap())
}

fn bh(h: &H) -> BitcoinNodeHash {
    BitcoinNodeHash::Some(*h)
}

fn raw(n: &BitcoinNodeHash) -> Result<H, String> {
    match n {
        BitcoinNodeHash::Some(h) => Ok(*h),
        _ => Err("an Empty or placeholder leaf".into()),
    }
}

impl Default for ArenaForest {
    fn default() -> Self {
        Self::new()
    }
}

impl ArenaForest {
    pub fn new() -> Self {
        ArenaForest { nodes: Vec::new(), free: Vec::new(), roots: Vec::new(), leaves: 0, map: HashMap::new(), more: HashMap::new() }
    }

    /// Live nodes (leaves and branches) and live leaves.
    pub fn counts(&self) -> (usize, usize) {
        (self.nodes.len() - self.free.len(), self.map.len() + self.more.len())
    }

    /// Heap bytes held (capacities, not lengths): the arena, the free list, the leaf index (std HashMap: key, value and
    /// one control byte per bucket; an estimate of hashbrown's layout).
    pub fn heap_bytes(&self) -> usize {
        self.nodes.capacity() * std::mem::size_of::<Node>()
            + self.free.capacity() * 4
            + self.map.capacity() * (16 + 1)
            + self.more.capacity() * (36 + 1)
    }

    fn alloc(&mut self, n: Node) -> Result<u32, String> {
        if let Some(i) = self.free.pop() {
            self.nodes[i as usize] = n;
            return Ok(i);
        }
        if self.nodes.len() >= (NIL - 1) as usize {
            return Err("forest arena: more than 2^32 - 2 nodes".into());
        }
        if self.nodes.len() == self.nodes.capacity() {
            // grow by an eighth, not by doubling: at the tip the arena is ~15 GB (UTREEXO_ZAPP.md §10.5.1)
            self.nodes.reserve_exact(self.nodes.len() / 8 + 1024);
        }
        self.nodes.push(n);
        Ok((self.nodes.len() - 1) as u32)
    }

    fn release(&mut self, i: u32) {
        self.nodes[i as usize] = Node { hash: [0; 32], parent: NIL, left: NIL, right: NIL };
        self.free.push(i);
    }

    fn get(&self, h: &H) -> Option<u32> {
        if let Some(&i) = self.map.get(&prefix(h)) {
            if self.nodes[i as usize].hash == *h {
                return Some(i);
            }
        }
        self.more.get(h).copied()
    }

    fn index(&mut self, h: &H, i: u32) -> Result<(), String> {
        if self.get(h).is_some() {
            return Err(format!("leaf {} is already in the forest (a repeated live leaf)", hexs(h)));
        }
        match self.map.get(&prefix(h)) {
            None => {
                self.map.insert(prefix(h), i);
            }
            Some(_) => {
                self.more.insert(*h, i);
            }
        }
        Ok(())
    }

    fn unindex(&mut self, h: &H) -> Option<u32> {
        let p = prefix(h);
        if let Some(&i) = self.map.get(&p) {
            if self.nodes[i as usize].hash == *h {
                // a leaf in `more` with the same prefix stays there (`get` falls back to `more`); promoting it would cost a
                // scan of `more` on every such delete and changes no result (review F4)
                self.map.remove(&p);
                return Some(i);
            }
        }
        self.more.remove(h)
    }

    pub fn get_roots(&self) -> Vec<BitcoinNodeHash> {
        self.roots.iter().map(|&r| if r == NIL { BitcoinNodeHash::empty() } else { bh(&self.nodes[r as usize].hash) }).collect()
    }

    // ---- MemForest::modify / del / del_single / add_single / recompute_hashes

    pub fn modify(&mut self, add: &[BitcoinNodeHash], del: &[BitcoinNodeHash]) -> Result<(), String> {
        self.del(del)?;
        for a in add {
            self.add_single(raw(a)?)?;
        }
        Ok(())
    }

    fn del(&mut self, targets: &[BitcoinNodeHash]) -> Result<(), String> {
        let mut nodes = Vec::with_capacity(targets.len());
        for t in targets {
            let h = raw(t)?;
            let i = self.get(&h).ok_or("Could not find node")?;
            nodes.push((self.get_pos(i)?, h));
        }
        nodes.sort_by_key(|a| a.0);
        for (_, h) in nodes {
            match self.unindex(&h) {
                Some(i) => self.del_single(i),
                None => return Err(format!("node {} not in the forest", hexs(&h))),
            }
        }
        Ok(())
    }

    fn del_single(&mut self, i: u32) {
        let p = self.nodes[i as usize].parent;
        if p == NIL {
            // deleting a root: it becomes an Empty root
            let k = self.roots.iter().position(|&r| r == i).expect("a parentless node is a root");
            self.roots[k] = NIL;
            self.release(i);
            return;
        }
        let pn = self.nodes[p as usize];
        let sib = if pn.left == i { pn.right } else { pn.left };
        let gp = pn.parent;
        self.nodes[sib as usize].parent = gp;
        if gp != NIL {
            let g = &mut self.nodes[gp as usize];
            if g.left == p {
                g.left = sib;
            } else {
                g.right = sib;
            }
            self.recompute_hashes(sib);
        } else {
            let k = self.roots.iter().position(|&r| r == p).expect("a parentless branch is a root");
            self.roots[k] = sib;
        }
        self.release(p);
        self.release(i);
    }

    fn recompute_hashes(&mut self, mut n: u32) {
        loop {
            let x = self.nodes[n as usize];
            if x.left != NIL && x.right != NIL {
                let h = BitcoinNodeHash::parent_hash(&bh(&self.nodes[x.left as usize].hash), &bh(&self.nodes[x.right as usize].hash));
                self.nodes[n as usize].hash = raw(&h).unwrap();
            }
            if x.parent == NIL {
                return;
            }
            n = x.parent;
        }
    }

    fn add_single(&mut self, value: H) -> Result<(), String> {
        let mut node = self.alloc(Node { hash: value, parent: NIL, left: NIL, right: NIL })?;
        if let Err(e) = self.index(&value, node) {
            self.release(node);
            return Err(e);
        }
        let mut leaves = self.leaves;
        while leaves & 1 != 0 {
            let root = self.roots.pop().unwrap();
            if root == NIL {
                leaves >>= 1;
                continue;
            }
            let h = BitcoinNodeHash::parent_hash(&bh(&self.nodes[root as usize].hash), &bh(&self.nodes[node as usize].hash));
            let new = self.alloc(Node { hash: raw(&h).unwrap(), parent: NIL, left: root, right: node })?;
            self.nodes[root as usize].parent = new;
            self.nodes[node as usize].parent = new;
            node = new;
            leaves >>= 1;
        }
        self.roots.push(node);
        self.leaves += 1;
        Ok(())
    }

    // ---- MemForest::get_pos / grab_node / get_hash / prove

    fn get_pos(&self, i: u32) -> Result<u64, String> {
        let mut left_child_indicator = 0u64;
        let mut rows_to_top = 0;
        let mut n = i;
        loop {
            let p = self.nodes[n as usize].parent;
            if p == NIL {
                break;
            }
            left_child_indicator <<= 1;
            if self.nodes[p as usize].left != n {
                left_child_indicator |= 1;
            }
            rows_to_top += 1;
            n = p;
        }
        let forest_rows = tree_rows(self.leaves);
        let mut root_idx = self.roots.len() as i64 - 1;
        let mut root_row = None;
        for row in 0..=forest_rows {
            if is_root_populated(row, self.leaves) {
                if root_idx < 0 {
                    break;
                }
                if self.roots[root_idx as usize] == n {
                    root_row = Some(row);
                    break;
                }
                root_idx -= 1;
            }
        }
        let root_row = root_row.ok_or(format!("Could not find the root position for row {root_idx}"))?;
        let mut pos = root_position(self.leaves, root_row, forest_rows);
        for _ in 0..rows_to_top {
            match left_child_indicator & 1 {
                0 => pos = left_child(pos, forest_rows),
                _ => pos = right_child(pos, forest_rows),
            }
            left_child_indicator >>= 1;
        }
        Ok(pos)
    }

    fn child(&self, s: Slot, right: bool) -> Slot {
        match s {
            Slot::At(i) => {
                let n = &self.nodes[i as usize];
                let c = if right { n.right } else { n.left };
                if c == NIL { Slot::None } else { Slot::At(c) }
            }
            _ => Slot::None,
        }
    }

    /// MemForest::grab_node: (node, sibling, parent) at `pos`; the walk follows MemForest's niece convention exactly.
    fn grab_node(&self, pos: u64) -> Result<(Slot, Slot, Slot), String> {
        let (tree, branch_len, bits) = detect_offset(pos, self.leaves);
        let r = *self.roots.get(tree as usize).ok_or(format!("node {pos} not found"))?;
        let root = if r == NIL { Slot::Empty } else { Slot::At(r) };
        let (mut n, mut sibling, mut parent) = (root, root, root);
        for row in (0..branch_len).rev() {
            parent = sibling;
            let niece_pos = (bits >> row) & 1;
            if n != Slot::None {
                if is_left_niece(niece_pos) {
                    sibling = self.child(n, false);
                    n = self.child(n, true);
                } else {
                    sibling = self.child(n, true);
                    n = self.child(n, false);
                }
            } else {
                sibling = Slot::None;
            }
        }
        if n != Slot::None && sibling != Slot::None && parent != Slot::None {
            return Ok((n, sibling, parent));
        }
        Err(format!("node {pos} not found"))
    }

    fn get_hash(&self, pos: u64) -> Result<BitcoinNodeHash, String> {
        match self.grab_node(pos)?.0 {
            Slot::At(i) => Ok(bh(&self.nodes[i as usize].hash)),
            _ => Ok(BitcoinNodeHash::empty()),
        }
    }

    pub fn prove(&self, targets: &[BitcoinNodeHash]) -> Result<Proof, String> {
        let mut positions = Vec::with_capacity(targets.len());
        for t in targets {
            let i = self.get(&raw(t)?).ok_or("Could not find node")?;
            positions.push(self.get_pos(i)?);
        }
        let rows = tree_rows(self.leaves);
        let needed = get_proof_positions(&positions, self.leaves, rows);
        let hashes = needed.iter().map(|p| self.get_hash(*p)).collect::<Result<Vec<_>, _>>()?;
        let targets = positions.into_iter().map(|p| translate(p, rows, MAX_FOREST_ROWS)).collect();
        Ok(Proof::new_with_hash(targets, hashes))
    }

    // ---- persistence: leaves u64 ‖ roots u64 ‖ live nodes u64 ‖ live leaves u64 ‖ preorder per root (0 = Empty root,
    // 1 = leaf ‖ hash, 2 = branch ‖ hash, then its children)

    pub fn write_to<W: Write>(&self, w: &mut W) -> std::io::Result<()> {
        let (nodes, live) = self.counts();
        w.write_all(&self.leaves.to_le_bytes())?;
        w.write_all(&(self.roots.len() as u64).to_le_bytes())?;
        w.write_all(&(nodes as u64).to_le_bytes())?;
        w.write_all(&(live as u64).to_le_bytes())?;
        let mut stack = Vec::new();
        for &r in &self.roots {
            if r == NIL {
                w.write_all(&[0])?;
                continue;
            }
            stack.push(r);
            while let Some(i) = stack.pop() {
                let n = &self.nodes[i as usize];
                if n.left == NIL {
                    w.write_all(&[1])?;
                    w.write_all(&n.hash)?;
                } else {
                    w.write_all(&[2])?;
                    w.write_all(&n.hash)?;
                    stack.push(n.right);
                    stack.push(n.left);
                }
            }
        }
        Ok(())
    }

    /// Reads [`write_to`]'s format and checks it: every branch hash is recomputed from its children and must equal the
    /// stored one (a store that lies about an inner node is refused here), every tree's height is the one its root
    /// row says, and no leaf hash repeats.
    /// `max_bytes`: the bytes the forest can occupy in its file; the stored counts are refused above what that many
    /// bytes can hold before anything is allocated (a corrupt count must not abort the allocator, review F5).
    pub fn read_from<R: Read>(r: &mut R, max_bytes: u64) -> Result<Self, String> {
        let mut f = ArenaForest::new();
        let mut b8 = [0u8; 8];
        r.read_exact(&mut b8).map_err(|e| e.to_string())?;
        f.leaves = u64::from_le_bytes(b8);
        r.read_exact(&mut b8).map_err(|e| e.to_string())?;
        let nroots = u64::from_le_bytes(b8);
        if nroots != f.leaves.count_ones() as u64 {
            return Err("forest: not one root per set bit of the leaf count".into());
        }
        r.read_exact(&mut b8).map_err(|e| e.to_string())?;
        let want_nodes = u64::from_le_bytes(b8);
        r.read_exact(&mut b8).map_err(|e| e.to_string())?;
        let want_live = u64::from_le_bytes(b8);
        // a node takes at least 33 bytes in the file (tag and hash)
        if want_live > f.leaves || want_nodes > 2 * want_live || want_nodes >= (NIL - 1) as u64 || want_nodes.saturating_mul(33) > max_bytes {
            return Err("forest: impossible node or leaf counts".into());
        }
        // the arena sized once, with an eighth to grow into (a run's adds), not by doubling
        f.nodes.reserve_exact(want_nodes as usize + want_nodes as usize / 8 + 1024);
        f.map.reserve(want_live as usize);
        // root k (from the left) is the tree of the k-th highest set bit
        let rows: Vec<u8> = (0..64u8).rev().filter(|b| f.leaves >> b & 1 == 1).collect();
        for &want_rows in &rows {
            let mut tag = [0u8; 1];
            r.read_exact(&mut tag).map_err(|e| e.to_string())?;
            if tag[0] == 0 {
                f.roots.push(NIL);
                continue;
            }
            let root = read_node(r, tag[0], &mut f, NIL)?;
            // depth-first, explicit stack of (branch, children read so far, depth)
            let mut stack: Vec<(u32, u8, u8)> = Vec::new();
            let mut max_depth = 0u8;
            if f.nodes[root as usize].left == 0 {
                stack.push((root, 0, 0));
            }
            while let Some(&mut (b, ref mut got, d)) = stack.last_mut() {
                if *got == 2 {
                    let n = f.nodes[b as usize];
                    let h = BitcoinNodeHash::parent_hash(&bh(&f.nodes[n.left as usize].hash), &bh(&f.nodes[n.right as usize].hash));
                    if raw(&h).unwrap() != n.hash {
                        return Err(format!("forest: a branch hash ({}) is not the hash of its children", hexs(&n.hash)));
                    }
                    stack.pop();
                    continue;
                }
                *got += 1;
                let right = *got == 2;
                if d + 1 > MAX_FOREST_ROWS {
                    return Err("forest: deeper than 63 rows".into());
                }
                let mut tag = [0u8; 1];
                r.read_exact(&mut tag).map_err(|e| e.to_string())?;
                let c = read_node(r, tag[0], &mut f, b)?;
                if right { f.nodes[b as usize].right = c } else { f.nodes[b as usize].left = c }
                max_depth = max_depth.max(d + 1);
                if f.nodes[c as usize].left == 0 {
                    stack.push((c, 0, d + 1));
                }
            }
            if max_depth > want_rows {
                return Err("forest: a tree deeper than its root's row".into());
            }
            f.roots.push(root);
        }
        if f.counts() != (want_nodes as usize, want_live as usize) {
            return Err("forest: the node or leaf count is not the stored one".into());
        }
        Ok(f)
    }

    /// Recomputes every branch hash from the leaves and compares the roots (a full check of the arena, for tests).
    #[allow(dead_code)] // the tests' (zapps/utreexo/tip-step)
    pub fn check(&self) -> Result<(), String> {
        fn up(f: &ArenaForest, i: u32) -> Result<H, String> {
            let n = &f.nodes[i as usize];
            if n.left == NIL {
                return Ok(n.hash);
            }
            let (l, r) = (up(f, n.left)?, up(f, n.right)?);
            if f.nodes[n.left as usize].parent != i || f.nodes[n.right as usize].parent != i {
                return Err("forest: a child's parent link is wrong".into());
            }
            let h = raw(&BitcoinNodeHash::parent_hash(&bh(&l), &bh(&r))).unwrap();
            if h != n.hash {
                return Err("forest: a branch hash is stale".into());
            }
            Ok(h)
        }
        for &r in &self.roots {
            if r != NIL {
                up(self, r)?;
            }
        }
        Ok(())
    }

    /// TEST ONLY (UTREEXO_TIP_SOURCE_LIE=store): flips one byte of every inner node that is not a root, without
    /// recomputing anything, as a store that lies about inner nodes would. Leaves and roots stay, so lookups work and the
    /// roots still equal the previous statement's: only a deletion's proof carries the lie.
    pub fn corrupt_inner_nodes(&mut self) -> usize {
        let roots: std::collections::HashSet<u32> = self.roots.iter().copied().collect();
        let free: std::collections::HashSet<u32> = self.free.iter().copied().collect();
        let mut n = 0;
        for i in 0..self.nodes.len() as u32 {
            let x = &mut self.nodes[i as usize];
            if x.left != NIL && !roots.contains(&i) && !free.contains(&i) {
                x.hash[0] ^= 1;
                n += 1;
            }
        }
        n
    }
}

/// One node of the preorder; a branch's children are read by the caller. A branch is marked by `left = 0` until its
/// children are linked (0 is never a child of a node read later: index 0 is always the first root read).
fn read_node<R: Read>(r: &mut R, tag: u8, f: &mut ArenaForest, parent: u32) -> Result<u32, String> {
    let mut h = [0u8; 32];
    r.read_exact(&mut h).map_err(|e| e.to_string())?;
    match tag {
        1 => {
            let i = f.alloc(Node { hash: h, parent, left: NIL, right: NIL })?;
            f.index(&h, i).map_err(|e| format!("forest: {e}"))?;
            Ok(i)
        }
        2 => {
            // children pending: left = 0 marks "a branch whose children are still to be read" (fixed when linked)
            f.alloc(Node { hash: h, parent, left: 0, right: 0 })
        }
        _ => Err("forest: a bad node tag".into()),
    }
}

pub fn hexs(b: &H) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ---------------------------------------------------------------------------------------------------------------
// Copied verbatim from rustreexo 0.7.0 src/util/mod.rs (pub(crate) there), MIT OR Apache-2.0. Do not edit: the
// positions must be rustreexo's own.

fn is_root_position(position: u64, num_leaves: u64, forest_rows: u8) -> bool {
    let row = detect_row(position, forest_rows);
    let root_present = num_leaves & (1 << row) != 0;
    let root_pos = root_position(num_leaves, row, forest_rows);
    root_present && root_pos == position
}

fn translate(pos: u64, from_rows: u8, to_rows: u8) -> u64 {
    let row = detect_row(pos, from_rows);
    if row == 0 {
        return pos;
    }
    let offset = pos - start_position_at_row(row, from_rows);
    offset + start_position_at_row(row, to_rows)
}

fn start_position_at_row(row: u8, forest_rows: u8) -> u64 {
    ((2_u128 << forest_rows) - (2_u128 << (forest_rows - row))) as u64
}

fn is_left_niece(position: u64) -> bool {
    position & 1 == 0
}

fn detect_row(pos: u64, forest_rows: u8) -> u8 {
    let mut marker: u64 = 1 << forest_rows;
    let mut h: u8 = 0;
    while pos & marker != 0 {
        marker >>= 1;
        h += 1;
    }
    h
}

fn detect_offset(pos: u64, num_leaves: u64) -> (u8, u8, u64) {
    let mut tr = tree_rows(num_leaves);
    let nr = detect_row(pos, tr);
    let mut bigger_trees: u8 = 0;
    let mut marker = pos;
    while (marker << nr) & ((2 << tr) - 1) >= (1 << tr) & num_leaves {
        let tree_size = (1 << tr) & num_leaves;
        if tree_size != 0 {
            marker -= tree_size;
            bigger_trees += 1;
        }
        tr -= 1;
    }
    (bigger_trees, tr - nr, !marker)
}

fn children(pos: u64, forest_rows: u8) -> u64 {
    let mask = (2 << forest_rows) - 1;
    (pos << 1) & mask
}

fn left_child(pos: u64, forest_rows: u8) -> u64 {
    children(pos, forest_rows)
}

fn right_child(pos: u64, forest_rows: u8) -> u64 {
    children(pos, forest_rows) + 1
}

fn is_root_populated(row: u8, num_leaves: u64) -> bool {
    (num_leaves >> row) & 1 == 1
}

fn parent(pos: u64, forest_rows: u8) -> u64 {
    (pos >> 1) | (1 << forest_rows)
}

fn tree_rows(n: u64) -> u8 {
    if n == 0 {
        return 0;
    }
    (64 - (n - 1).leading_zeros()) as u8
}

fn root_position(num_leaves: u64, row: u8, forest_rows: u8) -> u64 {
    let mask = (2 << forest_rows) - 1;
    let before = num_leaves & (mask << (row + 1));
    let shifted = (before >> row) | (mask << (forest_rows + 1 - row));
    shifted & mask
}

fn get_proof_positions(targets: &[u64], num_leaves: u64, forest_rows: u8) -> Vec<u64> {
    let mut proof_positions = std::collections::BTreeSet::new();
    let mut map = std::collections::HashSet::new();
    let mut computed_positions = targets.to_vec();
    computed_positions.reserve(targets.len() * 2);
    map.reserve(targets.len() * 2);
    map.extend(targets.iter().copied());
    let mut i = 0;
    while i < computed_positions.len() {
        let pos = computed_positions[i];
        if is_root_position(pos, num_leaves, forest_rows) {
            i += 1;
            continue;
        }
        let sibling = pos ^ 1;
        if !map.contains(&sibling) {
            proof_positions.insert(sibling);
        }
        if map.contains(&sibling) {
            proof_positions.remove(&pos);
        }
        let parent = parent(pos, forest_rows);
        if !map.contains(&parent) {
            computed_positions.push(parent);
            map.insert(parent);
        }
        i += 1;
    }
    proof_positions.into_iter().collect()
}
