// Copyright (c) 2020 lumi <lumi@pew.im>
// Copyright (c) 2020 Emmanuel Gil Peyrot <linkmauve@linkmauve.fr>
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! A module which exports a few traits for converting types to elements and attributes.

/// A trait for types which can be converted to an attribute value.
pub trait IntoAttributeValue {
    /// Turns this into an attribute string, or None if it shouldn't be added.
    fn into_attribute_value(self) -> Option<String>;
}

macro_rules! impl_into_attribute_value {
    ($t:ty) => {
        impl IntoAttributeValue for $t {
            fn into_attribute_value(self) -> Option<String> {
                Some(format!("{}", self))
            }
        }
    };
}

macro_rules! impl_into_attribute_values {
    ($($t:ty),*) => {
        $(impl_into_attribute_value!($t);)*
    }
}

impl_into_attribute_values!(
    usize,
    u64,
    u32,
    u16,
    u8,
    isize,
    i64,
    i32,
    i16,
    i8,
    ::std::net::IpAddr
);

impl IntoAttributeValue for String {
    fn into_attribute_value(self) -> Option<String> {
        Some(self)
    }
}

impl<'a> IntoAttributeValue for &'a String {
    fn into_attribute_value(self) -> Option<String> {
        Some(self.to_owned())
    }
}

impl<'a> IntoAttributeValue for &'a str {
    fn into_attribute_value(self) -> Option<String> {
        Some(self.to_owned())
    }
}

impl<T: IntoAttributeValue> IntoAttributeValue for Option<T> {
    fn into_attribute_value(self) -> Option<String> {
        self.and_then(IntoAttributeValue::into_attribute_value)
    }
}

#[cfg(test)]
mod tests {
    use super::IntoAttributeValue;
    use std::net::IpAddr;
    use std::str::FromStr;

    #[test]
    fn test_into_attribute_value_on_ints() {
        assert_eq!(16u8.into_attribute_value().unwrap(), "16");
        assert_eq!(17u16.into_attribute_value().unwrap(), "17");
        assert_eq!(18u32.into_attribute_value().unwrap(), "18");
        assert_eq!(19u64.into_attribute_value().unwrap(), "19");
        assert_eq!(16i8.into_attribute_value().unwrap(), "16");
        assert_eq!((-17i16).into_attribute_value().unwrap(), "-17");
        assert_eq!(18i32.into_attribute_value().unwrap(), "18");
        assert_eq!((-19i64).into_attribute_value().unwrap(), "-19");
        assert_eq!(
            IpAddr::from_str("0000:0::1")
                .unwrap()
                .into_attribute_value()
                .unwrap(),
            "::1"
        );
    }
}
