// The C / C++ heap of the libzync guest (hazync#794): malloc, free, calloc and realloc for Bitcoin Core and libsecp,
// with freed blocks RECYCLED.
//
// WHY. These used to hand every block to risc0's bump allocator, which never reclaims. Core frees as it goes (every
// stack element a script copies is a std::vector), so one Core-valid tapscript input could exhaust the ~3 GB guest
// heap on its own: a ~4 MB leaf of OP_3DUP over 520-byte items allocates ~5 GB in total while never holding more than
// a few MB at once. Core accepts it in under a second; the guest aborted, and no chunk split can help a single input.
//
// WHAT. Segregated free lists over the same bump allocator. A block's size class is fixed by its size; free() pushes
// the block on its class's list and malloc() pops from it before taking fresh memory. No search, no splitting, no
// coalescing: both are O(1), and a class can only ever hold as much as was once live in it at the same time, so the
// heap is bounded by what Core holds live, not by what it ever allocated.
//
// ONLY the C heap. Rust's allocator stays the bump allocator, so a Rust buffer's address still names one buffer for
// the whole run; verify_input.cpp's cached_tx / cached_spent are keyed by such addresses (the witness buffers main.rs
// passes in) and main() checks the property at start-up. The one cache keyed by C-heap addresses, Core's legacy
// sighash cache (patches/libzync/0101, keyed by a transaction's vin / vout buffers), is invalidated here: free() clears
// `hz_lsh_key[0]` when either buffer it is keyed by is freed, so an address can never be matched after the buffer it
// named is gone, whatever later lands there. It also keys on hz_tx_generation, which every script entry point bumps.
//
// Classes (16-byte header included): 16-byte steps up to 2 KiB, then four per power of two up to 1 GiB (at most 25%
// slack). A block above 1 GiB is never recycled, as before. Header: [class, capacity, next, state], state = FREE_MARK
// while the block is on a list. A double free, or a free of a pointer this heap never handed out, panics (no receipt)
// instead of putting a block on a list twice: the bump allocator made both harmless, recycling would not.
use std::alloc::{alloc as ralloc, Layout};

pub const HDR: usize = 16;
const SMALL_MAX: usize = 2048;
const N_SMALL: usize = SMALL_MAX / 16; // classes 1..=128 (0 unused: no block is smaller than its header)
const K_MIN: u32 = 11; // octaves 2^11 .. 2^29, four classes each
const K_MAX: u32 = 29;
const N_CLASSES: usize = N_SMALL + 1 + 4 * (K_MAX - K_MIN + 1) as usize;
const UNPOOLED: usize = usize::MAX;
const FREE_MARK: usize = 0x4652_4545; // "FREE"
const LIVE_MARK: usize = 0x4c49_5645; // "LIVE"

static mut FREE: [*mut u8; N_CLASSES] = [core::ptr::null_mut(); N_CLASSES];

#[allow(non_upper_case_globals)]
extern "C" {
    // verify_input.cpp. [0] = the vin buffer, [1] = the vout buffer the legacy sighash cache was filled from; the
    // cache refills whenever [0] is null.
    static mut hz_lsh_key: [*const u8; 2];
}

/// (class, capacity including the header) for a block of `total` bytes, total > HDR.
#[inline(always)]
fn class_of(total: usize) -> (usize, usize) {
    if total <= SMALL_MAX {
        let c = (total + 15) >> 4;
        return (c, c << 4);
    }
    let t = total - 1;
    let k = usize::BITS - 1 - t.leading_zeros(); // 2^k <= t < 2^(k+1), k >= 11
    if k > K_MAX {
        return (UNPOOLED, total);
    }
    let q = (t >> (k - 2)) & 3; // which quarter of the octave
    (N_SMALL + 1 + 4 * (k - K_MIN) as usize + q, (1usize << k) + ((q + 1) << (k - 2)))
}

#[inline(always)]
unsafe fn word(base: *mut u8, i: usize) -> *mut usize {
    (base as *mut usize).add(i)
}

#[no_mangle]
pub unsafe extern "C" fn malloc(size: usize) -> *mut u8 {
    if size == 0 {
        return core::ptr::null_mut();
    }
    // C's malloc fails on a size it cannot represent (review 11-14: `size + HDR` wrapped to a tiny block)
    let Some(total) = size.checked_add(HDR) else { return core::ptr::null_mut() };
    let (c, cap) = class_of(total);
    let free = &mut *core::ptr::addr_of_mut!(FREE);
    if c != UNPOOLED && !free[c].is_null() {
        let base = free[c];
        free[c] = *word(base, 2) as *mut u8;
        *word(base, 3) = LIVE_MARK;
        #[cfg(feature = "heap-poison")]
        core::ptr::write_bytes(base.add(HDR), 0x5a, cap - HDR);
        return base.add(HDR);
    }
    let Ok(layout) = Layout::from_size_align(cap, HDR) else { return core::ptr::null_mut() };
    let base = ralloc(layout);
    if base.is_null() {
        return base;
    }
    *word(base, 0) = c;
    *word(base, 1) = cap;
    *word(base, 3) = LIVE_MARK;
    base.add(HDR)
}

#[no_mangle]
pub unsafe extern "C" fn free(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    let key = &mut *core::ptr::addr_of_mut!(hz_lsh_key);
    if ptr as *const u8 == key[0] || ptr as *const u8 == key[1] {
        key[0] = core::ptr::null();
    }
    let base = ptr.sub(HDR);
    assert!(*word(base, 3) == LIVE_MARK, "C heap: free() of a block that is not live (a double free?)");
    *word(base, 3) = FREE_MARK;
    let c = *word(base, 0);
    #[cfg(feature = "heap-poison")]
    core::ptr::write_bytes(ptr, 0xa5, *word(base, 1) - HDR);
    if c == UNPOOLED {
        return; // never recycled (above 1 GiB), as the bump allocator never recycled anything
    }
    let free = &mut *core::ptr::addr_of_mut!(FREE);
    *word(base, 2) = free[c] as usize;
    free[c] = base;
}

#[no_mangle]
pub unsafe extern "C" fn calloc(n: usize, sz: usize) -> *mut u8 {
    // C's calloc fails on overflow instead of returning a short block (review 11-03).
    let Some(total) = n.checked_mul(sz) else { return core::ptr::null_mut() };
    let p = malloc(total);
    if !p.is_null() {
        core::ptr::write_bytes(p, 0, total);
    }
    p
}

#[no_mangle]
pub unsafe extern "C" fn realloc(ptr: *mut u8, size: usize) -> *mut u8 {
    if ptr.is_null() {
        return malloc(size);
    }
    let room = *word(ptr.sub(HDR), 1) - HDR;
    if size != 0 && size <= room {
        return ptr; // still fits its block
    }
    let np = malloc(size);
    if !np.is_null() {
        core::ptr::copy_nonoverlapping(ptr, np, core::cmp::min(room, size));
        free(ptr);
    }
    np
}

/// main()'s start-up check: a freed block is handed out again (so the guest is running THIS allocator), and freeing
/// a buffer the legacy sighash cache is keyed by clears its key (so the cache cannot match a reused address).
pub fn self_check() {
    unsafe {
        let a = malloc(64);
        free(a);
        let b = malloc(64);
        assert!(!a.is_null() && a == b, "the C heap does not recycle a freed block");
        let key = &mut *core::ptr::addr_of_mut!(hz_lsh_key);
        let saved = *key;
        key[0] = b;
        key[1] = core::ptr::null();
        free(b);
        assert!(key[0].is_null(), "freeing the legacy sighash cache's vin buffer did not invalidate it");
        let c = malloc(64);
        key[0] = 1 as *const u8;
        key[1] = c;
        free(c);
        assert!(key[0].is_null(), "freeing the legacy sighash cache's vout buffer did not invalidate it");
        *key = saved;
    }
}
