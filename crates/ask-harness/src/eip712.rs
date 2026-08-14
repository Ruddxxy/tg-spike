//! This module builds the EIP-712 payload for an EIP-3009
//! `transferWithAuthorization`.
//!
//! The module builds every byte by hand. It has no Ethereum library.
//! The two encodings it needs are simple, and writing them out makes
//! each field visible to a reviewer:
//!
//! - `abi.encode` for a fixed-size field is a 32 byte big-endian word.
//!   An address goes in the low 20 bytes, with 12 zero bytes in front.
//!   A `uint256` goes in the full 32 bytes. A `bytes32` goes as it is.
//! - A `string` field is NOT inlined. EIP-712 hashes it and puts the
//!   32 byte hash in the word.
//!
//! ## The two hashes
//!
//! The domain separator is
//! `keccak256(abi.encode(typeHash, keccak256(name), keccak256(version),
//! chainId, verifyingContract))`.
//!
//! The message digest is `keccak256(0x19 || 0x01 || domainSeparator ||
//! structHash)`. The `0x19 0x01` prefix is what keeps a signed
//! EIP-712 payload from ever being a valid transaction.

use sha3::{Digest, Keccak256};

/// The EIP-712 domain type string for a USDC style token.
///
/// The field order in this string fixes the field order in the
/// encoding. It must match the contract exactly, or the separator
/// differs and the token rejects the signature with no useful reason.
pub const DOMAIN_TYPE: &str =
    "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";

/// The EIP-3009 authorisation type string.
///
/// This is fixed by the EIP. The field order here also fixes the
/// encoding order below.
pub const TRANSFER_TYPE: &str = "TransferWithAuthorization(address from,address to,uint256 value,uint256 validAfter,uint256 validBefore,bytes32 nonce)";

/// This function gives the Keccak-256 hash of some bytes.
pub fn keccak(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(bytes);
    let output = hasher.finalize();
    let mut result = [0u8; 32];
    result.copy_from_slice(&output);
    result
}

/// This function writes a `u64` into a 32 byte big-endian word.
pub fn word_from_u64(value: u64) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[24..32].copy_from_slice(&value.to_be_bytes());
    word
}

/// This function writes a 20 byte address into a 32 byte word.
///
/// The address sits in the LOW 20 bytes. The first 12 bytes stay zero.
pub fn word_from_address(address: &[u8; 20]) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[12..32].copy_from_slice(address);
    word
}

/// This function builds the EIP-712 domain separator.
///
/// The caller must check this value against the `DOMAIN_SEPARATOR()`
/// that the deployed contract returns, BEFORE it signs anything. A
/// domain that does not match gives an opaque rejection on chain.
pub fn domain_separator(
    name: &str,
    version: &str,
    chain_id: u64,
    verifying_contract: &[u8; 20],
) -> [u8; 32] {
    let mut buffer = Vec::with_capacity(160);
    buffer.extend_from_slice(&keccak(DOMAIN_TYPE.as_bytes()));
    // A string field is hashed, never inlined.
    buffer.extend_from_slice(&keccak(name.as_bytes()));
    buffer.extend_from_slice(&keccak(version.as_bytes()));
    buffer.extend_from_slice(&word_from_u64(chain_id));
    buffer.extend_from_slice(&word_from_address(verifying_contract));
    keccak(&buffer)
}

/// One EIP-3009 authorisation, before encoding.
pub struct Authorization {
    /// The payer address.
    pub from: [u8; 20],
    /// The address that receives the value.
    pub to: [u8; 20],
    /// The token amount, in the token's own smallest unit.
    pub value: u64,
    /// The first second at which the authorisation is valid.
    pub valid_after: u64,
    /// The second after which the authorisation is no longer valid.
    pub valid_before: u64,
    /// A unique 32 byte nonce. It is NOT a counter; the token stores
    /// each used nonce, so any unused random value works.
    pub nonce: [u8; 32],
}

impl Authorization {
    /// This function builds the EIP-712 struct hash.
    pub fn struct_hash(&self) -> [u8; 32] {
        let mut buffer = Vec::with_capacity(224);
        buffer.extend_from_slice(&keccak(TRANSFER_TYPE.as_bytes()));
        buffer.extend_from_slice(&word_from_address(&self.from));
        buffer.extend_from_slice(&word_from_address(&self.to));
        buffer.extend_from_slice(&word_from_u64(self.value));
        buffer.extend_from_slice(&word_from_u64(self.valid_after));
        buffer.extend_from_slice(&word_from_u64(self.valid_before));
        // A bytes32 field goes in as it is, with no hash.
        buffer.extend_from_slice(&self.nonce);
        keccak(&buffer)
    }
}

/// This function builds the final EIP-712 digest to sign.
///
/// The `0x19 0x01` prefix is required. It makes the payload
/// impossible to confuse with an Ethereum transaction.
pub fn signing_digest(domain_separator: &[u8; 32], struct_hash: &[u8; 32]) -> [u8; 32] {
    let mut buffer = Vec::with_capacity(66);
    buffer.push(0x19);
    buffer.push(0x01);
    buffer.extend_from_slice(domain_separator);
    buffer.extend_from_slice(struct_hash);
    keccak(&buffer)
}

/// This function reads a hex text into a 20 byte address.
///
/// The function accepts an optional `0x` prefix. It returns `None` for
/// any text that is not exactly 20 bytes of hex.
pub fn address_from_hex(text: &str) -> Option<[u8; 20]> {
    let cleaned = text.strip_prefix("0x").unwrap_or(text);
    if cleaned.len() != 40 {
        return None;
    }
    let mut address = [0u8; 20];
    for (index, pair) in cleaned.as_bytes().chunks(2).enumerate() {
        let hex = core::str::from_utf8(pair).ok()?;
        address[index] = u8::from_str_radix(hex, 16).ok()?;
    }
    Some(address)
}

/// This function renders bytes as a lowercase hex text with a `0x`
/// prefix.
pub fn to_hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(2 + bytes.len() * 2);
    text.push_str("0x");
    for byte in bytes {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keccak_matches_the_known_empty_hash() {
        // The Keccak-256 of the empty input is a well known constant.
        assert_eq!(
            to_hex(&keccak(b"")),
            "0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
        );
    }

    #[test]
    fn the_domain_type_hash_is_the_known_constant() {
        // This type hash appears in every EIP-712 implementation.
        assert_eq!(
            to_hex(&keccak(DOMAIN_TYPE.as_bytes())),
            "0x8b73c3c69bb8fe3d512ecc4cf759cc79239f7b179b0ffacaa9a75d522b39400f"
        );
    }

    #[test]
    fn the_transfer_type_hash_is_the_eip3009_constant() {
        // EIP-3009 fixes this value.
        assert_eq!(
            to_hex(&keccak(TRANSFER_TYPE.as_bytes())),
            "0x7c7c6cdb67a18743f49ec6fa9b35f50d52ed05cbed4cc592e13b44501c1a2267"
        );
    }

    #[test]
    fn an_address_word_keeps_the_address_in_the_low_bytes() {
        let address = address_from_hex("0x036CbD53842c5426634e7929541eC2318f3dCF7e")
            .expect("a valid address must parse");
        let word = word_from_address(&address);
        assert_eq!(&word[..12], &[0u8; 12]);
        assert_eq!(&word[12..], &address);
    }

    #[test]
    fn a_number_word_is_big_endian() {
        let word = word_from_u64(10000);
        assert_eq!(word[31], 0x10);
        assert_eq!(word[30], 0x27);
        assert_eq!(&word[..30], &[0u8; 30]);
    }

    #[test]
    fn address_parsing_rejects_a_wrong_length() {
        assert!(address_from_hex("0x1234").is_none());
        assert!(address_from_hex("").is_none());
    }

    #[test]
    fn the_base_sepolia_usdc_domain_matches_the_deployed_contract() {
        // This is the check that stops a wasted spend. The value on the
        // right was read from the deployed contract on Base Sepolia
        // with DOMAIN_SEPARATOR() at 0x3644e515:
        //
        //   curl -sS -X POST https://sepolia.base.org \
        //     -H 'Content-Type: application/json' \
        //     -d '{"jsonrpc":"2.0","id":1,"method":"eth_call","params":
        //          [{"to":"0x036CbD53842c5426634e7929541eC2318f3dCF7e",
        //            "data":"0x3644e515"},"latest"]}'
        //
        // name() gave "USDC" and version() gave "2", which match the
        // challenge's extra fields.
        let asset = address_from_hex("0x036CbD53842c5426634e7929541eC2318f3dCF7e")
            .expect("the asset address must parse");
        let built = domain_separator("USDC", "2", 84532, &asset);
        assert_eq!(
            to_hex(&built),
            "0x71f17a3b2ff373b803d70a5a07c046c1a2bc8e89c09ef722fcb047abe94c9818",
            "the built domain separator must match the deployed contract"
        );
    }
}
