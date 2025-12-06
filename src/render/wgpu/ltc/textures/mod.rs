pub mod ltc_ggx;
pub mod ltc_oren_nayar;

//16384 = 64 * 64 * 4
pub const LTC_LUT_SIZE: usize = 64;

pub use ltc_ggx::*;
pub use ltc_oren_nayar::*;
