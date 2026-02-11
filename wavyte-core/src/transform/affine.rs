//! Affine transform helpers.

use crate::foundation::core::Affine;

#[inline]
/// Compose two affine transforms using matrix multiplication order `a * b`.
pub fn compose(a: Affine, b: Affine) -> Affine {
    a * b
}

#[inline]
/// Return affine identity transform.
pub fn identity() -> Affine {
    Affine::IDENTITY
}
