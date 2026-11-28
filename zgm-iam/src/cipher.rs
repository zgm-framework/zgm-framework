//! The cipher the application gives the module.

use std::error::Error;

use async_trait::async_trait;

/// Encrypts and decrypts what the module must be able to show but keeps readable nowhere,
/// such as the name of a tenant. The application gives an implementation to the builder of
/// [`Iam`](crate::Iam); the module holds no key and knows nothing of its versions.
///
/// Both methods get only the bytes, not what they are or whose they are: the module uses the
/// same cipher for whatever it encrypts. An implementation that can bind a ciphertext to a
/// place (a column, a tenant, a row) therefore binds everything to one fixed place, and
/// bytes copied to another row decrypt there too.
///
/// A ciphertext is only ever decrypted, never compared, so an implementation may encrypt the
/// same data to different bytes each time, and may tell from the bytes which version of its
/// key made them.
///
/// The methods are `async` so that an implementation can fetch its key from somewhere else,
/// e.g. a key store over the network. One that holds its key in memory simply awaits nothing.
#[async_trait]
pub trait Cipher: Send + Sync + 'static {
    /// The ciphertext of `data`. Never empty, because the module takes empty bytes for a
    /// broken cipher ([`IamError::EmptyCiphertext`](crate::IamError)).
    ///
    /// An error, e.g. the key could not be fetched, is the error of the operation: nothing
    /// is written and the caller gets [`IamError::Cipher`](crate::IamError).
    async fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>, CipherError>;

    /// `data` back from a ciphertext [`encrypt`](Self::encrypt) made, with whichever version
    /// of the key it was made.
    async fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, CipherError>;
}

/// Why a [`Cipher`] could not encrypt or decrypt. It wraps the implementation's own error.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct CipherError(Box<dyn Error + Send + Sync>);

impl CipherError {
    /// Wraps the error of an implementation.
    pub fn new(error: impl Into<Box<dyn Error + Send + Sync>>) -> Self {
        Self(error.into())
    }
}
