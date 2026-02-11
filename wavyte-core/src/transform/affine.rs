//! Affine transform helpers.

use crate::foundation::core::Affine;

#[inline]
pub fn compose(a: Affine, b: Affine) -> Affine {
    a * b
}

#[inline]
pub fn identity() -> Affine {
    Affine::IDENTITY
}
