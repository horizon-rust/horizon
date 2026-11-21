//! Named service keys: the name rule a mod's published service follows. A service is a trait
//! object one mod registers under a namespaced name for other mods to look up, the way
//! `CapabilityKey` names a positional capability. This first piece is the key and its name
//! rule only; no host method reaches a registry yet (the design issue decides the rest).
use std::{fmt, marker::PhantomData};

/// The longest service name, in bytes.
pub const MAX_SERVICE_NAME_LEN: usize = 128;

// no wire: process-local key; it names a trait object the game never serializes.
/// A service's name (`mymod:economy`) and the trait `T` it is answered with.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ServiceKey<T: ?Sized + 'static> {
    name: &'static str,
    handler: PhantomData<fn() -> Box<T>>,
}

impl<T: ?Sized + 'static> ServiceKey<T> {
    /// A service key with this name. The name is checked when the key is used, not here.
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            handler: PhantomData,
        }
    }

    /// The name.
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Whether the name follows the rule: see [`check_service_name`].
    pub fn validate(&self) -> Result<(), ServiceNameError> {
        check_service_name(self.name)
    }
}

/// Why a service name is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceNameError {
    /// Longer than [`MAX_SERVICE_NAME_LEN`] bytes.
    TooLong,
    /// Not `namespace:path`: no colon, or an empty namespace or path.
    NotNamespaced,
    /// A character outside the allowed set, or a second colon.
    BadCharacter,
}

impl fmt::Display for ServiceNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLong => write!(f, "a service name is at most {MAX_SERVICE_NAME_LEN} bytes"),
            Self::NotNamespaced => write!(f, "a service name is namespace:path, both non-empty"),
            Self::BadCharacter => write!(
                f,
                "a service name uses a-z, 0-9, _ . - in the namespace and a-z, 0-9, _ . - / in the path"
            ),
        }
    }
}

/// Checks a service name: at most [`MAX_SERVICE_NAME_LEN`] bytes, one colon, a non-empty
/// namespace of `a-z 0-9 _ . -` and a non-empty path of those plus `/`.
pub fn check_service_name(name: &str) -> Result<(), ServiceNameError> {
    if name.len() > MAX_SERVICE_NAME_LEN {
        return Err(ServiceNameError::TooLong);
    }
    let Some((namespace, path)) = name.split_once(':') else {
        return Err(ServiceNameError::NotNamespaced);
    };
    if namespace.is_empty() || path.is_empty() {
        return Err(ServiceNameError::NotNamespaced);
    }
    let namespace_ok = namespace
        .bytes()
        .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-'));
    let path_ok = path
        .bytes()
        .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'/'));
    if namespace_ok && path_ok {
        Ok(())
    } else {
        Err(ServiceNameError::BadCharacter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    trait Economy {}

    #[test]
    fn a_namespaced_name_is_accepted() {
        assert_eq!(check_service_name("mymod:economy"), Ok(()));
        assert_eq!(check_service_name("corestage:npc/guard_2"), Ok(()));
        assert_eq!(check_service_name("a.b-c_d:e"), Ok(()));
    }

    #[test]
    fn a_name_without_a_namespace_is_refused() {
        assert_eq!(
            check_service_name("economy"),
            Err(ServiceNameError::NotNamespaced)
        );
        assert_eq!(
            check_service_name(":economy"),
            Err(ServiceNameError::NotNamespaced)
        );
        assert_eq!(
            check_service_name("mymod:"),
            Err(ServiceNameError::NotNamespaced)
        );
        assert_eq!(check_service_name(""), Err(ServiceNameError::NotNamespaced));
    }

    #[test]
    fn upper_case_spaces_and_a_second_colon_are_refused() {
        assert_eq!(
            check_service_name("MyMod:economy"),
            Err(ServiceNameError::BadCharacter)
        );
        assert_eq!(
            check_service_name("mymod:eco nomy"),
            Err(ServiceNameError::BadCharacter)
        );
        assert_eq!(
            check_service_name("mymod:a:b"),
            Err(ServiceNameError::BadCharacter)
        );
        assert_eq!(
            check_service_name("my/mod:economy"),
            Err(ServiceNameError::BadCharacter)
        );
    }

    #[test]
    fn the_length_limit_is_literal() {
        let at_limit = format!("m:{}", "a".repeat(MAX_SERVICE_NAME_LEN - 2));
        assert_eq!(at_limit.len(), 128);
        assert_eq!(check_service_name(&at_limit), Ok(()));
        let over = format!("m:{}", "a".repeat(MAX_SERVICE_NAME_LEN - 1));
        assert_eq!(over.len(), 129);
        assert_eq!(check_service_name(&over), Err(ServiceNameError::TooLong));
    }

    #[test]
    fn a_key_keeps_its_name_and_validates_it() {
        let key = ServiceKey::<dyn Economy>::new("mymod:economy");
        assert_eq!(key.name(), "mymod:economy");
        assert_eq!(key.validate(), Ok(()));
        let bad = ServiceKey::<dyn Economy>::new("economy");
        assert_eq!(bad.validate(), Err(ServiceNameError::NotNamespaced));
    }

    #[test]
    fn the_messages_name_the_rule() {
        assert_eq!(
            ServiceNameError::NotNamespaced.to_string(),
            "a service name is namespace:path, both non-empty"
        );
    }
}
