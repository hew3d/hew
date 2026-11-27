//! The kernel's transcendental math: one implementation on every target.
//!
//! IEEE 754 rounds `+ - * /` and `sqrt` exactly, so those give the same bits
//! everywhere. It does not do the same for `sin`, `cos`, `atan2` and the
//! rest: each C math library rounds them its own way, and std's `f64::sin`
//! calls the platform's library on a native build (glibc on Linux, Apple's
//! on macOS, the CRT on Windows) but pure-Rust code on wasm32. The results
//! differ in the last bit on a few percent of inputs, which is enough to move
//! a vertex and so a document's state hash. The kernel is required to be
//! bit-for-bit deterministic across platforms (docs/dev/DEVELOPMENT.md, "The
//! determinism golden"), so every transcendental the kernel crates compute
//! goes through this module instead, and this module calls the pure-Rust
//! `libm` crate, pinned to an exact version, on every target, wasm included.
//!
//! Clippy's `disallowed-methods` (clippy.toml) bans the std methods in the
//! kernel crates, so a new call site cannot bypass this module by accident.
//!
//! A `libm` upgrade can change results by an ulp, which moves geometry. The
//! pinned bit patterns in this module's tests are there to catch such a
//! change, so a bump is a deliberate, reviewed change that regenerates the
//! determinism golden and the replay fixtures with it.
//!
//! `libm`'s default `arch` feature stays on: on the targets Hew builds for it
//! replaces only operations IEEE 754 rounds exactly (`sqrt`, `fma`,
//! `floor`, `ceil`, `trunc`, `rint`, `fabs`) with hardware instructions, so
//! it cannot change a result.

/// Sine of `x` (radians).
#[inline]
pub fn sin(x: f64) -> f64 {
    libm::sin(x)
}

/// Cosine of `x` (radians).
#[inline]
pub fn cos(x: f64) -> f64 {
    libm::cos(x)
}

/// Tangent of `x` (radians).
#[inline]
pub fn tan(x: f64) -> f64 {
    libm::tan(x)
}

/// Arcsine of `x`, in radians in `[-π/2, π/2]`; NaN outside `[-1, 1]`.
#[inline]
pub fn asin(x: f64) -> f64 {
    libm::asin(x)
}

/// Arccosine of `x`, in radians in `[0, π]`; NaN outside `[-1, 1]`.
#[inline]
pub fn acos(x: f64) -> f64 {
    libm::acos(x)
}

/// Arctangent of `x`, in radians in `[-π/2, π/2]`.
#[inline]
pub fn atan(x: f64) -> f64 {
    libm::atan(x)
}

/// The angle of the point `(x, y)` from the positive x axis, in radians in
/// `[-π, π]`. The argument order is `y` first, as in `y.atan2(x)` and C's
/// `atan2(y, x)`.
#[inline]
pub fn atan2(y: f64, x: f64) -> f64 {
    libm::atan2(y, x)
}

/// `(sin(x), cos(x))`. Computed as the two separate calls, exactly as std's
/// `f64::sin_cos` is, rather than through `libm::sincos`, whose shared
/// argument reduction is a different computation.
#[inline]
pub fn sin_cos(x: f64) -> (f64, f64) {
    (sin(x), cos(x))
}

/// `x` raised to the real power `y`. An integer power that is exactly
/// representable as an `i32` should stay `x.powi(n)`, which is a sequence of
/// multiplications and so already gives the same bits everywhere.
#[inline]
pub fn powf(x: f64, y: f64) -> f64 {
    libm::pow(x, y)
}

/// The length of the 2D vector `(x, y)`: `sqrt(x² + y²)`, rounded exactly as
/// written, so every platform gives the same bits.
///
/// This is not a replacement for a library `hypot`, which scales its
/// arguments to avoid overflow and underflow in the squares and rounds more
/// carefully, and which differs between platforms in the last bit. Every
/// caller passes model-scale values (lengths in meters, or ratios of them,
/// far from `1e±150`), where the squares can neither overflow nor
/// underflow.
#[inline]
pub fn hypot(x: f64, y: f64) -> f64 {
    (x * x + y * y).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_1_SQRT_2, FRAC_PI_2, FRAC_PI_3, FRAC_PI_4, FRAC_PI_6, PI, TAU};

    // Each function's exact output bits for fixed inputs, as the pinned
    // `libm` computes them. A `libm` bump that rounds differently, a build
    // that reaches a different implementation, or a change to this module
    // fails here, before it can move geometry. The inputs cover zero of both
    // signs, small and large arguments, the angles the kernel builds most
    // (fractions of π and of a full turn), and, after them, inputs on which
    // glibc rounds differently from `libm`, so a call that fell through to
    // the platform library would fail here too on Linux.
    const SIN: &[(f64, u64)] = &[
        (0.0, 0x0000000000000000),
        (-0.0, 0x8000000000000000),
        (1e-9, 0x3e112e0be826d695),
        (0.1, 0x3fb98eaecb8bcb2c),
        (0.5, 0x3fdeaee8744b05f0),
        (1.0, 0x3feaed548f090cee),
        (FRAC_PI_6, 0x3fe0000000000000),
        (FRAC_PI_4, 0x3fe6a09e667f3bcc),
        (FRAC_PI_3, 0x3febb67ae8584cab),
        (FRAC_PI_2, 0x3ff0000000000000),
        (2.0, 0x3fed18f6ead1b446),
        (3.0, 0x3fc210386db6d55b),
        (PI, 0x3ca1a62633145c07),
        (-2.5, 0xbfe326af0dcfcab0),
        (7.0 * FRAC_PI_4, 0xbfe6a09e667f3bce),
        (TAU / 6.0, 0x3febb67ae8584caa),
        (TAU * 5.0 / 64.0, 0x3fde2b5d3806f63b),
        (TAU * 37.0 / 256.0, 0x3fe93a22499263fb),
        (10.0, 0xbfe1689ef5f34f52),
        (1e6, 0xbfd6664b2568d867),
        (-1.9726, 0xbfed7390c27b736b),
        (-1.91095, 0xbfee2aa055857e84),
        (-1.36295, 0xbfef4fb041d8730c),
        (-1.97671, 0xbfed6655a45f237b),
        (-1.87533, 0xbfee870f803c9d1d),
        (-1.4944700000000002, 0xbfefe826446ce67a),
        (-1.99726, 0xbfed2247047b84c0),
        (-1.98356, 0xbfed5000260c11fc),
        (-1.95616, 0xbfeda736a0a19351),
    ];

    const COS: &[(f64, u64)] = &[
        (0.0, 0x3ff0000000000000),
        (-0.0, 0x3ff0000000000000),
        (1e-9, 0x3ff0000000000000),
        (0.1, 0x3fefd712f9a817c1),
        (0.5, 0x3fec1528065b7d50),
        (1.0, 0x3fe14a280fb5068c),
        (FRAC_PI_6, 0x3febb67ae8584caa),
        (FRAC_PI_4, 0x3fe6a09e667f3bcd),
        (FRAC_PI_3, 0x3fdffffffffffffe),
        (FRAC_PI_2, 0x3c91a62633145c07),
        (2.0, 0xbfdaa22657537205),
        (3.0, 0xbfefae04be85e5d2),
        (PI, 0xbff0000000000000),
        (-2.5, 0xbfe9a2f7ef858b7d),
        (7.0 * FRAC_PI_4, 0x3fe6a09e667f3bcb),
        (TAU / 6.0, 0x3fe0000000000001),
        (TAU * 5.0 / 64.0, 0x3fec38b2f180bdb1),
        (TAU * 37.0 / 256.0, 0x3fe3affa292050b9),
        (10.0, 0xbfead9ac890c6b1f),
        (1e6, 0x3fedf9df9906d32c),
        (-1.9726, 0xbfd9077037dedcf0),
        (-1.91095, 0xbfd55a39e1eda271),
        (-1.36295, 0x3fca69c6ec05da5c),
        (-1.97671, 0xbfd9455bfa584bc2),
        (-1.87533, 0xbfd330b715902850),
        (-1.4944700000000002, 0x3fb3854449bb20b3),
        (-1.99726, 0xbfda794dcc7dbfeb),
        (-1.98356, 0xbfd9ac5146d1caef),
        (-1.95616, 0xbfd80eaec86776d4),
    ];

    const TAN: &[(f64, u64)] = &[
        (0.0, 0x0000000000000000),
        (-0.0, 0x8000000000000000),
        (1e-9, 0x3e112e0be826d695),
        (0.1, 0x3fb9af8877430b80),
        (0.5, 0x3fe17b4f5bf3474a),
        (1.0, 0x3ff8eb245cbee3a6),
        (FRAC_PI_6, 0x3fe279a74590331d),
        (FRAC_PI_4, 0x3fefffffffffffff),
        (FRAC_PI_3, 0x3ffbb67ae8584cac),
        (FRAC_PI_2, 0x434d02967c31cdb5),
        (2.0, 0xc0017af62e0950f8),
        (3.0, 0xbfc23ef71254b86f),
        (PI, 0xbca1a62633145c07),
        (-2.5, 0x3fe7e79b4e00bb15),
        (7.0 * FRAC_PI_4, 0xbff0000000000002),
        (TAU / 6.0, 0x3ffbb67ae8584ca8),
        (TAU * 5.0 / 64.0, 0x3fe11ab7190834eb),
        (TAU * 37.0 / 256.0, 0x3ff48089f8bf42cc),
        (10.0, 0x3fe4bf5f34be3782),
        (1e6, 0xbfd7e9768ab734c0),
        (-1.9726, 0x4002d3b860059fa1),
        (-1.91095, 0x40069ac90f334ad1),
        (-1.36295, 0xc012f79119f61796),
        (-1.97671, 0x40029d362eec7426),
        (-1.87533, 0x400973d82f801a24),
        (-1.4944700000000002, 0xc02a270204828689),
        (-1.99726, 0x40019b8f5f1dd0a2),
        (-1.98356, 0x400244a4c1a1aaa6),
        (-1.95616, 0x4003b8be5bf96913),
    ];

    const ASIN: &[(f64, u64)] = &[
        (-1.0, 0xbff921fb54442d18),
        (-0.999999, 0xbff91c306b2c13ad),
        (-0.5, 0xbfe0c152382d7366),
        (0.0, 0x0000000000000000),
        (0.1, 0x3fb9a49276037884),
        (0.5, 0x3fe0c152382d7366),
        (FRAC_1_SQRT_2, 0x3fe921fb54442d19),
        (0.9, 0x3ff1ea93705fa172),
        (0.99999999, 0x3ff9216709c28b31),
        (1.0, 0x3ff921fb54442d18),
        (-0.9825085, 0xbff622c15d68f98e),
        (-0.9725135, 0xbff55f682e2caf4c),
        (-0.9710142500000001, 0xbff54560fec2bf10),
        (-0.9900047500000001, 0xbff6de5fbefdb43b),
        (-0.95952, 0xbff4908f54525541),
        (-0.9550222500000001, 0xbff450d57190c992),
    ];

    const ACOS: &[(f64, u64)] = &[
        (-1.0, 0x400921fb54442d18),
        (-0.999999, 0x40091f15dfb82062),
        (-0.5, 0x4000c152382d7366),
        (0.0, 0x3ff921fb54442d18),
        (0.1, 0x3ff787b22ce3f590),
        (0.5, 0x3ff0c152382d7366),
        (FRAC_1_SQRT_2, 0x3fe921fb54442d18),
        (0.9, 0x3fdcdd9f8f922e98),
        (0.99999999, 0x3f228950343cef55),
        (1.0, 0x0000000000000000),
        (-0.9825085, 0x4007a25e58d69353),
        (-0.9725135, 0x400740b1c1386e32),
        (-0.9710142500000001, 0x400733ae29837614),
        (-0.9900047500000001, 0x4008002d89a0f0aa),
        (-0.95952, 0x4006d945544b412c),
        (-0.9550222500000001, 0x4006b96862ea7b56),
    ];

    const ATAN: &[(f64, u64)] = &[
        (-1e3, 0xbff91de2c0e658bd),
        (-1.0, 0xbfe921fb54442d18),
        (-0.3, 0xbfd2a73a661eaf06),
        (0.0, 0x0000000000000000),
        (1e-8, 0x3e45798ee2308c3a),
        (0.5, 0x3fddac670561bb4f),
        (1.0, 0x3fe921fb54442d18),
        (2.5, 0x3ff30b6d796a4da8),
        (1e8, 0x3ff921fb5194fb3c),
        (-1.98767, 0xbff1acbaf9788012),
        (-1.94657, 0xbff18a283dade360),
        (-1.85889, 0xbff13c67bf28ea70),
    ];

    const ATAN2: &[(f64, f64, u64)] = &[
        (1.0, 1.0, 0x3fe921fb54442d18),
        (-1.0, 1.0, 0xbfe921fb54442d18),
        (1.0, -1.0, 0x4002d97c7f3321d2),
        (-1.0, -1.0, 0xc002d97c7f3321d2),
        (0.0, -1.0, 0x400921fb54442d18),
        (-0.0, -1.0, 0xc00921fb54442d18),
        (1e-9, 1.0, 0x3e112e0be826d695),
        (3.0, 4.0, 0x3fe4978fa3269ee1),
        (0.2, -0.05, 0x3ffd0d6a1369bd34),
        (-0.013, 0.021, 0xbfe1bce31303a035),
        (-1.99726, -0.9985005, 0xc000466f589bea34),
        (-1.98904, -0.9955020000000001, 0xc0004759938fbda6),
        (-1.9863, -0.9945025000000001, 0xc00047a80f80c128),
    ];

    const HYPOT: &[(f64, f64, u64)] = &[
        (1.0, 1.0, 0x3ff6a09e667f3bcd),
        (-1.0, 1.0, 0x3ff6a09e667f3bcd),
        (1.0, -1.0, 0x3ff6a09e667f3bcd),
        (-1.0, -1.0, 0x3ff6a09e667f3bcd),
        (0.0, -1.0, 0x3ff0000000000000),
        (-0.0, -1.0, 0x3ff0000000000000),
        (1e-9, 1.0, 0x3ff0000000000000),
        (3.0, 4.0, 0x4014000000000000),
        (0.2, -0.05, 0x3fca634bd77fe1a6),
        (-0.013, 0.021, 0x3f994a7aac556602),
        (-1.99726, -0.9985005, 0x4001dd13228a23a5),
        (-1.98904, -0.9955020000000001, 0x4001cb458107aa77),
        (-1.9863, -0.9945025000000001, 0x4001c5565880f267),
    ];

    const POWF: &[(f64, f64, u64)] = &[
        (10.0, 0.025, 0x3ff0f2b408bcd790),
        (10.0, -3.5, 0x3f34b96be9c2da2c),
        (300.0, 0.37, 0x402080c97d5b63dc),
        (2.0, 0.5, 0x3ff6a09e667f3bcd),
        (0.3, 2.4, 0x3fac77e018c0c76f),
        (1.91506, -0.9685155000000001, 0x3fe10e15196f92c4),
        (1.85889, -0.94802575, 0x3fe1c73f83d97f98),
        (1.84656, -0.943528, 0x3fe1f0b499cfaffc),
    ];

    fn check_unary(name: &str, f: fn(f64) -> f64, table: &[(f64, u64)]) {
        let wrong: Vec<String> = table
            .iter()
            .filter(|(x, bits)| f(*x).to_bits() != *bits)
            .map(|(x, bits)| {
                format!(
                    "{name}({x:e}) = {:#018x}, pinned {bits:#018x}",
                    f(*x).to_bits()
                )
            })
            .collect();
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    fn check_binary(name: &str, f: fn(f64, f64) -> f64, table: &[(f64, f64, u64)]) {
        let wrong: Vec<String> = table
            .iter()
            .filter(|(a, b, bits)| f(*a, *b).to_bits() != *bits)
            .map(|(a, b, bits)| {
                format!(
                    "{name}({a:e}, {b:e}) = {:#018x}, pinned {bits:#018x}",
                    f(*a, *b).to_bits()
                )
            })
            .collect();
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[test]
    fn every_function_gives_its_pinned_bits() {
        check_unary("sin", sin, SIN);
        check_unary("cos", cos, COS);
        check_unary("tan", tan, TAN);
        check_unary("asin", asin, ASIN);
        check_unary("acos", acos, ACOS);
        check_unary("atan", atan, ATAN);
        check_binary("atan2", atan2, ATAN2);
        check_binary("hypot", hypot, HYPOT);
        check_binary("powf", powf, POWF);
    }

    #[test]
    fn sin_cos_is_the_two_separate_calls() {
        for (x, _) in SIN {
            let (s, c) = sin_cos(*x);
            assert_eq!(
                (s.to_bits(), c.to_bits()),
                (sin(*x).to_bits(), cos(*x).to_bits()),
                "{x:e}"
            );
        }
    }
}
