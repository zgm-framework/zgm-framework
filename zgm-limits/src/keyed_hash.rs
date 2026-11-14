//! The hash of the hidden part of a key.

use std::error::Error;

use async_trait::async_trait;
use uuid::Uuid;

/// A hash with a secret key, e.g. HMAC-SHA-256. The application gives the module an
/// implementation, and the module calls it for every key with a hidden part (an e-mail, an IP
/// address), so that the hidden part lies in the database only as a hash.
///
/// **The hash must use a secret key.** A plain hash such as SHA-256 compiles too, but then a
/// copy of the database lets anybody guess the IP addresses in minutes.
///
/// **A hash is at most [`MAX_HASH_BYTES`] long.** Every keyed hash is far shorter. An
/// implementation that returned its data unchanged would put text of any length into the
/// primary key of the table, where PostgreSQL refuses a long entry, and the attempt would
/// never be counted.
///
/// The module holds no key and knows no version of it; the implementation does. The method is
/// `async`, so an implementation may fetch its key from somewhere; one that has the key in
/// memory simply awaits nothing.
#[async_trait]
pub trait KeyedHash: Send + Sync + 'static {
    /// One hash of `data` for every version of the secret key that can still be read, the
    /// current version first. Never an empty list, never an empty hash, and never a hash
    /// longer than [`MAX_HASH_BYTES`].
    ///
    /// The module looks a counter up by all of them, so after a change of the key the counter
    /// written with the older version is still found, and writes a new counter with the first.
    ///
    /// An error, e.g. the key could not be fetched, is the error of the attempt: the attempt
    /// is not counted and the caller gets [`LimitsError::KeyedHash`](crate::LimitsError).
    async fn hashes(&self, data: &[u8]) -> Result<Vec<Vec<u8>>, KeyedHashError>;
}

/// The longest hash the module takes from a [`KeyedHash`]. HMAC-SHA-512 gives 64 bytes, and a
/// header with the version of the key a few more, so this leaves room; it is far below what
/// an entry of the table's primary key may hold.
pub const MAX_HASH_BYTES: usize = 128;

/// Why a [`KeyedHash`] could not compute a hash. It wraps the implementation's own error.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct KeyedHashError(Box<dyn Error + Send + Sync>);

impl KeyedHashError {
    /// Wraps the error of an implementation.
    pub fn new(error: impl Into<Box<dyn Error + Send + Sync>>) -> Self {
        Self(error.into())
    }
}

/// What the hash of a key is computed from: the limit name, the tenant, the account and the
/// hidden part together. So the same e-mail gives another hash in every limit and every
/// tenant, and a row copied under another limit or tenant matches nothing.
///
/// Every piece goes with its length in front, so that no two different keys give the same
/// bytes: without the lengths, "ab" + "c" and "a" + "bc" would.
pub(crate) fn key_data(
    limit_name: &str,
    tenant_id: Uuid,
    subject_id: Uuid,
    hidden: &str,
) -> Vec<u8> {
    let pieces: [&[u8]; 4] = [
        limit_name.as_bytes(),
        tenant_id.as_bytes(),
        subject_id.as_bytes(),
        hidden.as_bytes(),
    ];
    let mut data = Vec::new();
    for piece in pieces {
        data.extend_from_slice(&(piece.len() as u64).to_be_bytes());
        data.extend_from_slice(piece);
    }
    data
}

/// What the hash of an element of a window of distinct elements is computed from: the limit
/// name, the tenant and the element, behind a label of its own. The label keeps an element
/// apart from the hidden part of a key with the same text.
pub(crate) fn element_data(limit_name: &str, tenant_id: Uuid, element: &str) -> Vec<u8> {
    let pieces: [&[u8]; 4] = [
        b"element",
        limit_name.as_bytes(),
        tenant_id.as_bytes(),
        element.as_bytes(),
    ];
    let mut data = Vec::new();
    for piece in pieces {
        data.extend_from_slice(&(piece.len() as u64).to_be_bytes());
        data.extend_from_slice(piece);
    }
    data
}

/// The text of a hidden part made uniform: letters in lower case, and no white space and no
/// invisible characters anywhere, so that writing the same e-mail another way gives the same
/// key.
pub(crate) fn made_uniform(text: &str) -> String {
    text.chars()
        .filter(|&character| !character.is_whitespace() && !is_invisible(character))
        .flat_map(char::to_lowercase)
        .collect()
}

/// Whether a character takes no room on the screen: a control character, or one of the
/// formatting characters of Unicode such as the zero-width space. Pasted into the middle of an
/// e-mail they leave it looking the same, so left in they would give one address any number of
/// counters.
///
/// It is a list of the blocks where such characters lie, not the whole of Unicode's "format"
/// category, which only a table of Unicode has.
fn is_invisible(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{00AD}'                    // soft hyphen
            | '\u{034F}'                  // combining grapheme joiner
            | '\u{061C}'                  // Arabic letter mark
            | '\u{115F}' | '\u{1160}'     // Hangul fillers
            | '\u{17B4}' | '\u{17B5}'     // Khmer inherent vowels
            | '\u{180B}'..='\u{180F}'     // Mongolian variation selectors and separator
            | '\u{200B}'..='\u{200F}'     // zero-width space, joiners, direction marks
            | '\u{202A}'..='\u{202E}'     // direction embeddings and overrides
            | '\u{2060}'..='\u{206F}'     // word joiner, invisible operators, isolates
            | '\u{3164}'                  // Hangul filler
            | '\u{FE00}'..='\u{FE0F}'     // variation selectors
            | '\u{FEFF}'                  // byte order mark
            | '\u{FFA0}'                  // halfwidth Hangul filler
            | '\u{FFF9}'..='\u{FFFB}'     // interlinear annotation marks
            | '\u{1D173}'..='\u{1D17A}'   // musical formatting
            | '\u{E0000}'..='\u{E0FFF}'   // tags and more variation selectors
        )
}
