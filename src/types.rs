use super::miracl_core::arch::Chunk;
use super::ECCurve::big::BIG;
use super::ECCurve::dbig::DBIG;
use super::ECCurve::ecp::ECP;
#[cfg(any(feature = "bls381", feature = "bn254"))]
use super::ECCurve::ecp2::ECP2;
pub use super::ECCurve::fp::FP;
#[cfg(any(feature = "bls381", feature = "bn254"))]
use super::ECCurve::fp12::FP12;
#[cfg(any(feature = "bls381", feature = "bn254"))]
pub use super::ECCurve::fp2::FP2;

pub type Limb = Chunk;
pub type BigNum = BIG;
pub type DoubleBigNum = DBIG;
pub type GroupG1 = ECP;
#[cfg(any(feature = "bls381", feature = "bn254"))]
pub type GroupG2 = ECP2;
#[cfg(any(feature = "bls381", feature = "bn254"))]
pub type GroupGT = FP12;
