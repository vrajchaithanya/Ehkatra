//! Exact base-10 arithmetic for currency (ADR-010, ADR-035).
//!
//! # What this is, precisely
//! A signed 128-bit coefficient with a base-10 exponent: `value = coeff × 10^exp`.
//! That gives **38 significant decimal digits** — more than IEEE 754-2008
//! decimal128's 34 — with exact `+`, `−`, `×` and exact comparison, computed
//! entirely in integer arithmetic.
//!
//! # What this is NOT
//! It is *not* IEEE 754-2008 decimal128 and the code does not call it that
//! (ADR-035). There is no ±Infinity, no NaN, no signalling NaN, no
//! densely-packed-decimal bit layout, and no cohort/quantum preservation:
//! `1.50` and `1.5` are the same value here and normalise to one canonical
//! form. Overflow is not a trap — every operation returns `None` and the value
//! layer turns that into a `#NUM!` error, because errors are values and the
//! kernel never panics across a boundary (DP-A10).
//!
//! Losing the quantum is deliberate and consistent with the architecture: two
//! representations of one number would mean two canonical encodings and two
//! state hashes for the same content, which DP-A4 forbids. Trailing-zero
//! display is a number-format concern, and formatting never feeds back into
//! stored values (docs/04).
//!
//! # Why it is hand-built rather than bought
//! DP-B9 says buy boring and never outsource the op algebra or its encoding.
//! A stored decimal *is* part of the canonical op encoding, which DP-A4 freezes
//! for the life of the format, so its representation is core, not commodity.
//! The candidate crates also do not deliver what the name promises: the popular
//! fixed-point crates carry a 96-bit mantissa (~28 digits), fewer than both
//! this type and real decimal128. See ADR-035 for the full comparison.

use core::cmp::Ordering;

/// Largest power of ten that fits in `i128` (10^38 overflows).
const MAX_POW10: u32 = 38;

/// Fractional digits kept by `div` before rounding half-even. Chosen to leave
/// headroom under the 38-digit coefficient for a subsequent multiply.
pub const DIV_SCALE: i16 = 28;

/// `10^n` for `n <= 38`, or `None` if it would not fit in `i128`.
fn pow10(n: u32) -> Option<i128> {
    if n > MAX_POW10 {
        return None;
    }
    let mut acc: i128 = 1;
    for _ in 0..n {
        acc = acc.checked_mul(10)?;
    }
    Some(acc)
}

/// An exact base-10 number: `coeff × 10^exp`, always in canonical form.
///
/// Canonical form (the DP-A4 requirement — exactly one encoding per value):
/// trailing zeros are stripped from the coefficient, and zero is always
/// `{ coeff: 0, exp: 0 }`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Decimal {
    coeff: i128,
    exp: i16,
}

impl Decimal {
    pub const ZERO: Decimal = Decimal { coeff: 0, exp: 0 };
    pub const ONE: Decimal = Decimal { coeff: 1, exp: 0 };

    /// Builds a decimal from raw parts and canonicalises it.
    pub fn new(coeff: i128, exp: i16) -> Decimal {
        Decimal { coeff, exp }.normalized()
    }

    pub fn from_i64(n: i64) -> Decimal {
        Decimal::new(n as i128, 0)
    }

    pub fn coefficient(&self) -> i128 {
        self.coeff
    }

    pub fn exponent(&self) -> i16 {
        self.exp
    }

    pub fn is_zero(&self) -> bool {
        self.coeff == 0
    }

    pub fn is_negative(&self) -> bool {
        self.coeff < 0
    }

    /// Strips trailing zeros so equal values have identical representations.
    fn normalized(mut self) -> Decimal {
        if self.coeff == 0 {
            return Decimal::ZERO;
        }
        while self.coeff % 10 == 0 && self.exp < i16::MAX {
            self.coeff /= 10;
            self.exp += 1;
        }
        self
    }

    /// Rescales to exactly `target` exponent, or `None` if that would overflow
    /// or would discard non-zero digits.
    fn rescaled_to(&self, target: i16) -> Option<Decimal> {
        match self.exp.cmp(&target) {
            Ordering::Equal => Some(*self),
            Ordering::Greater => {
                let shift = (self.exp - target) as u32;
                let factor = pow10(shift)?;
                Some(Decimal {
                    coeff: self.coeff.checked_mul(factor)?,
                    exp: target,
                })
            }
            // Scaling up would drop digits; only exact when they are all zero.
            Ordering::Less => {
                let shift = (target - self.exp) as u32;
                let factor = pow10(shift)?;
                if self.coeff % factor != 0 {
                    return None;
                }
                Some(Decimal {
                    coeff: self.coeff / factor,
                    exp: target,
                })
            }
        }
    }

    /// Brings both operands to a common exponent without losing digits.
    fn align(a: &Decimal, b: &Decimal) -> Option<(i128, i128, i16)> {
        let target = a.exp.min(b.exp);
        Some((
            a.rescaled_to(target)?.coeff,
            b.rescaled_to(target)?.coeff,
            target,
        ))
    }

    /// Exact addition. `None` on coefficient overflow (→ `#NUM!`).
    pub fn add(&self, other: &Decimal) -> Option<Decimal> {
        let (x, y, exp) = Decimal::align(self, other)?;
        Some(Decimal::new(x.checked_add(y)?, exp))
    }

    /// Exact subtraction. `None` on coefficient overflow.
    pub fn sub(&self, other: &Decimal) -> Option<Decimal> {
        let (x, y, exp) = Decimal::align(self, other)?;
        Some(Decimal::new(x.checked_sub(y)?, exp))
    }

    /// Exact multiplication. `None` on coefficient or exponent overflow.
    pub fn mul(&self, other: &Decimal) -> Option<Decimal> {
        let coeff = self.coeff.checked_mul(other.coeff)?;
        let exp = self.exp.checked_add(other.exp)?;
        Some(Decimal::new(coeff, exp))
    }

    pub fn neg(&self) -> Option<Decimal> {
        Some(Decimal::new(self.coeff.checked_neg()?, self.exp))
    }

    /// Division, exact when it terminates within [`DIV_SCALE`] fractional
    /// digits and rounded half-even otherwise.
    ///
    /// Returns `None` for division by zero — the value layer turns that into
    /// `#DIV/0!` — and for overflow.
    ///
    /// Half-even ("banker's rounding") is the deliberate choice over Excel's
    /// half-away-from-zero: repeated half-up rounding accumulates an upward
    /// bias, which is exactly the phantom-penny drift this type exists to kill.
    /// The `compat` profile is where Excel's rounding lives (docs/32), and it
    /// operates on `Number`, not here.
    pub fn div(&self, other: &Decimal) -> Option<Decimal> {
        if other.is_zero() {
            return None;
        }
        if self.is_zero() {
            return Some(Decimal::ZERO);
        }
        // Scale the dividend up so the quotient carries DIV_SCALE extra digits,
        // then round the final digit half-even.
        let lift = pow10(DIV_SCALE as u32 + 1)?;
        let numerator = self.coeff.checked_mul(lift)?;
        let raw = numerator / other.coeff;
        let exp = self
            .exp
            .checked_sub(other.exp)?
            .checked_sub(DIV_SCALE + 1)?;

        let last = (raw % 10).abs();
        let mut quotient = raw / 10;
        let exact = numerator % other.coeff == 0;
        let round_up = match last.cmp(&5) {
            Ordering::Greater => true,
            Ordering::Less => false,
            // Exactly half only if nothing was truncated below; otherwise the
            // true remainder is above half.
            Ordering::Equal => !exact || quotient % 2 != 0,
        };
        if round_up {
            quotient = quotient.checked_add(if raw < 0 { -1 } else { 1 })?;
        }
        Some(Decimal::new(quotient, exp.checked_add(1)?))
    }

    /// Total order over decimals. Exact: never routes through `f64`.
    pub fn compare(&self, other: &Decimal) -> Ordering {
        // Sign decides immediately and cannot overflow.
        let (sa, sb) = (self.coeff.signum(), other.coeff.signum());
        if sa != sb {
            return sa.cmp(&sb);
        }
        if let Some((x, y, _)) = Decimal::align(self, other) {
            return x.cmp(&y);
        }
        // Alignment overflowed, so the operands differ hugely in magnitude:
        // compare by decimal magnitude (digit count + exponent) instead.
        let mag = |d: &Decimal| digits(d.coeff) as i32 + d.exp as i32;
        match mag(self).cmp(&mag(other)) {
            Ordering::Equal => Ordering::Equal,
            // Larger magnitude wins, flipped for negatives.
            other_ord if sa < 0 => other_ord.reverse(),
            other_ord => other_ord,
        }
    }

    /// Converts to `f64` for the `Number` domain. Lossy by nature — this is the
    /// direction that gives up exactness, which is why promotion prefers the
    /// other way (see `Value::promote`).
    pub fn to_f64(&self) -> f64 {
        let mut v = self.coeff as f64;
        let mut e = self.exp;
        while e > 0 {
            v *= 10.0;
            e -= 1;
        }
        while e < 0 {
            v /= 10.0;
            e += 1;
        }
        v
    }

    /// Exact conversion from `f64`, or `None` when the float's *true* value
    /// cannot be represented in 38 decimal digits.
    ///
    /// This is what makes mixed `Number`/`Decimal` arithmetic honest: it
    /// promotes to `Decimal` only when doing so loses nothing, and falls back
    /// to `f64` otherwise (archive/DOC-GRID-DESIGN §V.1).
    ///
    /// The conversion works on the float's bits, never on float arithmetic.
    /// That distinction is the whole point. A float is `m × 2^e` for integers
    /// `m` and `e`; when `e` is negative that is `m × 5^|e| × 10^-|e|`, which is
    /// exact iff `m × 5^|e|` fits the coefficient. Scaling by ten in `f64`
    /// instead would round at every step and cheerfully report that `0.1_f64`
    /// converts exactly — it does not. `0.1_f64` is really
    /// `0.1000000000000000055511151231257827…`, needing 55 decimal digits, so
    /// this returns `None` for it, and `Value` arithmetic correctly stays on
    /// the `f64` path rather than manufacturing precision.
    pub fn try_from_f64_exact(v: f64) -> Option<Decimal> {
        if !v.is_finite() {
            return None;
        }
        if v == 0.0 {
            return Some(Decimal::ZERO);
        }
        let bits = v.to_bits();
        let negative = bits >> 63 == 1;
        let raw_exp = ((bits >> 52) & 0x7FF) as i32;
        let frac = bits & 0x000F_FFFF_FFFF_FFFF;
        // Subnormals have no implicit leading 1 and a fixed exponent.
        let (mut mantissa, mut exp2) = if raw_exp == 0 {
            (frac as i128, -1074i32)
        } else {
            ((frac | 0x0010_0000_0000_0000) as i128, raw_exp - 1075)
        };
        // Strip trailing binary zeros: without this, 0.5 arrives as
        // 2^52 × 2^-53 and the 5^53 factor overflows a value that is plainly
        // representable.
        while mantissa != 0 && mantissa % 2 == 0 {
            mantissa /= 2;
            exp2 += 1;
        }

        let decimal = if exp2 >= 0 {
            let factor = pow2(exp2 as u32)?;
            Decimal::new(mantissa.checked_mul(factor)?, 0)
        } else {
            let shift = (-exp2) as u32;
            let factor = pow5(shift)?;
            let exp10 = i16::try_from(-exp2).ok()?.checked_neg()?;
            Decimal::new(mantissa.checked_mul(factor)?, exp10)
        };
        if negative {
            decimal.neg()
        } else {
            Some(decimal)
        }
    }
}

/// `2^n` as `i128`, or `None` if it does not fit.
fn pow2(n: u32) -> Option<i128> {
    if n >= 127 {
        return None;
    }
    Some(1i128 << n)
}

/// `5^n` as `i128`, or `None` if it does not fit.
fn pow5(n: u32) -> Option<i128> {
    let mut acc: i128 = 1;
    for _ in 0..n {
        acc = acc.checked_mul(5)?;
    }
    Some(acc)
}

/// Number of decimal digits in `n`'s magnitude (0 has one digit).
fn digits(mut n: i128) -> u32 {
    if n == 0 {
        return 1;
    }
    let mut d = 0;
    while n != 0 {
        n /= 10;
        d += 1;
    }
    d
}

/// Plain decimal notation, never scientific — the canonical *text* form.
///
/// Because the value is normalised, this prints the shortest exact spelling:
/// `1.50` and `1.5` are one value and both print as `1.5`. Trailing zeros are a
/// number-format decision, applied by the display layer on top of this.
impl core::fmt::Display for Decimal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.coeff == 0 {
            return f.write_str("0");
        }
        if self.coeff < 0 {
            f.write_str("-")?;
        }
        let mag = self.coeff.unsigned_abs();
        if self.exp >= 0 {
            write!(f, "{mag}")?;
            for _ in 0..self.exp {
                f.write_str("0")?;
            }
            return Ok(());
        }
        let frac_digits = (-self.exp) as u32;
        let divisor = match pow10(frac_digits) {
            Some(d) => d as u128,
            // Beyond 38 fractional digits the value cannot exist: the
            // coefficient would have had to overflow to get here.
            None => return f.write_str("0"),
        };
        let int_part = mag / divisor;
        let frac_part = mag % divisor;
        write!(f, "{int_part}.")?;
        // Left-pad the fraction so 1.05 does not print as 1.5.
        let mut leading = frac_digits.saturating_sub(digits(frac_part as i128));
        while leading > 0 {
            f.write_str("0")?;
            leading -= 1;
        }
        write!(f, "{frac_part}")
    }
}

impl Ord for Decimal {
    fn cmp(&self, other: &Decimal) -> Ordering {
        self.compare(other)
    }
}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Decimal) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Parses a plain decimal literal: optional sign, digits, optional fraction.
///
/// Deliberately does **not** accept scientific notation, `Inf`, or `NaN`.
/// Exponent-form text is one of Excel's data-mangling vectors (`1E2` silently
/// becoming `100`), so admitting it here would smuggle the very coercion the
/// `strict` profile exists to refuse. `Profile` decides whether text becomes a
/// number at all; this function only decides whether the digits are well formed.
pub fn parse_decimal(s: &str) -> Option<Decimal> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut i = 0;
    let negative = match bytes[0] {
        b'-' => {
            i = 1;
            true
        }
        b'+' => {
            i = 1;
            false
        }
        _ => false,
    };

    let mut coeff: i128 = 0;
    let mut exp: i16 = 0;
    let mut seen_digit = false;
    let mut seen_point = false;

    while i < bytes.len() {
        match bytes[i] {
            b'0'..=b'9' => {
                seen_digit = true;
                coeff = coeff
                    .checked_mul(10)?
                    .checked_add((bytes[i] - b'0') as i128)?;
                if seen_point {
                    exp = exp.checked_sub(1)?;
                }
            }
            b'.' if !seen_point => seen_point = true,
            _ => return None,
        }
        i += 1;
    }

    if !seen_digit {
        return None;
    }
    Some(Decimal::new(if negative { -coeff } else { coeff }, exp))
}
