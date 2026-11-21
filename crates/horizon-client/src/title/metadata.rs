//! Cancel metadata-dependent screen operations through their existing error path.
macro_rules! metadata {
    (log $result:expr) => { metadata!(log $result, $crate::title::Action::None) };
    (log $result:expr, ()) => {
        match $result {
            Ok(value) => value,
            Err(error) => {
                log::error!("Horizon: metadata context: {error}");
                return;
            }
        }
    };
    (log $result:expr, $exit:expr) => {
        match $result {
            Ok(value) => value,
            Err(error) => {
                log::error!("Horizon: metadata context: {error}");
                return $exit;
            }
        }
    };
    ($target:expr, $result:expr) => { metadata!($target, $result, $crate::title::Action::None) };
    ($target:expr, $result:expr, ()) => {
        match $result {
            Ok(value) => value,
            Err(error) => {
                $target.failed(error);
                return;
            }
        }
    };
    ($target:expr, $result:expr, $exit:expr) => {
        match $result {
            Ok(value) => value,
            Err(error) => {
                $target.failed(error);
                return $exit;
            }
        }
    };
}
pub(super) use metadata;

pub(super) fn error_mode(error: impl std::fmt::Display, return_to: super::Mode) -> super::Mode {
    super::Mode::Error {
        message: error.to_string(),
        return_to: Box::new(return_to),
        alert: None,
    }
}
