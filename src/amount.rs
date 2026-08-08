//! Exact decimal quantities: macro masses in grams and serving counts.
//!
//! Values are rounded to [`SCALE`] decimal places (0.001 g) at construction,
//! division, and serialization, and written to log files as bare decimal
//! literals (integers when integral, floats otherwise) — no quotes, and
//! exact round-trip for values below ≈2.2×10^12 at full 0.001 precision
//! (~15 significant digits), which covers any realistic diet quantity.
//! Values that cannot round-trip exactly through the `f64` representation —
//! and integral values too large for a TOML integer — are rejected at
//! serialization instead of being written lossily; strings are rejected on
//! read. In-memory sums and products stay exact.
//!
//! [`Grams`] and [`Calories`] are non-negative; [`Servings`] is strictly
//! positive. Arithmetic on these types is checked: sums and products return
//! `None` on overflow instead of panicking.

use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use rust_decimal::{Decimal, RoundingStrategy};
use serde::de::Error as DeError;
use serde::ser::Error as SerError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

/// Storage precision: 0.001 g (1 mg), 0.001 kcal, and 0.001 servings.
const SCALE: u32 = 3;

/// Round a raw decimal at midpoints, away from zero (matches `f64::round`).
pub fn round_away(value: Decimal, places: u32) -> Decimal {
    value.round_dp_with_strategy(places, RoundingStrategy::MidpointAwayFromZero)
}

macro_rules! decimal_type {
    ($t:ident, $expecting:literal, $valid_f64:expr, $valid_decimal:expr) => {
        #[derive(Debug, Clone, Copy)]
        pub struct $t(Decimal);

        impl $t {
            /// Build from an exact decimal, rounding to storage precision.
            /// Errors if the value (or its rounded form) is out of range.
            pub fn from_decimal(value: Decimal) -> Result<Self, String> {
                let value = if value.is_zero() {
                    Decimal::ZERO
                } else {
                    value
                };
                if !$valid_decimal(value) {
                    return Err(format!("{value} is not a valid {}", stringify!($t)));
                }
                let rounded = value.round_dp(SCALE);
                if !$valid_decimal(rounded) {
                    return Err(format!(
                        "{value} rounds to {rounded}, not a valid {}",
                        stringify!($t)
                    ));
                }
                Ok($t(rounded))
            }

            /// Build from a float (e.g. bare TOML float literals), rounding to
            /// storage precision. Errors on NaN, infinity, or out-of-range.
            pub fn from_f64(value: f64) -> Result<Self, String> {
                if value.is_nan() || value.is_infinite() || !$valid_f64(value) {
                    return Err(format!("{value} is not a valid {}", stringify!($t)));
                }
                let dec = Decimal::from_f64(value)
                    .ok_or_else(|| format!("cannot convert {value} to decimal"))?;
                Self::from_decimal(dec)
            }

            /// Build from an integer (unchecked).
            pub fn from_u32(value: u32) -> Self {
                $t(Decimal::from(value))
            }

            /// The exact decimal value (may have more than [`SCALE`] places).
            pub fn to_decimal(self) -> Decimal {
                self.0
            }

            pub fn is_integer(self) -> bool {
                self.0.fract().is_zero()
            }

            /// Round to `places` decimal places (half to even).
            pub fn round_dp(self, places: u32) -> Self {
                $t(self.0.round_dp(places))
            }

            /// Round half away from zero (matches `f64::round`).
            pub fn round_dp_away(self, places: u32) -> Self {
                $t(round_away(self.0, places))
            }

            /// Round to storage precision.
            pub fn rounded(self) -> Self {
                $t(self.0.round_dp(SCALE))
            }

            /// Add, returning `None` on overflow instead of panicking.
            pub fn checked_add(self, rhs: Self) -> Option<Self> {
                self.0.checked_add(rhs.0).map($t)
            }

            /// Multiply by a decimal, returning `None` on overflow instead
            /// of panicking.
            pub fn checked_mul(self, rhs: Decimal) -> Option<Self> {
                self.0.checked_mul(rhs).map($t)
            }

            /// Divide by a decimal, rounding to storage precision. Returns
            /// `None` for a zero divisor or on overflow.
            pub fn checked_div(self, rhs: Decimal) -> Option<Self> {
                self.0.checked_div(rhs).map(|q| $t(q.round_dp(SCALE)))
            }
        }

        impl FromStr for $t {
            type Err = String;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let dec =
                    Decimal::from_str(s).map_err(|e| format!("invalid decimal '{s}': {e}"))?;
                Self::from_decimal(dec)
            }
        }

        impl TryFrom<Decimal> for $t {
            type Error = String;

            fn try_from(value: Decimal) -> Result<Self, Self::Error> {
                Self::from_decimal(value)
            }
        }

        impl From<$t> for Decimal {
            fn from(value: $t) -> Decimal {
                value.0
            }
        }

        impl fmt::Display for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0.normalize())
            }
        }

        impl PartialEq for $t {
            fn eq(&self, other: &Self) -> bool {
                self.0.normalize() == other.0.normalize()
            }
        }

        impl Eq for $t {}

        impl PartialOrd for $t {
            fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }

        impl Ord for $t {
            fn cmp(&self, other: &Self) -> std::cmp::Ordering {
                self.0.normalize().cmp(&other.0.normalize())
            }
        }

        impl Serialize for $t {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let value = self.0.round_dp(SCALE).normalize();
                if value.fract().is_zero() {
                    if let Some(i) = value.to_i64() {
                        return serializer.serialize_i64(i);
                    }
                    return Err(S::Error::custom(format!(
                        "{value} exceeds the TOML integer range"
                    )));
                }
                let f = value.to_f64().expect("decimal always fits f64");
                if Decimal::from_f64(f).map(|d| d.round_dp(SCALE).normalize()) != Some(value) {
                    return Err(S::Error::custom(format!(
                        "{value} cannot be serialized exactly as a TOML float"
                    )));
                }
                serializer.serialize_f64(f)
            }
        }

        impl<'de> Deserialize<'de> for $t {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                struct Visitor;
                impl<'de> serde::de::Visitor<'de> for Visitor {
                    type Value = $t;

                    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                        f.write_str($expecting)
                    }

                    fn visit_i64<E: DeError>(self, v: i64) -> Result<Self::Value, E> {
                        Self::Value::from_decimal(Decimal::from(v)).map_err(E::custom)
                    }

                    fn visit_u64<E: DeError>(self, v: u64) -> Result<Self::Value, E> {
                        Self::Value::from_decimal(Decimal::from(v)).map_err(E::custom)
                    }

                    fn visit_f64<E: DeError>(self, v: f64) -> Result<Self::Value, E> {
                        Self::Value::from_f64(v).map_err(E::custom)
                    }
                }
                deserializer.deserialize_any(Visitor)
            }
        }
    };
}

decimal_type!(
    Grams,
    "a non-negative decimal number",
    |v: f64| v >= 0.0,
    |d: Decimal| !d.is_sign_negative()
);

decimal_type!(
    Servings,
    "a positive decimal number",
    |v: f64| v > 0.0,
    |d: Decimal| !d.is_sign_negative() && !d.is_zero()
);

decimal_type!(
    Calories,
    "a non-negative decimal number",
    |v: f64| v >= 0.0,
    |d: Decimal| !d.is_sign_negative()
);

impl Grams {
    /// Zero grams.
    pub const ZERO: Grams = Grams(Decimal::ZERO);
}

impl Calories {
    /// Zero calories.
    pub const ZERO: Calories = Calories(Decimal::ZERO);
}

/// The six macros: calories plus five masses in grams.
#[derive(Debug, Clone, Copy)]
pub struct Macros {
    pub calories: Calories,
    pub protein_g: Grams,
    pub fiber_g: Grams,
    pub fat_g: Grams,
    pub carbs_g: Grams,
    pub alcohol_g: Grams,
}

impl Macros {
    /// Zero macros.
    pub const ZERO: Macros = Macros {
        calories: Calories::ZERO,
        protein_g: Grams::ZERO,
        fiber_g: Grams::ZERO,
        fat_g: Grams::ZERO,
        carbs_g: Grams::ZERO,
        alcohol_g: Grams::ZERO,
    };

    /// Add another macros set element-wise; `None` on overflow.
    pub fn checked_add(&self, other: &Macros) -> Option<Macros> {
        Some(Macros {
            calories: self.calories.checked_add(other.calories)?,
            protein_g: self.protein_g.checked_add(other.protein_g)?,
            fiber_g: self.fiber_g.checked_add(other.fiber_g)?,
            fat_g: self.fat_g.checked_add(other.fat_g)?,
            carbs_g: self.carbs_g.checked_add(other.carbs_g)?,
            alcohol_g: self.alcohol_g.checked_add(other.alcohol_g)?,
        })
    }
}

/// Sum grams with overflow check; `None` if any partial sum overflows.
pub(crate) fn grams_sum(iter: impl IntoIterator<Item = Grams>) -> Option<Grams> {
    iter.into_iter()
        .try_fold(Grams::ZERO, |acc, v| acc.checked_add(v))
}

/// Sum calories with overflow check; `None` if any partial sum overflows.
pub(crate) fn calories_sum(iter: impl IntoIterator<Item = Calories>) -> Option<Calories> {
    iter.into_iter()
        .try_fold(Calories::ZERO, |acc, v| acc.checked_add(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(s: &str) -> Grams {
        Grams::from_str(s).unwrap()
    }

    fn servings(s: &str) -> Servings {
        Servings::from_str(s).unwrap()
    }

    fn calories(s: &str) -> Calories {
        Calories::from_str(s).unwrap()
    }

    #[test]
    fn test_macros_checked_add() {
        let a = Macros {
            calories: calories("100.0"),
            protein_g: g("10.0"),
            fiber_g: g("5.0"),
            fat_g: g("4.0"),
            carbs_g: g("30.0"),
            alcohol_g: g("2.0"),
        };
        let b = Macros::ZERO;
        let sum = a.checked_add(&b).unwrap();
        assert_eq!(sum.calories, calories("100.0"));
        assert_eq!(sum.protein_g, g("10.0"));
        assert_eq!(a.checked_add(&a).unwrap().fat_g, g("8.0"));
    }

    #[test]
    fn test_macros_checked_add_overflow() {
        let a = Macros {
            calories: Calories::from_decimal(Decimal::MAX).unwrap(),
            protein_g: Grams::ZERO,
            fiber_g: Grams::ZERO,
            fat_g: Grams::ZERO,
            carbs_g: Grams::ZERO,
            alcohol_g: Grams::ZERO,
        };
        assert!(a.checked_add(&a).is_none());
    }

    #[test]
    fn test_equality_is_scale_independent() {
        assert_eq!(g("3.5"), Grams::from_f64(3.5).unwrap());
        assert_eq!(g("3.500"), g("3.5"));
        assert_eq!(g("3.333"), Grams::from_f64(3.3333333333333335).unwrap());
    }

    #[test]
    fn test_ordering() {
        assert!(g("9.999") < g("10"));
        assert!(g("10") > g("9.999"));
        assert_eq!(g("10").cmp(&g("10.000")), std::cmp::Ordering::Equal);
    }

    #[test]
    fn test_negative_grams_rejected() {
        assert!(Grams::from_str("-0.1").is_err());
        assert!(Grams::from_f64(-1.0).is_err());
        assert!(Grams::try_from(Decimal::new(-1, 0)).is_err());
    }

    #[test]
    fn test_nan_inf_rejected() {
        assert!(Grams::from_f64(f64::NAN).is_err());
        assert!(Grams::from_f64(f64::INFINITY).is_err());
    }

    #[test]
    fn test_negative_zero_normalized() {
        assert_eq!(Grams::from_f64(-0.0).unwrap(), g("0"));
        assert!(Servings::from_f64(-0.0).is_err());
    }

    #[test]
    fn test_servings_must_be_positive() {
        assert!(Servings::from_str("0").is_err());
        assert!(Servings::from_str("-1").is_err());
        assert!(Servings::from_f64(0.0).is_err());
        assert_eq!(servings("1.5"), Servings::from_f64(1.5).unwrap());
    }

    #[test]
    fn test_values_rounding_to_zero_rejected_for_servings() {
        assert!(Servings::from_str("0.0004").is_err());
        assert!(Servings::from_f64(0.0004).is_err());
        assert_eq!(Grams::from_str("0.0004").unwrap(), g("0"));
    }

    #[test]
    fn test_calories_reject_negative_and_round() {
        assert!(Calories::from_str("-0.1").is_err());
        assert!(Calories::from_f64(-1.0).is_err());
        assert!(Calories::try_from(Decimal::new(-1, 0)).is_err());
        assert_eq!(Calories::from_u32(250), calories("250"));
        assert_eq!(Calories::from_f64(1.23456).unwrap(), calories("1.235"));
        assert_eq!(Calories::ZERO, calories("0"));
    }

    #[test]
    fn test_rounds_to_storage_precision() {
        let value = Grams::from_f64(1.23456).unwrap();
        assert_eq!(value, g("1.235"));
        assert_eq!(value.to_string(), "1.235");
    }

    #[test]
    fn test_division_rounds_to_storage_precision() {
        assert_eq!(
            g("10").checked_div(Decimal::from(3u32)).unwrap(),
            g("3.333")
        );
        assert_eq!(g("5").checked_div(Decimal::from(3u32)).unwrap(), g("1.667"));
    }

    #[test]
    fn test_checked_div() {
        assert_eq!(g("10").checked_div(Decimal::from(3u32)), Some(g("3.333")));
        assert_eq!(g("1").checked_div(Decimal::ZERO), None);
    }

    #[test]
    fn test_checked_add_mul_overflow_returns_none() {
        let max = Grams::from_decimal(Decimal::MAX).unwrap();
        assert_eq!(max.checked_add(g("1")), None);
        assert_eq!(g("1").checked_add(g("2")), Some(g("3")));
        assert_eq!(max.checked_mul(Decimal::from(2u32)), None);
        assert_eq!(g("3.5").checked_mul(Decimal::from(2u32)), Some(g("7")));
    }

    #[test]
    fn test_checked_sums() {
        assert_eq!(grams_sum([g("1"), g("2"), g("3")]), Some(g("6")));
        assert_eq!(
            calories_sum([calories("1"), calories("2")]),
            Some(calories("3"))
        );
        let max = Grams::from_decimal(Decimal::MAX).unwrap();
        assert_eq!(grams_sum([max, g("1")]), None);
    }

    #[test]
    fn test_addition_is_exact_decimal() {
        let sum = g("0.1").checked_add(g("0.2")).unwrap();
        assert_eq!(sum, g("0.3"));
        assert_eq!(sum.to_string(), "0.3");
    }

    #[test]
    fn test_multiplication_is_exact() {
        assert_eq!(g("3.5").checked_mul(Decimal::from(2u32)).unwrap(), g("7"));
        assert_eq!(
            g("3.5")
                .checked_mul(Decimal::from_str("1.5").unwrap())
                .unwrap(),
            g("5.25")
        );
    }

    #[test]
    fn test_is_integer() {
        assert!(g("3").is_integer());
        assert!(!g("3.5").is_integer());
        assert!(servings("2").is_integer());
        assert!(!servings("1.5").is_integer());
    }

    #[test]
    fn test_round_dp_away() {
        assert_eq!(g("107.5").round_dp_away(0), g("108"));
        assert_eq!(g("106.5").round_dp_away(0), g("107"));
        assert_eq!(g("106.5").round_dp(0), g("106"));
    }

    #[derive(Serialize, Deserialize)]
    struct Wrap {
        value: Grams,
    }

    #[derive(Serialize, Deserialize)]
    struct WrapServings {
        value: Servings,
    }

    #[derive(Serialize, Deserialize)]
    struct WrapCalories {
        value: Calories,
    }

    #[test]
    fn test_toml_round_trip() {
        for value in [
            "0",
            "0.3",
            "3.333",
            "9.999",
            "12.5",
            "100",
            "3.333333",
            "999999999999.999",
        ] {
            let wrap = Wrap {
                value: Grams::from_str(value).unwrap(),
            };
            let serialized = toml::to_string(&wrap).unwrap();
            let deserialized: Wrap = toml::from_str(&serialized).unwrap();
            assert_eq!(
                deserialized.value, wrap.value,
                "round trip failed for {value}"
            );
        }
        for value in ["0.5", "1", "3.333", "9.999", "12.5", "100"] {
            let wrap = WrapServings {
                value: Servings::from_str(value).unwrap(),
            };
            let serialized = toml::to_string(&wrap).unwrap();
            let deserialized: WrapServings = toml::from_str(&serialized).unwrap();
            assert_eq!(
                deserialized.value, wrap.value,
                "round trip failed for {value}"
            );
        }
        for value in ["0", "0.333", "250", "999.999", "999999999999.999"] {
            let wrap = WrapCalories {
                value: Calories::from_str(value).unwrap(),
            };
            let serialized = toml::to_string(&wrap).unwrap();
            let deserialized: WrapCalories = toml::from_str(&serialized).unwrap();
            assert_eq!(
                deserialized.value, wrap.value,
                "round trip failed for {value}"
            );
        }
    }

    #[test]
    fn test_toml_accepts_float_and_int() {
        assert_eq!(
            toml::from_str::<Wrap>("value = 3.3").unwrap().value,
            g("3.3")
        );
        assert_eq!(toml::from_str::<Wrap>("value = 7").unwrap().value, g("7"));
        assert!(toml::from_str::<Wrap>("value = -1").is_err());
        assert!(toml::from_str::<Wrap>("value = -1.5").is_err());
        assert!(toml::from_str::<WrapServings>("value = 0").is_err());
        assert!(toml::from_str::<WrapCalories>("value = -5").is_err());
        assert_eq!(
            toml::from_str::<WrapCalories>("value = 250").unwrap().value,
            calories("250")
        );
        assert_eq!(
            toml::from_str::<WrapCalories>("value = 250.5")
                .unwrap()
                .value,
            calories("250.5")
        );
    }

    #[test]
    fn test_toml_rejects_quoted_strings() {
        assert!(toml::from_str::<Wrap>("value = \"2.5\"").is_err());
        assert!(toml::from_str::<WrapCalories>("value = \"250.5\"").is_err());
    }

    #[test]
    fn test_toml_serialize_rejects_out_of_range_integer() {
        let wrap = Wrap {
            value: Grams::from_decimal(Decimal::MAX).unwrap(),
        };
        let err = toml::to_string(&wrap).unwrap_err().to_string();
        assert!(err.contains("TOML integer range"), "got: {err}");
    }

    #[test]
    fn test_toml_serialize_rejects_large_integrals() {
        // 2^70 is exactly representable in f64 but exceeds the i64 TOML
        // integer range; reject rather than write scientific notation.
        let wrap = Wrap {
            value: g("1180591620717411303424"),
        };
        let err = toml::to_string(&wrap).unwrap_err().to_string();
        assert!(err.contains("TOML integer"), "got: {err}");
    }

    #[test]
    fn test_toml_serializes_as_bare_numbers() {
        let wrap = Wrap { value: g("9.99") };
        assert_eq!(toml::to_string(&wrap).unwrap().trim(), "value = 9.99");
        let wrap = Wrap { value: Grams::ZERO };
        assert_eq!(toml::to_string(&wrap).unwrap().trim(), "value = 0");
        let wrap = WrapServings {
            value: servings("1.5"),
        };
        assert_eq!(toml::to_string(&wrap).unwrap().trim(), "value = 1.5");
    }

    #[test]
    fn test_toml_serializes_integral_as_int() {
        let wrap = Wrap { value: g("3") };
        assert_eq!(toml::to_string(&wrap).unwrap().trim(), "value = 3");
        let wrap = Wrap { value: g("2.5") };
        assert_eq!(toml::to_string(&wrap).unwrap().trim(), "value = 2.5");
        let wrap = Wrap { value: g("3.00") };
        assert_eq!(toml::to_string(&wrap).unwrap().trim(), "value = 3");
    }
}
