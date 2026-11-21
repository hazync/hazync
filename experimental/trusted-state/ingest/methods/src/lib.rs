include!(concat!(env!("OUT_DIR"), "/methods.rs"));
/// Built with TS_CONTROLS=1: the chunk / bip30 / combine / join guests read a cut mask first (negative controls only).
pub const CONTROLS: bool = cfg!(ts_controls);
