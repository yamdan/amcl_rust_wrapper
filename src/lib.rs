#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
pub extern crate miracl_core;

#[macro_use]
extern crate lazy_static;

#[cfg(feature = "bn254")]
pub use miracl_core::bn254 as ECCurve;

#[cfg(feature = "bls381")]
pub use miracl_core::bls12381 as ECCurve;

#[cfg(feature = "secp256k1")]
pub use miracl_core::secp256k1 as ECCurve;

#[cfg(feature = "ed25519")]
pub use miracl_core::ed25519 as ECCurve;

#[macro_use]
extern crate serde;

extern crate serde_json;

#[cfg(feature="rayon")]
extern crate rayon;

extern crate subtle_encoding;

pub mod constants;
pub mod types;

#[macro_use]
pub mod errors;

#[macro_use]
pub mod utils;

#[macro_use]
pub mod field_elem;
#[macro_use]
pub mod group_elem;
#[macro_use]
pub mod group_elem_g1;
pub mod commitment;
#[macro_use]
pub mod univar_poly;

#[cfg(any(feature = "bls381", feature = "bn254"))]
pub mod types_g2;

#[cfg(any(feature = "bls381", feature = "bn254"))]
#[macro_use]
pub mod group_elem_g2;

#[cfg(any(feature = "bls381", feature = "bn254"))]
#[macro_use]
pub mod extension_field_gt;

// TODO: Move the timing tests to benchmark
