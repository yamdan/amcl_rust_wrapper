use rand::{CryptoRng, RngCore};

use crate::constants::{
    BarrettRedc_k, BarrettRedc_u, BarrettRedc_v, BigNumBits, CurveOrder, FieldElement_SIZE, NLEN,
};
use crate::errors::{SerzDeserzError, ValueError};
use crate::types::{BigNum, DoubleBigNum, Limb};
use crate::utils::{barrett_reduction, get_seeded_RNG, get_seeded_RNG_with_rng, hash_msg};
use miracl_core::rand::RAND;
use core::fmt;
use std::cmp::Ordering;
use std::hash::{Hash, Hasher};
use std::ops::{Add, AddAssign, Index, IndexMut, Mul, Neg, RangeBounds, Sub, SubAssign};
use std::slice::Iter;

use serde::de::{Deserialize, Deserializer, Error as DError, Visitor};
use serde::ser::{Error as SError, Serialize, Serializer};

#[cfg(feature = "rayon")]
use crate::rayon::iter::IntoParallelRefMutIterator;
#[cfg(feature = "rayon")]
use rayon::prelude::*;

use zeroize::Zeroize;

/// Don't derive Copy trait as it can hold secret data and should not be accidentally copied
#[derive(Clone)]
pub struct FieldElement {
    value: BigNum,
}

impl fmt::Display for FieldElement {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let big = self.value.clone();
        write!(f, "BIG: [ {} ]", big.tostring())
    }
}

impl fmt::Debug for FieldElement {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let big = self.value.clone();
        write!(f, "BIG: [ {} ]", big.tostring())
    }
}

impl Hash for FieldElement {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let mut bytes: [u8; FieldElement_SIZE] = [0; FieldElement_SIZE];
        self.write_to_slice_unchecked(&mut bytes);
        state.write(&bytes)
    }
}

impl Default for FieldElement {
    fn default() -> Self {
        Self::new()
    }
}

impl Zeroize for FieldElement {
    fn zeroize(&mut self) {
        self.value.w.zeroize();
    }
}

impl Drop for FieldElement {
    fn drop(&mut self) {
        self.zeroize()
    }
}

/// Represents an element of the prime field of the curve. All operations are done modulo the curve order
impl FieldElement {
    /// Creates a new field element with value 0
    pub fn new() -> Self {
        Self {
            value: BigNum::new(),
        }
    }

    pub fn zero() -> Self {
        Self {
            value: BigNum::new(),
        }
    }

    pub fn one() -> Self {
        Self {
            value: BigNum::new_int(1),
        }
    }

    pub fn minus_one() -> Self {
        let mut o = Self::one();
        o.negate();
        o
    }

    /// Return a random non-zero field element
    pub fn random() -> Self {
        Self::random_field_element().into()
    }

    /// Return a random non-zero field element using the given random number generator
    pub fn random_using_rng<R: RngCore + CryptoRng>(rng: &mut R) -> Self {
        Self::random_field_element_using_rng(rng).into()
    }

    pub fn is_zero(&self) -> bool {
        BigNum::iszilch(&self.value)
    }

    pub fn is_one(&self) -> bool {
        BigNum::isunity(&self.value)
    }

    /// Return bytes in MSB form
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes: [u8; FieldElement_SIZE] = [0; FieldElement_SIZE];
        self.write_to_slice_unchecked(&mut bytes);
        bytes.to_vec()
    }

    /// Expects bytes in MSB form
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SerzDeserzError> {
        if bytes.len() != FieldElement_SIZE {
            return Err(SerzDeserzError::FieldElementBytesIncorrectSize(
                bytes.len(),
                FieldElement_SIZE,
            ));
        }
        let mut n = BigNum::frombytes(bytes);
        n.rmod(&CurveOrder);
        Ok(Self { value: n })
    }

    /// Writes bytes in MSB form to given slice. Raises exception when given slice is not of
    /// desired length.
    pub fn write_to_slice(&self, target: &mut [u8]) -> Result<(), SerzDeserzError> {
        if target.len() != FieldElement_SIZE {
            return Err(SerzDeserzError::FieldElementBytesIncorrectSize(
                target.len(),
                FieldElement_SIZE,
            ));
        }
        self.write_to_slice_unchecked(target);
        Ok(())
    }

    /// Writes bytes in MSB form to given slice. Will panic when given slice is not of
    /// desired length.
    pub fn write_to_slice_unchecked(&self, target: &mut [u8]) {
        let mut temp = BigNum::new_copy(&self.value);
        temp.tobytes(target);
    }

    pub fn to_bignum(&self) -> BigNum {
        let mut v = self.value.clone();
        v.rmod(&CurveOrder);
        v
    }

    pub fn normalize(&mut self) {
        self.value.norm();
    }

    /// Hash an arbitrary sized message using SHAKE and return output as a field element
    pub fn from_msg_hash(msg: &[u8]) -> Self {
        // TODO: Ensure result is not 0
        let h = &hash_msg(msg);
        h.into()
    }

    /// Add a field element to itself. `self = self + b`
    pub fn add_assign_(&mut self, b: &Self) {
        // Not using `self.plus` to avoid cloning. Breaking the abstraction a bit for performance.
        self.value.add(&b.value);
        self.value.rmod(&CurveOrder);
    }

    /// Subtract a field element from itself. `self = self - b`
    pub fn sub_assign_(&mut self, b: &Self) {
        // Not using `self.minus` to avoid cloning. Breaking the abstraction a bit for performance.
        let neg_b = BigNum::modneg(&b.value, &CurveOrder);
        self.value.add(&neg_b);
        self.value.rmod(&CurveOrder);
    }

    /// Return sum of a field element and itself. `self + b`
    pub fn plus(&self, b: &Self) -> Self {
        let mut sum = self.value.clone();
        sum.add(&b.value);
        sum.rmod(&CurveOrder);
        sum.into()
    }

    /// Return difference of a field element and itself. `self - b`
    pub fn minus(&self, b: &Self) -> Self {
        let mut sum = self.value.clone();
        let neg_b = BigNum::modneg(&b.value, &CurveOrder);
        sum.add(&neg_b);
        sum.rmod(&CurveOrder);
        sum.into()
    }

    /// Multiply 2 field elements modulus the order of the curve.
    /// (field_element_a * field_element_b) % curve_order
    pub fn multiply(&self, b: &Self) -> Self {
        let d = BigNum::mul(&self.value, &b.value);
        Self::reduce_dmod_curve_order(&d).into()
    }

    /// Calculate square of a field element modulo the curve order, i.e `a^2 % curve_order`
    pub fn square(&self) -> Self {
        let d = BigNum::sqr(&self.value);
        Self::reduce_dmod_curve_order(&d).into()
    }

    /// Exponentiation modulo curve order, i.e. self^exp % CurveOrder
    pub fn pow(&self, exp: &Self) -> Self {
        let mut base = self.value.clone();
        let res = base.powmod(&exp.value, &CurveOrder);
        res.into()
    }

    /// Return negative of field element
    pub fn negation(&self) -> Self {
        let zero = Self::zero();
        zero.minus(&self)
    }

    pub fn negate(&mut self) {
        let zero = Self::zero();
        self.value = zero.minus(&self).value;
    }

    /// Calculate inverse of a field element modulo the curve order, i.e `a^-1 % curve_order`
    pub fn inverse(&self) -> Self {
        // Violating constant time guarantee until bug fixed in amcl
        if self.is_zero() {
            return Self::zero();
        }
        let mut inv = self.value.clone();
        inv.invmodp(&CurveOrder);
        inv.into()
    }

    pub fn inverse_mut(&mut self) {
        // Violating constant time guarantee until bug fixed in amcl
        if self.is_zero() {
            self.value = BigNum::new();
        } else {
            self.value.invmodp(&CurveOrder);
        }
    }

    pub fn shift_right(&self, k: usize) -> Self {
        let mut t = self.value.clone();
        t.shr(k);
        t.into()
    }

    pub fn shift_left(&self, k: usize) -> Self {
        let mut t = self.value.clone();
        t.shl(k);
        t.into()
    }

    pub fn is_even(&self) -> bool {
        self.value.parity() == 0
    }

    pub fn is_odd(&self) -> bool {
        !self.is_even()
    }

    /// Gives vectors of bit-vectors for the Big number. Each limb has a separate bit-vector,
    /// hence upto NLEN bit-vectors possible. Least significant bytes come first.
    /// NOT SIDE CHANNEL RESISTANT
    pub fn to_bitvectors(&self) -> Vec<Vec<u8>> {
        let mut k = NLEN - 1;
        let mut s = BigNum::new_copy(&self.value);
        s.norm();

        while (k as isize) >= 0 && s.w[k] == 0 {
            k = k.wrapping_sub(1)
        }
        if (k as isize) < 0 {
            return vec![];
        }

        let mut b_vec: Vec<Vec<u8>> = vec![vec![]; k + 1];
        for i in 0..k + 1 {
            let mut c = s.w[i];
            let mut c_vec: Vec<u8> = vec![];
            while c != 0 {
                c_vec.push((c % 2) as u8);
                c /= 2;
            }
            b_vec[i] = c_vec;
        }
        return b_vec;
    }

    /// Returns bits. Least significant bits come first
    pub fn to_bits(&self) -> Vec<u8> {
        let mut bit_vecs = self.to_bitvectors();
        let mut bits = vec![];
        for mut bit_vec in bit_vecs.drain(..) {
            let len = bit_vec.len();
            bits.append(&mut bit_vec);
            bits.append(&mut vec![0; BigNumBits - len]);
        }
        bits
    }

    /// Return a random non-zero field element using given random number generator
    fn random_field_element_using_rng<R: RngCore + CryptoRng>(rng: &mut R) -> BigNum {
        // initialise from at least 128 byte string of raw random entropy
        let entropy_size = 256;
        let mut r = get_seeded_RNG_with_rng(entropy_size, rng);
        Self::get_big_num_from_RAND(&mut r)
    }

    fn random_field_element() -> BigNum {
        // initialise from at least 128 byte string of raw random entropy
        let entropy_size = 256;
        let mut r = get_seeded_RNG(entropy_size);
        Self::get_big_num_from_RAND(&mut r)
    }

    fn get_big_num_from_RAND(r: &mut RAND) -> BigNum {
        let mut n = BigNum::randomnum(&BigNum::new_big(&CurveOrder), r);
        while n.iszilch() {
            n = BigNum::randomnum(&BigNum::new_big(&CurveOrder), r);
        }
        n
    }

    /// Conversion to wNAF, i.e. windowed Non Adjacent form
    /// Taken from Guide to Elliptic Curve Cryptography book, "Algorithm 3.35 Computing the width-w NAF of a positive integer" with modification
    /// at step 2.1, if k_i >= 2^(w-1), k_i = k_i - 2^w
    pub fn to_wnaf(&self, w: usize) -> Vec<i8> {
        // required by the NAF definition
        debug_assert!(w >= 2);
        // required so that the NAF digits fit in i8
        debug_assert!(w <= 8);

        // Working on the the underlying BIG to save the cost of to and from conversion with FieldElement
        let mut k = self.to_bignum();
        let mut naf: Vec<i8> = vec![];

        let two_w_1 = 1 << (w - 1); // 2^(w-1)
        let two_w = 1 << w; // 2^w

        // While k is not zero
        while !k.iszilch() {
            // If k is odd
            let t = if k.parity() == 1 {
                let mut b = k.clone();
                // b = b % 2^w
                b.mod2m(w);

                // Only the first limb is useful as b <2^w
                let mut u = b.w[0];
                if u >= two_w_1 {
                    u = u - two_w;
                }

                k.w[0] = k.w[0] - u;
                u as i8
            } else {
                0i8
            };
            naf.push(t);
            k.fshr(1usize);
        }

        naf
    }

    /// Convert to base that is power of 2. Does not handle negative nos or `base` higher than 2^7
    pub fn to_power_of_2_base(&self, n: usize) -> Vec<u8> {
        debug_assert!(n <= 7);

        if self.is_zero() {
            return vec![0u8];
        }
        let mut t = self.to_bignum();
        t.norm();

        let mut base_repr = vec![];
        while !t.iszilch() {
            let mut d = t.clone();
            d.mod2m(n);
            base_repr.push(d.w[0] as u8);
            t.fshr(n);
        }
        base_repr
    }

    /// Convert to base that is power of 2. Does not handle negative nos or `base` higher than 2^7
    pub fn from_power_of_2_base(repr: &[u8], n: usize) -> Self {
        debug_assert!(n <= 7);

        let mut accum = FieldElement::zero();
        let mut factor = FieldElement::one().to_bignum();
        for i in 0..repr.len() {
            accum += FieldElement::from(factor) * FieldElement::from(repr[i]);
            factor.fshl(n);
        }
        accum
    }

    /// Return n-th bit, n starts from 0
    pub fn nth_bit(&self, n: usize) -> u8 {
        self.value.bit(n) as u8
    }

    pub fn or(&mut self, other: &Self) {
        self.value.or(&other.value);
    }

    /// Takes a bunch of field elements and returns the inverse of all field elements.
    /// Also returns the product of all inverses as its computed as a side effect.
    /// For an input of n elements, rather than doing n inversions, does only 1 inversion but 3n multiplications.
    /// eg `batch_invert([a, b, c, d])` returns ([1/a, 1/b, 1/c, 1/d], 1/a * 1/b * 1/c * 1/d)
    /// Algorithm taken from Guide to Elliptic Curve Cryptography book, "Algorithm 2.26 Simultaneous inversion"
    pub fn batch_invert(elems: &[Self]) -> (Vec<Self>, Self) {
        debug_assert!(elems.len() > 0);

        let k = elems.len();

        // TODO: Multiplications below can be sped up by using montgomery multiplication.

        // Construct c as [elems[0], elems[0]*elems[1], elems[0]*elems[1]*elems[2], .... elems[0]*elems[1]*elems[2]*...elems[k-1]]
        let mut c: Vec<Self> = vec![elems[0].clone()];
        for i in 1..k {
            c.push(&c[i - 1] * &elems[i])
        }

        // u = 1 / elems[0]*elems[1]*elems[2]*...elems[k-1]
        let all_inv = c[k - 1].inverse();
        let mut u = all_inv.clone();
        let mut inverses = vec![FieldElement::one(); k];

        for i in (1..k).rev() {
            inverses[i] = &u * &c[i - 1];
            u = &u * &elems[i];
        }

        inverses[0] = u;

        (inverses, all_inv)
    }

    /// Returns hex string in big endian
    pub fn to_hex(&self) -> String {
        // TODO: Make constant time.
        self.to_bignum().tostring()
    }

    /// Create big number from hex string in big endian
    pub fn from_hex(mut s: String) -> Result<Self, SerzDeserzError> {
        let mut f = Self::parse_hex_as_bignum(s)?;
        f.rmod(&CurveOrder);
        Ok(f.into())
    }

    /// Useful for reducing product of BigNums. Uses Barrett reduction
    pub fn reduce_dmod_curve_order(x: &DoubleBigNum) -> BigNum {
        barrett_reduction(
            &x,
            &CurveOrder,
            *BarrettRedc_k,
            &*BarrettRedc_u,
            &*BarrettRedc_v,
        )
    }

    /// Parse given hex string as BigNum in constant time.
    pub fn parse_hex_as_bignum(mut val: String) -> Result<BigNum, SerzDeserzError> {
        // Logic almost copied from AMCL but with error handling and constant time execution.
        // Constant time is important as hex is used during serialization and deserialization.
        // A seemingly effortless solution is to filter string for errors and pad with 0s before
        // passing to AMCL but that would be expensive as the string is scanned twice

        // Given hex cannot be bigger than max byte size
        if val.len() > FieldElement_SIZE * 2 {
            return Err(SerzDeserzError::FieldElementBytesIncorrectSize(
                val.len(),
                FieldElement_SIZE,
            ));
        }

        // Pad the string for constant time parsing.
        while val.len() < FieldElement_SIZE * 2 {
            val.insert(0, '0');
        }

        let mut res = BigNum::new();
        for i in 0..val.len() {
            match u8::from_str_radix(&val[i..i + 1], 16) {
                Ok(n) => res.w[0] += n as Limb,
                Err(_) => return Err(SerzDeserzError::RequiredHexChar),
            }
            if i == (val.len() - 1) {
                break;
            }
            res.shl(4);
        }
        return Ok(res);
    }
}

impl Serialize for FieldElement {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_newtype_struct("FieldElement", &self.to_hex())
    }
}
impl<'a> Deserialize<'a> for FieldElement {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'a>,
    {
        struct FieldElementVisitor;

        impl<'a> Visitor<'a> for FieldElementVisitor {
            type Value = FieldElement;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("expected FieldElement")
            }

            fn visit_str<E>(self, value: &str) -> Result<FieldElement, E>
            where
                E: DError,
            {
                Ok(FieldElement::from_hex(value.to_string()).map_err(DError::custom)?)
            }
        }

        deserializer.deserialize_str(FieldElementVisitor)
    }
}

impl From<u8> for FieldElement {
    fn from(x: u8) -> Self {
        Self {
            value: BigNum::new_int(x as isize),
        }
    }
}

impl From<u32> for FieldElement {
    fn from(x: u32) -> Self {
        Self {
            value: BigNum::new_int(x as isize),
        }
    }
}

impl From<u64> for FieldElement {
    fn from(x: u64) -> Self {
        Self {
            value: BigNum::new_int(x as isize),
        }
    }
}

impl From<i32> for FieldElement {
    fn from(x: i32) -> Self {
        Self {
            value: BigNum::new_int(x as isize),
        }
    }
}

impl From<BigNum> for FieldElement {
    fn from(x: BigNum) -> Self {
        Self { value: x }
    }
}

impl From<&[u8; FieldElement_SIZE]> for FieldElement {
    fn from(x: &[u8; FieldElement_SIZE]) -> Self {
        let mut n = BigNum::frombytes(x);
        n.rmod(&CurveOrder);
        Self { value: n }
    }
}

impl PartialEq for FieldElement {
    fn eq(&self, other: &FieldElement) -> bool {
        BigNum::comp(&self.value, &other.value) == 0
    }
}

impl PartialOrd for FieldElement {
    fn partial_cmp(&self, other: &FieldElement) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Eq for FieldElement {}

impl Ord for FieldElement {
    fn cmp(&self, other: &FieldElement) -> Ordering {
        match BigNum::comp(&self.value, &other.value) {
            0 => Ordering::Equal,
            -1 => Ordering::Less,
            _ => Ordering::Greater,
        }
    }
}

impl Add for FieldElement {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        self.plus(&other)
    }
}

impl Add<FieldElement> for &FieldElement {
    type Output = FieldElement;

    fn add(self, other: FieldElement) -> FieldElement {
        self.plus(&other)
    }
}

impl<'a> Add<&'a FieldElement> for FieldElement {
    type Output = Self;
    fn add(self, other: &'a FieldElement) -> Self {
        self.plus(other)
    }
}

impl<'a> Add<&'a FieldElement> for &FieldElement {
    type Output = FieldElement;
    fn add(self, other: &'a FieldElement) -> FieldElement {
        self.plus(other)
    }
}

impl AddAssign for FieldElement {
    fn add_assign(&mut self, other: Self) {
        self.add_assign_(&other)
    }
}

impl<'a> AddAssign<&'a FieldElement> for FieldElement {
    fn add_assign(&mut self, other: &'a FieldElement) {
        self.add_assign_(other)
    }
}

impl Sub for FieldElement {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        self.minus(&other)
    }
}

impl Sub<FieldElement> for &FieldElement {
    type Output = FieldElement;

    fn sub(self, other: FieldElement) -> FieldElement {
        self.minus(&other)
    }
}

impl<'a> Sub<&'a FieldElement> for FieldElement {
    type Output = Self;

    fn sub(self, other: &'a FieldElement) -> Self {
        self.minus(&other)
    }
}

impl<'a> Sub<&'a FieldElement> for &FieldElement {
    type Output = FieldElement;

    fn sub(self, other: &'a FieldElement) -> FieldElement {
        self.minus(&other)
    }
}

impl SubAssign for FieldElement {
    fn sub_assign(&mut self, other: Self) {
        self.sub_assign_(&other)
    }
}

impl<'a> SubAssign<&'a FieldElement> for FieldElement {
    fn sub_assign(&mut self, other: &'a Self) {
        self.sub_assign_(other)
    }
}

impl Mul for FieldElement {
    type Output = Self;

    fn mul(self, other: Self) -> Self {
        self.multiply(&other)
    }
}

impl Mul<FieldElement> for &FieldElement {
    type Output = FieldElement;

    fn mul(self, other: FieldElement) -> FieldElement {
        self.multiply(&other)
    }
}

impl<'a> Mul<&'a FieldElement> for FieldElement {
    type Output = FieldElement;

    fn mul(self, other: &'a FieldElement) -> FieldElement {
        self.multiply(other)
    }
}

impl<'a> Mul<&'a FieldElement> for &FieldElement {
    type Output = FieldElement;

    fn mul(self, other: &'a FieldElement) -> FieldElement {
        self.multiply(other)
    }
}

impl Neg for FieldElement {
    type Output = Self;

    fn neg(self) -> Self::Output {
        self.negation()
    }
}

impl Neg for &FieldElement {
    type Output = FieldElement;

    fn neg(self) -> Self::Output {
        self.negation()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FieldElementVector {
    elems: Vec<FieldElement>,
}

impl FieldElementVector {
    /// Creates a new field element vector with each element being 0
    // FIXME: size should have a type like u64 since usize can be small on older/smaller machines. This code
    // is less likely to be used on older/smaller machines though
    #[cfg(feature = "rayon")]
    pub fn new(size: usize) -> Self {
        Self {
            elems: (0..size)
                .into_par_iter()
                .map(|_| FieldElement::new())
                .collect(),
        }
    }

    #[cfg(not(feature = "rayon"))]
    pub fn new(size: usize) -> Self {
        Self {
            elems: (0..size).into_iter().map(|_| FieldElement::new()).collect(),
        }
    }

    /// Generate a Vandermonde vector of field elements as:
    /// FieldElementVector::new_vandermonde_vector(k, n) => vec![1, k, k^2, k^3, ... k^n-1]
    /// FieldElementVector::new_vandermonde_vector(0, n) => vec![0, 0, ... n times]
    pub fn new_vandermonde_vector(elem: &FieldElement, size: usize) -> Self {
        if size == 0 {
            Self::new(0)
        } else if elem.is_zero() {
            Self::new(size)
        } else if elem.is_one() {
            vec![FieldElement::one(); size].into()
        } else {
            let mut v = Vec::<FieldElement>::with_capacity(size);
            v.push(FieldElement::one());
            for i in 1..size {
                v.push(&v[i - 1] * elem);
            }
            v.into()
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            elems: Vec::<FieldElement>::with_capacity(capacity),
        }
    }

    /// Get a vector of random field elements
    #[cfg(feature = "rayon")]
    pub fn random(size: usize) -> Self {
        (0..size)
            .into_par_iter()
            .map(|_| FieldElement::random())
            .collect::<Vec<FieldElement>>()
            .into()
    }

    #[cfg(not(feature = "rayon"))]
    pub fn random(size: usize) -> Self {
        (0..size)
            .into_iter()
            .map(|_| FieldElement::random())
            .collect::<Vec<FieldElement>>()
            .into()
    }

    pub fn as_slice(&self) -> &[FieldElement] {
        self.elems.as_slice()
    }

    pub fn as_mut_slice(&mut self) -> &mut [FieldElement] {
        self.elems.as_mut_slice()
    }

    pub fn len(&self) -> usize {
        self.elems.len()
    }

    pub fn push(&mut self, value: FieldElement) {
        self.elems.push(value)
    }

    pub fn append(&mut self, other: &mut Self) {
        self.elems.append(&mut other.elems)
    }

    pub fn pop(&mut self) -> Option<FieldElement> {
        self.elems.pop()
    }

    pub fn insert(&mut self, index: usize, element: FieldElement) {
        self.elems.insert(index, element)
    }

    pub fn remove(&mut self, index: usize) -> FieldElement {
        self.elems.remove(index)
    }

    /// Multiply each element of the vector with a given field
    /// element `n` (scale the vector). Modifies the vector.
    #[cfg(feature = "rayon")]
    pub fn scale(&mut self, n: &FieldElement) {
        self.elems.as_mut_slice().par_iter_mut().for_each(|e| {
            *e = e.multiply(n);
        })
    }

    #[cfg(not(feature = "rayon"))]
    pub fn scale(&mut self, n: &FieldElement) {
        self.elems.as_mut_slice().iter_mut().for_each(|e| {
            *e = e.multiply(n);
        })
    }

    /// Multiply each element of the vector with a given field
    /// element `n` to create a new vector
    pub fn scaled_by(&self, n: &FieldElement) -> Self {
        let mut scaled = self.clone();
        scaled.scale(n);
        scaled.into()
    }

    /// Add 2 vectors of field elements
    #[cfg(feature = "rayon")]
    pub fn plus(&self, b: &FieldElementVector) -> Result<FieldElementVector, ValueError> {
        check_vector_size_for_equality!(self, b)?;
        let mut sum_vector = Self::new(self.len());
        sum_vector
            .as_mut_slice()
            .par_iter_mut()
            .enumerate()
            .for_each(|(i, e)| *e = &self[i] + &b[i]);
        Ok(sum_vector)
    }

    #[cfg(not(feature = "rayon"))]
    pub fn plus(&self, b: &FieldElementVector) -> Result<FieldElementVector, ValueError> {
        check_vector_size_for_equality!(self, b)?;
        let mut sum_vector = Self::new(self.len());
        sum_vector
            .as_mut_slice()
            .iter_mut()
            .enumerate()
            .for_each(|(i, e)| *e = &self[i] + &b[i]);
        Ok(sum_vector)
    }

    /// Subtract 2 vectors of field elements
    #[cfg(feature = "rayon")]
    pub fn minus(&self, b: &FieldElementVector) -> Result<FieldElementVector, ValueError> {
        check_vector_size_for_equality!(self, b)?;
        let mut diff_vector = Self::new(self.len());
        diff_vector
            .as_mut_slice()
            .par_iter_mut()
            .enumerate()
            .for_each(|(i, e)| *e = &self[i] - &b[i]);
        Ok(diff_vector)
    }

    #[cfg(not(feature = "rayon"))]
    pub fn minus(&self, b: &FieldElementVector) -> Result<FieldElementVector, ValueError> {
        check_vector_size_for_equality!(self, b)?;
        let mut diff_vector = Self::new(self.len());
        diff_vector
            .as_mut_slice()
            .iter_mut()
            .enumerate()
            .for_each(|(i, e)| *e = &self[i] - &b[i]);
        Ok(diff_vector)
    }

    /// Compute sum of all elements of a vector
    #[cfg(feature = "rayon")]
    pub fn sum(&self) -> FieldElement {
        self.as_slice()
            .par_iter()
            .cloned()
            .reduce(|| FieldElement::new(), |a, b| a + b)
    }

    #[cfg(not(feature = "rayon"))]
    pub fn sum(&self) -> FieldElement {
        self.as_slice()
            .iter()
            .cloned()
            .fold(FieldElement::new(), |a, b| a + b)
    }

    /// Computes inner product of 2 vectors of field elements
    /// [a1, a2, a3, ...field elements].[b1, b2, b3, ...field elements] = (a1*b1 + a2*b2 + a3*b3) % curve_order
    #[cfg(feature = "rayon")]
    pub fn inner_product(&self, b: &FieldElementVector) -> Result<FieldElement, ValueError> {
        check_vector_size_for_equality!(self, b)?;
        let r = (0..b.len())
            .into_par_iter()
            .map(|i| (&self[i] * &b[i]))
            .reduce(|| FieldElement::new(), |a, b| a + b);
        Ok(r)
    }

    #[cfg(not(feature = "rayon"))]
    pub fn inner_product(&self, b: &FieldElementVector) -> Result<FieldElement, ValueError> {
        check_vector_size_for_equality!(self, b)?;
        let r = (0..b.len())
            .into_iter()
            .map(|i| (&self[i] * &b[i]))
            .fold(FieldElement::new(), |a, b| a + b);
        Ok(r)
    }

    /// Calculates Hadamard product of 2 field element vectors.
    /// Hadamard product of `a` and `b` = `a` o `b` = (a0 o b0, a1 o b1, ...).
    /// Here `o` denotes multiply operation
    #[cfg(feature = "rayon")]
    pub fn hadamard_product(
        &self,
        b: &FieldElementVector,
    ) -> Result<FieldElementVector, ValueError> {
        check_vector_size_for_equality!(self, b)?;
        let mut hadamard_product = Self::new(self.len());
        hadamard_product
            .as_mut_slice()
            .par_iter_mut()
            .enumerate()
            .for_each(|(i, e)| *e = &self[i] * &b[i]);
        Ok(hadamard_product)
    }

    #[cfg(not(feature = "rayon"))]
    pub fn hadamard_product(
        &self,
        b: &FieldElementVector,
    ) -> Result<FieldElementVector, ValueError> {
        check_vector_size_for_equality!(self, b)?;
        let mut hadamard_product = Self::new(self.len());
        hadamard_product
            .as_mut_slice()
            .iter_mut()
            .enumerate()
            .for_each(|(i, e)| *e = &self[i] * &b[i]);
        Ok(hadamard_product)
    }

    pub fn split_at(&self, mid: usize) -> (Self, Self) {
        let (l, r) = self.as_slice().split_at(mid);
        (Self::from(l), Self::from(r))
    }

    /// Replace a range `R` of the vector with `I`. Same as Vector's splice except it does not return
    /// anything. Only available to this crate for now for some manipulations in Polynomial
    pub(crate) fn splice<R, I>(&mut self, range: R, replace_with: I)
    where
        R: RangeBounds<usize>,
        I: IntoIterator<Item = FieldElement>,
    {
        self.elems.splice(range, replace_with);
    }

    pub fn iter(&self) -> Iter<FieldElement> {
        self.as_slice().iter()
    }
}

impl From<Vec<FieldElement>> for FieldElementVector {
    fn from(x: Vec<FieldElement>) -> Self {
        Self { elems: x }
    }
}

impl From<&[FieldElement]> for FieldElementVector {
    fn from(x: &[FieldElement]) -> Self {
        Self { elems: x.to_vec() }
    }
}

impl Into<Vec<FieldElement>> for FieldElementVector {
    fn into(self) -> Vec<FieldElement> {
        self.elems
    }
}

impl<'a> Into<&'a [FieldElement]> for &'a FieldElementVector {
    fn into(self) -> &'a [FieldElement] {
        &self.elems
    }
}

impl Index<usize> for FieldElementVector {
    type Output = FieldElement;

    fn index(&self, idx: usize) -> &FieldElement {
        &self.elems[idx]
    }
}

impl IndexMut<usize> for FieldElementVector {
    fn index_mut(&mut self, idx: usize) -> &mut FieldElement {
        &mut self.elems[idx]
    }
}

impl PartialEq for FieldElementVector {
    fn eq(&self, other: &Self) -> bool {
        if self.len() != other.len() {
            return false;
        }
        for i in 0..self.len() {
            if self[i] != other[i] {
                return false;
            }
        }
        true
    }
}

impl IntoIterator for FieldElementVector {
    type Item = FieldElement;
    type IntoIter = ::std::vec::IntoIter<FieldElement>;

    fn into_iter(self) -> Self::IntoIter {
        self.elems.into_iter()
    }
}

impl AsRef<[FieldElement]> for FieldElementVector {
    fn as_ref(&self) -> &[FieldElement] {
        self.elems.as_slice()
    }
}

// TODO: Implement add/sub/mul ops but need some way to handle error when vectors are of different length

pub fn multiply_row_vector_with_matrix(
    vector: &FieldElementVector,
    matrix: &Vec<FieldElementVector>,
) -> Result<FieldElementVector, ValueError> {
    check_vector_size_for_equality!(vector, matrix)?;
    let out_len = matrix[0].len();
    let mut out = FieldElementVector::new(out_len);
    for i in 0..out_len {
        for j in 0..vector.len() {
            out[i] += &vector[j] * &matrix[j][i];
        }
    }
    Ok(out)
}

#[cfg(test)]
mod test {
    use super::*;
    use rand::Rng;
    use serde_json;
    use std::collections::{HashMap, HashSet};
    use std::time::Instant;

    #[test]
    fn test_to_and_from_bytes() {
        let mut rng = rand::thread_rng();
        for _ in 0..100 {
            let x = FieldElement::random_using_rng(&mut rng);

            let mut bytes_x: [u8; FieldElement_SIZE] = [0; FieldElement_SIZE];
            x.write_to_slice(&mut bytes_x).unwrap();
            let y = FieldElement::from(&bytes_x);
            assert_eq!(x, y);

            let z = FieldElement::from_bytes(&x.to_bytes()).unwrap();
            assert_eq!(x, z);

            let y1 = FieldElement::random_using_rng(&mut rng);
            let z1 = FieldElement::from_bytes(&y1.to_bytes()).unwrap();
            assert_ne!(x, z1);
        }
    }

    #[test]
    fn test_equality() {
        for _ in 0..10 {
            // Very unlikely that 2 randomly chosen elements will be equal
            let a = FieldElement::random();
            let b = FieldElement::random();
            assert_ne!(&a, &b);
        }
    }

    #[test]
    fn test_field_elem_multiplication() {
        let a: FieldElement = 5u8.into();
        let b: FieldElement = 18u8.into();
        let c: FieldElement = 90u8.into();
        assert_eq!(a.multiply(&b), c);
        assert_eq!(a * b, c);
    }

    #[test]
    fn test_inversion() {
        assert_eq!(FieldElement::zero().inverse(), FieldElement::zero());
        assert_eq!(FieldElement::one().inverse(), FieldElement::one());
        let mut zero = FieldElement::zero();
        zero.inverse_mut();
        assert_eq!(zero, FieldElement::zero());
        for _ in 0..10 {
            let x = FieldElement::random();
            let x_inv = x.inverse();
            assert_eq!(x * x_inv, FieldElement::one())
        }
    }

    #[test]
    fn test_pow() {
        for _ in 0..5 {
            let base = FieldElement::random();

            let base_sqr = base.square();
            assert_eq!(base.pow(&FieldElement::from(2u64)), base_sqr);

            let base_cube = &base_sqr * &base;
            assert_eq!(base.pow(&FieldElement::from(3u64)), base_cube);

            let base_4 = base_sqr.square();
            assert_eq!(base.pow(&FieldElement::from(4u64)), base_4);

            let base_5 = &base_4 * &base;
            assert_eq!(base.pow(&FieldElement::from(5u64)), base_5);

            /*// Check base^r1 * base^r2 = base^(r1 + r2)
            let r1 = FieldElement::random();
            let r2 = FieldElement::random();
            let mut r3 = &r1 + &r2;
            assert_eq!(&r1 + &r2, r3);
            let x = &base.pow(&r1) * &base.pow(&r2);
            r3.normalize();
            let y = base.pow(&r3);
            //assert_eq!(base.pow(&r1) * base.pow(&r2), base.pow(&(&r1 + &r2)));
            assert_eq!(y, x);*/
        }
    }

    #[test]
    fn test_nth_bit() {
        assert_eq!(FieldElement::one().nth_bit(0), 1);
        assert_eq!(FieldElement::one().nth_bit(1), 0);
        assert_eq!(FieldElement::one().nth_bit(2), 0);

        assert_eq!(FieldElement::from(2u64).nth_bit(0), 0);
        assert_eq!(FieldElement::from(2u64).nth_bit(1), 1);
        assert_eq!(FieldElement::from(2u64).nth_bit(2), 0);
        assert_eq!(FieldElement::from(2u64).nth_bit(3), 0);

        assert_eq!(FieldElement::from(3u64).nth_bit(0), 1);
        assert_eq!(FieldElement::from(3u64).nth_bit(1), 1);
        assert_eq!(FieldElement::from(3u64).nth_bit(2), 0);
        assert_eq!(FieldElement::from(3u64).nth_bit(3), 0);

        let mut rng = rand::thread_rng();

        for _ in 0..10 {
            let r = FieldElement::random();
            let bits = r.to_bits();
            // Check bits at 100 random indices
            for _ in 0..100 {
                let idx = rng.gen_range(0, bits.len());
                assert_eq!(r.nth_bit(idx), bits[idx]);
            }
        }
    }

    #[test]
    fn test_or() {
        let mut a = FieldElement::one();
        let b = FieldElement::from(2);
        a.or(&b);
        assert_eq!(a, FieldElement::from(3));

        let mut a = FieldElement::from(4);
        a.or(&b);
        assert_eq!(a, FieldElement::from(6));

        for _ in 0..100 {
            let r1 = FieldElement::random();
            let r2 = FieldElement::random();
            // r3 = r1 | r2
            let mut r3 = r1.clone();
            r3.or(&r2);

            // Check that the OR operation was done correctly
            let r1_bits = r1.to_bits();
            let r2_bits = r2.to_bits();
            let r3_bits = r3.to_bits();

            let r1_len = r1_bits.len();
            let r2_len = r2_bits.len();
            for i in 0..r3_bits.len() {
                let r1_bit = if i < r1_len { r1_bits[i] } else { 0 };
                let r2_bit = if i < r2_len { r2_bits[i] } else { 0 };
                assert_eq!(r3_bits[i], r1_bit | r2_bit);
            }
        }
    }

    #[test]
    fn test_field_elements_inner_product() {
        let a: FieldElementVector = vec![
            FieldElement::from(5),
            FieldElement::one(),
            FieldElement::from(100),
            FieldElement::zero(),
        ]
        .into();
        let b: FieldElementVector = vec![
            FieldElement::from(18),
            FieldElement::one(),
            FieldElement::from(200),
            FieldElement::zero(),
        ]
        .into();
        let c = FieldElement::from((90 + 1 + 200 * 100) as u32);
        assert_eq!(a.inner_product(&b).unwrap(), c);
    }

    #[test]
    fn test_field_elements_hadamard_product() {
        let a: FieldElementVector = vec![
            FieldElement::from(5),
            FieldElement::one(),
            FieldElement::from(100),
            FieldElement::zero(),
        ]
        .into();
        let b: FieldElementVector = vec![
            FieldElement::from(18),
            FieldElement::one(),
            FieldElement::from(200),
            FieldElement::zero(),
        ]
        .into();
        let h: FieldElementVector = vec![
            FieldElement::from(90),
            FieldElement::one(),
            FieldElement::from(200 * 100),
            FieldElement::zero(),
        ]
        .into();
        let c = FieldElement::from((90 + 1 + 200 * 100) as u32);
        assert_eq!(a.hadamard_product(&b).unwrap(), h);
        assert_eq!(h.sum(), c);
    }

    #[test]
    fn test_scale_field_element_vector() {
        let a: FieldElementVector = vec![
            FieldElement::from(5),
            FieldElement::from(1),
            FieldElement::from(100),
            FieldElement::from(0),
        ]
        .into();
        let n = FieldElement::from(3);
        let na = a.scaled_by(&n);
        assert_eq!(na[0], FieldElement::from(5 * 3));
        assert_eq!(na[1], FieldElement::from(1 * 3));
        assert_eq!(na[2], FieldElement::from(100 * 3));
        assert_eq!(na[3], FieldElement::from(0));
    }

    #[test]
    fn test_add_field_element_vectors() {
        let a: FieldElementVector = vec![
            FieldElement::from(5),
            FieldElement::one(),
            FieldElement::from(100),
            FieldElement::zero(),
        ]
        .into();
        let b: FieldElementVector = vec![
            FieldElement::from(18),
            FieldElement::one(),
            FieldElement::from(200),
            FieldElement::zero(),
        ]
        .into();
        let c = a.plus(&b).unwrap();
        assert_eq!(c[0], FieldElement::from(5 + 18));
        assert_eq!(c[1], FieldElement::from(1 + 1));
        assert_eq!(c[2], FieldElement::from(100 + 200));
        assert_eq!(c[3], FieldElement::from(0));
    }

    #[test]
    fn test_field_elem_vandermonde_vector() {
        let zero_vec = FieldElementVector::new_vandermonde_vector(&FieldElement::zero(), 5);
        for i in 0..5 {
            assert!(zero_vec[i].is_zero())
        }

        let unit_vec = FieldElementVector::new_vandermonde_vector(&FieldElement::one(), 5);
        for i in 0..4 {
            assert!(unit_vec[i].is_one())
        }

        let two_vec = FieldElementVector::new_vandermonde_vector(&FieldElement::from(2u8), 10);
        let base = 2u32;
        for i in 0..10 {
            assert_eq!(two_vec[i], FieldElement::from(base.pow(i as u32) as u32));
        }
    }

    #[test]
    fn test_to_bitvectors() {
        let n = FieldElement::from(100u32);
        assert_eq!(n.to_bitvectors(), vec![vec![0, 0, 1, 0, 0, 1, 1]]);
        let mut c = vec![0i64; NLEN];
        c[0] = 2;
        c[1] = 100;
        let m: FieldElement = BigNum::new_ints(&c).into();
        assert_eq!(
            m.to_bitvectors(),
            vec![vec![0, 1], vec![0, 0, 1, 0, 0, 1, 1]]
        );
    }

    #[test]
    fn test_to_bits() {
        let mut bits = vec![0, 1, 0, 1];
        bits.append(&mut vec![0; BigNumBits - 4]);
        assert_eq!(FieldElement::from(10u32).to_bits(), bits);

        let mut bits = vec![0, 0, 1, 0, 0, 1, 1];
        bits.append(&mut vec![0; BigNumBits - 7]);
        assert_eq!(FieldElement::from(100u32).to_bits(), bits);

        let mut c = vec![0i64; NLEN];
        c[0] = 2;
        c[1] = 100;
        let m: FieldElement = BigNum::new_ints(&c).into();
        let mut bits = vec![0, 1];
        bits.append(&mut vec![0; BigNumBits - 2]);
        bits.append(&mut vec![0, 0, 1, 0, 0, 1, 1]);
        bits.append(&mut vec![0; BigNumBits - 7]);
        assert_eq!(m.to_bits(), bits);
    }

    #[test]
    fn test_negating_field_elems() {
        let b = FieldElement::random();
        let neg_b = -&b;
        assert_ne!(b, neg_b);
        let neg_neg_b = -&neg_b;
        assert_eq!(b, neg_neg_b);
        assert_eq!(b + neg_b, FieldElement::zero());
    }

    #[test]
    fn test_field_elem_addition() {
        let a = FieldElement::random();
        let b = FieldElement::random();
        let c = FieldElement::random();

        let sum = &a + &b + &c;

        let mut expected_sum = FieldElement::new();
        expected_sum = expected_sum.plus(&a);
        expected_sum = expected_sum.plus(&b);
        expected_sum += &c;
        assert_eq!(sum, expected_sum);
    }

    #[test]
    fn test_field_elem_subtraction() {
        let a = FieldElement::random();
        let b = FieldElement::random();
        let c = FieldElement::random();

        let sum = &a - &b - &c;

        let mut expected_sum = FieldElement::new();
        expected_sum = expected_sum.plus(&a);
        expected_sum = expected_sum - &b;
        expected_sum -= &c;
        assert_eq!(sum, expected_sum);
    }

    #[test]
    fn test_static_field_elems() {
        let zero = FieldElement::zero();
        let one = FieldElement::one();
        let minus_one = FieldElement::minus_one();
        assert_eq!(one + minus_one, zero);
    }

    #[test]
    fn test_field_elem_to_from_base() {
        for i in 0..4 {
            let x = FieldElement::from(i as u8);
            let b = x.to_power_of_2_base(2);
            assert_eq!(b, vec![i]);
            assert_eq!(x, FieldElement::from_power_of_2_base(&b, 2));
        }

        for i in 0..8 {
            let x = FieldElement::from(i as u8);
            let b = x.to_power_of_2_base(3);
            assert_eq!(b, vec![i]);
            assert_eq!(x, FieldElement::from_power_of_2_base(&b, 3));
        }

        for (n, expected_4) in vec![
            (4, vec![0, 1]),
            (5, vec![1, 1]),
            (6, vec![2, 1]),
            (7, vec![3, 1]),
            (8, vec![0, 2]),
            (63, vec![3, 3, 3]),
            (6719, vec![3, 3, 3, 0, 2, 2, 1]),
            (
                8911009812u64,
                vec![0, 1, 1, 0, 0, 2, 3, 0, 3, 0, 2, 0, 3, 0, 1, 0, 2],
            ),
        ] {
            let x = FieldElement::from(n as u64);
            let b = x.to_power_of_2_base(2);
            assert_eq!(b, expected_4);
            assert_eq!(x, FieldElement::from_power_of_2_base(&b, 2));
        }

        for (n, expected_8) in vec![
            (8, vec![0, 1]),
            (63, vec![7, 7]),
            (6719, vec![7, 7, 0, 5, 1]),
            (8911009812u64, vec![4, 2, 0, 4, 3, 6, 0, 1, 3, 2, 0, 1]),
        ] {
            let x = FieldElement::from(n as u64);
            let b = x.to_power_of_2_base(3);
            assert_eq!(b, expected_8);
            assert_eq!(x, FieldElement::from_power_of_2_base(&b, 3));
        }

        for _ in 0..100 {
            let x = FieldElement::random();
            for base in 2..8 {
                let digits = x.to_power_of_2_base(base);
                assert_eq!(x, FieldElement::from_power_of_2_base(&digits, base));
            }
        }
    }

    #[test]
    fn test_hashing() {
        // If the element can be added to HashSet or HashMap, it must be hashable.
        let mut set = HashSet::new();
        let mut map = HashMap::new();
        let x = FieldElement::random();
        set.insert(x.clone());
        let y = FieldElement::random();
        map.insert(y.clone(), FieldElement::random());
        assert!(set.contains(&x));
        assert!(map.contains_key(&y));
    }

    #[test]
    fn timing_multiplication_inversion() {
        // Timing multiplication and inversion
        let count = 100;
        let elems: Vec<_> = (0..count).map(|_| FieldElement::random()).collect();

        let mut res_mul = FieldElement::one();
        let mut start = Instant::now();
        for e in &elems {
            res_mul = res_mul * e;
        }
        println!(
            "Multiplication time for {} field elems = {:?}",
            count,
            start.elapsed()
        );

        start = Instant::now();
        let mut inverses = vec![];
        for e in &elems {
            inverses.push(e.inverse());
        }
        println!(
            "Inverse time for {} field elems = {:?}",
            count,
            start.elapsed()
        );

        start = Instant::now();
        let (inverses_1, all_inv) = FieldElement::batch_invert(&elems);
        println!(
            "Batch inverse time for {} field elems = {:?}",
            count,
            start.elapsed()
        );

        let mut expected_inv_product = FieldElement::one();
        for i in 0..count {
            assert_eq!(inverses[i], inverses_1[i]);
            expected_inv_product = expected_inv_product * &inverses[i];
        }

        assert_eq!(expected_inv_product, all_inv);
    }

    #[test]
    fn timing_field_elem_addition() {
        let count = 100;
        let points: Vec<FieldElement> = (0..count).map(|_| FieldElement::random()).collect();
        let mut R = FieldElement::random();
        let start = Instant::now();
        for i in 0..count {
            R = &R + &points[i];
        }
        println!(
            "Addition time for {} field elems = {:?}",
            count,
            start.elapsed()
        );
    }

    #[test]
    fn timing_field_elem_multiplication() {
        let count = 1000;
        let l: Vec<BigNum> = (0..count)
            .map(|_| FieldElement::random().to_bignum())
            .collect();
        let r: Vec<BigNum> = (0..count)
            .map(|_| FieldElement::random().to_bignum())
            .collect();
        let mut o1 = vec![];
        let mut o2 = vec![];

        let (k, u, v) = (*BarrettRedc_k, *BarrettRedc_u, *BarrettRedc_v);
        let mut start = Instant::now();
        for i in 0..count {
            let mut a = l[i].clone();
            a.rmod(&CurveOrder);
            let mut b = r[i].clone();
            b.rmod(&CurveOrder);
            let d = BigNum::mul(&a, &b);
            o1.push(barrett_reduction(&d, &CurveOrder, k, &u, &v));
        }
        println!("Mul1 for {} field elems = {:?}", count, start.elapsed());

        start = Instant::now();
        for i in 0..count {
            let a = l[i].clone();
            let b = r[i].clone();
            let d = BigNum::mul(&a, &b);
            o2.push(barrett_reduction(&d, &CurveOrder, k, &u, &v));
        }
        println!("Mul2 for {} field elems = {:?}", count, start.elapsed());

        for i in 0..count {
            assert_eq!(BigNum::comp(&o1[i], &o2[i]), 0);
        }

        let mut x = BigNum::new_int(1isize);
        start = Instant::now();
        for i in 0..count {
            x.rmod(&CurveOrder);
            let mut b = o1[i].clone();
            b.rmod(&CurveOrder);
            let d = BigNum::mul(&x, &b);
            x = barrett_reduction(&d, &CurveOrder, k, &u, &v);
        }
        println!("Mul1 all for {} field elems = {:?}", count, start.elapsed());

        let mut y = BigNum::new_int(1isize);
        start = Instant::now();
        for i in 0..count {
            let b = o2[i].clone();
            let d = BigNum::mul(&y, &b);
            y = barrett_reduction(&d, &CurveOrder, k, &u, &v);
        }
        println!("Mul2 all for {} field elems = {:?}", count, start.elapsed());

        assert_eq!(BigNum::comp(&x, &y), 0);
    }

    #[test]
    fn timing_field_elem_squaring() {
        let count = 1000;
        let fs: Vec<FieldElement> = (0..count).map(|_| FieldElement::random()).collect();
        let nums: Vec<BigNum> = fs.iter().map(|f| f.to_bignum()).collect();
        let mut r1 = vec![];
        let mut r2 = vec![];

        let start = Instant::now();
        for i in 0..count {
            r1.push(&fs[i] * &fs[i])
        }
        println!("Mul time for {} field elems = {:?}", count, start.elapsed());

        let start = Instant::now();
        for i in 0..count {
            r2.push(fs[i].square())
        }
        println!(
            "Square time for {} field elems = {:?}",
            count,
            start.elapsed()
        );
        for i in 0..count {
            assert_eq!(r1[i], r2[i])
        }

        let start = Instant::now();
        for i in 0..count {
            BigNum::modmul(&nums[i], &nums[i], &CurveOrder);
        }
        println!("Mul time for {} big nums = {:?}", count, start.elapsed());

        let start = Instant::now();
        for i in 0..count {
            BigNum::modsqr(&nums[i], &CurveOrder);
        }
        println!("Sqr time for {} big nums = {:?}", count, start.elapsed());
    }

    #[test]
    fn test_parse_hex_as_bignum() {
        for _ in 0..100 {
            let b = FieldElement::random().to_bignum();
            let h = b.clone().tostring();
            let b1 = FieldElement::parse_hex_as_bignum(h.clone()).unwrap();
            let b2 = BigNum::fromstring(h);
            // b == b2
            assert_eq!(BigNum::comp(&b, &b2), 0);
            // b == b1
            assert_eq!(BigNum::comp(&b, &b1), 0);
        }
    }

    #[test]
    fn test_parse_bad_hex_for_bignum() {
        let r1 = FieldElement::random();
        let mut h = r1.to_hex();
        // Make hex string bigger
        h.insert(0, '0');
        assert!(h.len() > FieldElement_SIZE * 2);
        assert!(FieldElement::parse_hex_as_bignum(h.clone()).is_err());

        let mut h = r1.to_hex();
        // Add non hex character
        h = h.replacen("0", "G", 1);
        assert_eq!(h.len(), FieldElement_SIZE * 2);
        assert!(FieldElement::parse_hex_as_bignum(h.clone()).is_err());
    }

    #[test]
    fn test_hex_field_elem() {
        for _ in 0..1000 {
            let r = FieldElement::random();
            let h = r.to_hex();
            let r_ = FieldElement::from_hex(h).unwrap();
            assert_eq!(r, r_);

            // Very unlikely that 2 randomly chosen elements will be equal
            let s = FieldElement::random();
            assert_ne!(r, s);
        }
    }

    #[test]
    fn test_serialization_deserialization_field_elem() {
        #[derive(Serialize, Deserialize)]
        struct Temp {
            val: FieldElement,
        }
        for _ in 0..100 {
            let r = FieldElement::random();
            let s = Temp { val: r.clone() };

            let serialized = serde_json::to_string(&s);
            assert!(serialized.is_ok());

            let j = serialized.unwrap();
            let f: Result<Temp, _> = serde_json::from_str(&j);
            assert!(f.is_ok());
            assert_eq!(f.unwrap().val, r)
        }
    }
}
