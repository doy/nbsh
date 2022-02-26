pub use crate::env::Env;

pub use std::io::{Read as _, Write as _};

pub use futures_util::future::FutureExt as _;
pub use futures_util::stream::StreamExt as _;
pub use futures_util::stream::TryStreamExt as _;
pub use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

pub use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
pub use std::os::unix::io::{AsRawFd as _, FromRawFd as _, IntoRawFd as _};
pub use std::os::unix::process::ExitStatusExt as _;
pub use users::os::unix::UserExt as _;

pub use ext::Result as _;

mod ext {
    pub trait Result {
        type T;
        type E;

        fn allow(self, allow_e: Self::E) -> Self;
        fn allow_with(self, allow_e: Self::E, default_t: Self::T) -> Self;
    }

    impl<T, E> Result for std::result::Result<T, E>
    where
        T: std::default::Default,
        E: std::cmp::PartialEq,
    {
        type T = T;
        type E = E;

        fn allow(self, allow_e: Self::E) -> Self {
            self.or_else(|e| {
                if e == allow_e {
                    Ok(std::default::Default::default())
                } else {
                    Err(e)
                }
            })
        }

        fn allow_with(self, allow_e: Self::E, default_t: Self::T) -> Self {
            self.or_else(
                |e| if e == allow_e { Ok(default_t) } else { Err(e) },
            )
        }
    }
}
