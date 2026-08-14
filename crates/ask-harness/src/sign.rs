//! This module signs an EIP-712 digest with a secp256k1 key.
//!
//! ## Secret handling
//!
//! The key comes from an environment variable and stays in memory. It
//! is NEVER written to disk, never put into the response cache, and
//! never printed. The only value this module prints is the ADDRESS,
//! which is public.
//!
//! The `Signer` type does not implement `Debug` or `Display` on
//! purpose. A derived `Debug` is the usual way a key reaches a log
//! line.
//!
//! ## Why the signature needs a recovery id
//!
//! An Ethereum signature is 65 bytes: r, s, and v. `v` is the recovery
//! id plus 27. The token contract calls `ecrecover`, which needs `v`
//! to rebuild the public key from r and s. A signature with the wrong
//! `v` recovers a different address, and the contract then rejects it
//! as a signature from the wrong signer.
//!
//! ## Low-s
//!
//! The `k256` crate normalises `s` to the lower half of the curve
//! order. Ethereum requires that. A high-s signature is malleable, and
//! most token contracts reject it.

use k256::ecdsa::signature::hazmat::PrehashSigner;
use k256::ecdsa::{RecoveryId, Signature, SigningKey};

use crate::eip712::keccak;

/// A secp256k1 signing key and its address.
///
/// This type has no `Debug` and no `Display`. See the module doc
/// comment for the reason.
pub struct Signer {
    key: SigningKey,
    address: [u8; 20],
}

impl Signer {
    /// This function builds a signer from a hex private key.
    ///
    /// The function accepts an optional `0x` prefix. It returns an
    /// error text that NEVER holds any part of the key.
    pub fn from_hex(text: &str) -> Result<Self, String> {
        let cleaned = text.trim();
        let cleaned = cleaned.strip_prefix("0x").unwrap_or(cleaned);
        if cleaned.len() != 64 {
            return Err(format!(
                "a private key must be 64 hex characters, but this one has {}",
                cleaned.len()
            ));
        }
        let mut bytes = [0u8; 32];
        for (index, pair) in cleaned.as_bytes().chunks(2).enumerate() {
            let hex = core::str::from_utf8(pair)
                .map_err(|_| "the private key is not valid text".to_string())?;
            bytes[index] = u8::from_str_radix(hex, 16)
                .map_err(|_| "the private key is not valid hex".to_string())?;
        }
        let key = SigningKey::from_bytes(&bytes.into())
            .map_err(|_| "the private key is not a valid secp256k1 key".to_string())?;
        let address = address_of(&key);
        Ok(Signer { key, address })
    }

    /// This function gives the public address of this signer.
    pub fn address(&self) -> [u8; 20] {
        self.address
    }

    /// This function signs a 32 byte digest.
    ///
    /// The function returns the 65 byte Ethereum signature: r, then s,
    /// then v. `v` is the recovery id plus 27.
    pub fn sign_digest(&self, digest: &[u8; 32]) -> Result<[u8; 65], String> {
        let (signature, recovery): (Signature, RecoveryId) = self
            .key
            .sign_prehash(digest)
            .map_err(|error| format!("the signature failed: {error}"))?;

        let mut output = [0u8; 65];
        output[..32].copy_from_slice(&signature.r().to_bytes());
        output[32..64].copy_from_slice(&signature.s().to_bytes());
        output[64] = recovery.to_byte() + 27;
        Ok(output)
    }
}

/// This function gives the Ethereum address of a signing key.
///
/// The address is the last 20 bytes of the Keccak-256 of the
/// uncompressed public key, with its `0x04` prefix byte removed.
fn address_of(key: &SigningKey) -> [u8; 20] {
    let point = key.verifying_key().to_encoded_point(false);
    let bytes = point.as_bytes();
    // The first byte is the 0x04 uncompressed marker. Skip it.
    let hash = keccak(&bytes[1..]);
    let mut address = [0u8; 20];
    address.copy_from_slice(&hash[12..32]);
    address
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eip712::to_hex;

    /// This is a well known TEST key. It holds nothing and it must
    /// never be funded. It appears in many test suites, so its address
    /// is easy to check against another implementation.
    const TEST_KEY: &str = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";

    #[test]
    fn the_address_matches_a_known_test_vector() {
        let signer = Signer::from_hex(TEST_KEY).expect("the test key must parse");
        assert_eq!(
            to_hex(&signer.address()),
            "0x2c7536e3605d9c16a7a3d7b1898e529396a65c23"
        );
    }

    #[test]
    fn a_key_without_the_prefix_also_parses() {
        let bare = TEST_KEY.trim_start_matches("0x");
        let signer = Signer::from_hex(bare).expect("a bare key must parse");
        assert_eq!(
            to_hex(&signer.address()),
            "0x2c7536e3605d9c16a7a3d7b1898e529396a65c23"
        );
    }

    #[test]
    fn a_wrong_length_key_is_an_error() {
        let outcome = Signer::from_hex("0x1234");
        assert!(outcome.is_err());
    }

    #[test]
    fn an_error_message_never_holds_the_key() {
        // A key of the right length but invalid hex must not appear in
        // the message.
        let secret = "zz".repeat(32);
        let outcome = Signer::from_hex(&secret);
        match outcome {
            Ok(_) => panic!("invalid hex must not parse"),
            Err(message) => {
                assert!(
                    !message.contains("zz"),
                    "the error text leaked key material: {message}"
                );
            }
        }
    }

    #[test]
    fn a_signature_is_65_bytes_with_a_valid_v() {
        let signer = Signer::from_hex(TEST_KEY).expect("the test key must parse");
        let digest = keccak(b"a test digest");
        let signature = signer.sign_digest(&digest).expect("the signing must work");
        assert_eq!(signature.len(), 65);
        assert!(
            signature[64] == 27 || signature[64] == 28,
            "v must be 27 or 28, but it was {}",
            signature[64]
        );
    }

    #[test]
    fn signing_is_deterministic() {
        // RFC 6979 makes ECDSA deterministic, so the same key and the
        // same digest always give the same signature. A run therefore
        // reproduces.
        let signer = Signer::from_hex(TEST_KEY).expect("the test key must parse");
        let digest = keccak(b"a test digest");
        let first = signer.sign_digest(&digest).expect("the signing must work");
        let second = signer.sign_digest(&digest).expect("the signing must work");
        assert_eq!(first, second);
    }

    #[test]
    fn the_s_value_is_in_the_low_half() {
        // Ethereum rejects a high-s signature as malleable. The curve
        // order halved is the boundary.
        let signer = Signer::from_hex(TEST_KEY).expect("the test key must parse");
        let digest = keccak(b"another digest");
        let signature = signer.sign_digest(&digest).expect("the signing must work");
        // The high half starts at 0x7FFF...A1 for secp256k1; a first
        // byte above 0x7F is a sure sign of a high-s value.
        assert!(
            signature[32] <= 0x7f,
            "s is in the high half, which Ethereum rejects"
        );
    }
}
