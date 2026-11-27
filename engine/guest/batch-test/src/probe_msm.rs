#[cfg(test)]
pub mod probe {
    use super::*;
    /// The window size `msm` picks for these scalar bit lengths (the range's top read from this msm_ovm.rs by build.rs).
    pub fn pick(bits: &[usize]) -> usize {
        let max: usize = env!("HZ_MSM_MAX_C").parse().unwrap();
        (1..=max).min_by_key(|&c| cost(bits, c)).unwrap()
    }
    /// The signed digits `msm` stores for a scalar at window size c, as the i16 it keeps them in.
    pub fn stored_digits(s: &[u8], c: usize) -> Vec<(i32, i16)> {
        let b = bit_len(s);
        let mut v = alloc::vec![(0i32, 0i16); windows(b, c)];
        digits(s, b, c, |w, d| v[w] = (d, d as i16));
        v
    }
    pub fn run(c: &[Fr], b: &[P]) -> P { msm(c, b) }
}
