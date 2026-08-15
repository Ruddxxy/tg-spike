//! This module reads the x402 payment challenge.
//!
//! The node answers an unpaid ask with HTTP 402 and a
//! `payment-required` header. The header holds base64 of a JSON
//! document. This module decodes it and picks the leg to pay.
//!
//! Nothing in this module is hardcoded from a past observation. Every
//! value comes out of the live response. The amounts and addresses
//! WILL change on mainnet, and they can change on testnet too, so a
//! client that remembers them is a client that pays the wrong party.

use serde::Deserialize;

/// The whole decoded challenge.
#[derive(Debug, Deserialize)]
pub struct Challenge {
    /// The x402 protocol version the node speaks.
    #[serde(rename = "x402Version")]
    pub x402_version: u32,
    /// The error text the node gave.
    #[serde(default)]
    pub error: String,
    /// The payment legs the node accepts.
    pub accepts: Vec<Accept>,
    /// The challenge exactly as it arrived, before any typing.
    ///
    /// The v2 payment payload echoes the `resource` object and the
    /// CHOSEN leg back to the node. Echoing the raw JSON rather than
    /// re-serialising the typed struct means a field this client does
    /// not model still reaches the node unchanged. A dropped field is
    /// a rejected payment, and the node is the authority on its own
    /// challenge.
    #[serde(skip)]
    pub raw: serde_json::Value,
}

/// One payment leg.
#[derive(Debug, Deserialize, Clone)]
pub struct Accept {
    /// The payment scheme. This client handles `exact` only.
    pub scheme: String,
    /// The CAIP-2 network id, for example `eip155:84532`.
    pub network: String,
    /// The token contract address, or the Solana mint.
    pub asset: String,
    /// The amount, in the token's smallest unit, as a decimal text.
    pub amount: String,
    /// The address that receives the payment.
    #[serde(rename = "payTo")]
    pub pay_to: String,
    /// How long the node will wait for settlement.
    #[serde(rename = "maxTimeoutSeconds")]
    pub max_timeout_seconds: u64,
    /// Scheme-specific extra fields.
    #[serde(default)]
    pub extra: Extra,
}

/// The extra fields of a payment leg.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct Extra {
    /// The EIP-712 domain name, for an eip155 leg.
    #[serde(default)]
    pub name: Option<String>,
    /// The EIP-712 domain version, for an eip155 leg.
    #[serde(default)]
    pub version: Option<String>,
    /// The sponsor that pays the fee, for a Solana leg.
    ///
    /// This client does not use the Solana leg yet. The sponsor is
    /// worth a look if Base Sepolia gas ever becomes the problem: a
    /// sponsored fee payer means the caller needs no native token at
    /// all, only the stablecoin.
    #[serde(rename = "feePayer", default)]
    pub fee_payer: Option<String>,
}

impl Challenge {
    /// This function picks the leg for a chain id.
    ///
    /// The function matches on the CAIP-2 name, so it cannot pick a
    /// leg for a different chain by accident.
    ///
    /// The paying path uses [`Challenge::eip155_leg_index`] instead,
    /// because it also needs the raw JSON at that position. This reader
    /// stays for the tests that check leg selection on its own.
    #[allow(dead_code)]
    pub fn eip155_leg(&self, chain_id: u64) -> Option<&Accept> {
        self.eip155_leg_index(chain_id)
            .map(|index| &self.accepts[index])
    }

    /// This function gives the POSITION of the leg for a chain id.
    ///
    /// The payment payload echoes the chosen leg back as raw JSON, and
    /// the position is what finds it in [`Challenge::raw`].
    pub fn eip155_leg_index(&self, chain_id: u64) -> Option<usize> {
        let wanted = format!("eip155:{chain_id}");
        self.accepts
            .iter()
            .position(|leg| leg.network == wanted && leg.scheme == "exact")
    }

    /// This function gives the raw JSON of one leg.
    ///
    /// The function returns `None` when the raw challenge is absent,
    /// which happens only for a value this client built itself rather
    /// than decoded from a node.
    pub fn raw_leg(&self, index: usize) -> Option<&serde_json::Value> {
        self.raw.get("accepts")?.get(index)
    }

    /// This function gives the raw `resource` object of the challenge.
    ///
    /// The v2 payload carries it. A challenge with no `resource` gives
    /// `None`, and the payload then omits the field rather than
    /// sending a null.
    pub fn raw_resource(&self) -> Option<&serde_json::Value> {
        self.raw.get("resource")
    }
}

/// This function decodes standard base64 with padding.
///
/// The function is written out here rather than pulled from a crate,
/// to keep this crate to cryptographic and HTTP primitives only.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    /// This function gives the 6 bit value of one base64 character.
    fn value_of(character: u8) -> Option<u8> {
        match character {
            b'A'..=b'Z' => Some(character - b'A'),
            b'a'..=b'z' => Some(character - b'a' + 26),
            b'0'..=b'9' => Some(character - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    let cleaned: Vec<u8> = text
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();

    let mut output = Vec::with_capacity(cleaned.len() / 4 * 3);
    let mut accumulator: u32 = 0;
    let mut bits: u32 = 0;
    for byte in cleaned {
        if byte == b'=' {
            break;
        }
        let value = value_of(byte)? as u32;
        accumulator = (accumulator << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push(((accumulator >> bits) & 0xff) as u8);
        }
    }
    Some(output)
}

/// This function encodes bytes as standard base64 with padding.
pub fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0] as u32;
        let second = chunk.get(1).copied().unwrap_or(0) as u32;
        let third = chunk.get(2).copied().unwrap_or(0) as u32;
        let block = (first << 16) | (second << 8) | third;

        output.push(ALPHABET[((block >> 18) & 0x3f) as usize] as char);
        output.push(ALPHABET[((block >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            output.push(ALPHABET[((block >> 6) & 0x3f) as usize] as char);
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(ALPHABET[(block & 0x3f) as usize] as char);
        } else {
            output.push('=');
        }
    }
    output
}

/// This function reads a challenge out of a base64 header value.
///
/// The function keeps the decoded JSON on the returned value, because
/// the payment payload has to echo parts of it back unchanged.
pub fn parse_header(header_value: &str) -> Result<Challenge, String> {
    let raw = base64_decode(header_value.trim())
        .ok_or_else(|| "the payment-required header is not valid base64".to_string())?;
    let text = String::from_utf8(raw)
        .map_err(|_| "the decoded challenge is not valid UTF-8".to_string())?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| format!("the challenge is not valid JSON: {error}"))?;
    let mut challenge: Challenge = serde_json::from_value(value.clone())
        .map_err(|error| format!("the challenge is not a valid x402 challenge: {error}"))?;
    challenge.raw = value;
    Ok(challenge)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This is the real header the dev node sent on 2026-08-14.
    const REAL_HEADER: &str = "eyJ4NDAyVmVyc2lvbiI6MiwiZXJyb3IiOiJQYXltZW50IHJlcXVpcmVkIiwicmVzb3VyY2UiOnsidXJsIjoiaHR0cDovL2Rldm5vZGUudGVsZWdyYXBocHJvdG9jb2wuY29tL3YxL2FzayIsImRlc2NyaXB0aW9uIjoiUGF5bWVudCByZXF1aXJlZCBmb3IgTExNLXJvdXRlZCBpbmZlcmVuY2UuIiwibWltZVR5cGUiOiJhcHBsaWNhdGlvbi9qc29uIn0sImFjY2VwdHMiOlt7InNjaGVtZSI6ImV4YWN0IiwibmV0d29yayI6ImVpcDE1NTo4NDUzMiIsImFzc2V0IjoiMHgwMzZDYkQ1Mzg0MmM1NDI2NjM0ZTc5Mjk1NDFlQzIzMThmM2RDRjdlIiwiYW1vdW50IjoiMTAwMDAiLCJwYXlUbyI6IjB4NWEyMzI0YUExODYxM0ZBRDRlNDRiREYwZDZjNzNFYzFmNkQ4N2ZmOCIsIm1heFRpbWVvdXRTZWNvbmRzIjo2MCwiZXh0cmEiOnsibmFtZSI6IlVTREMiLCJ2ZXJzaW9uIjoiMiJ9fSx7InNjaGVtZSI6ImV4YWN0IiwibmV0d29yayI6InNvbGFuYTpFdFdUUkFCWmFZcTZpTWZlWUtvdVJ1MTY2VlUyeHFhMSIsImFzc2V0IjoiNHpNTUM5c3J0NVJpNVgxNEdBZ1hoYUhpaTNHblBBRUVSWVBKZ1pKRG5jRFUiLCJhbW91bnQiOiIxMDAwMCIsInBheVRvIjoiRzUzRWJlVFpTTnNBbjdiajZpTUZVUW5xM3pwRGRFYkhoS2tQUnl3bzhiaXgiLCJtYXhUaW1lb3V0U2Vjb25kcyI6NjAsImV4dHJhIjp7ImZlZVBheWVyIjoiMndLdXBMUjlxNndYWXBwdzhHcjJOdld4S0JVcW00UFBKS2tRZm94SERCZzQifX1dfQ==";

    #[test]
    fn base64_round_trips() {
        for sample in [b"".as_slice(), b"a", b"ab", b"abc", b"abcd", b"hello world"] {
            let encoded = base64_encode(sample);
            let decoded = base64_decode(&encoded).expect("the encoding must decode");
            assert_eq!(decoded, sample, "round trip failed for {sample:?}");
        }
    }

    #[test]
    fn the_real_header_parses() {
        let challenge = parse_header(REAL_HEADER).expect("the real header must parse");
        assert_eq!(challenge.x402_version, 2);
        assert_eq!(challenge.accepts.len(), 2);
    }

    #[test]
    fn the_base_sepolia_leg_is_found_and_complete() {
        let challenge = parse_header(REAL_HEADER).expect("the real header must parse");
        let leg = challenge
            .eip155_leg(84532)
            .expect("the Base Sepolia leg must be present");
        assert_eq!(leg.scheme, "exact");
        assert_eq!(leg.asset, "0x036CbD53842c5426634e7929541eC2318f3dCF7e");
        assert_eq!(leg.amount, "10000");
        assert_eq!(leg.pay_to, "0x5a2324aA18613FAD4e44bDF0d6c73Ec1f6D87ff8");
        assert_eq!(leg.extra.name.as_deref(), Some("USDC"));
        assert_eq!(leg.extra.version.as_deref(), Some("2"));
    }

    #[test]
    fn a_leg_for_another_chain_is_not_returned() {
        let challenge = parse_header(REAL_HEADER).expect("the real header must parse");
        // Mainnet must never match the testnet leg.
        assert!(challenge.eip155_leg(1).is_none());
        assert!(challenge.eip155_leg(8453).is_none());
    }

    #[test]
    fn the_solana_leg_carries_a_fee_payer() {
        // This client does not use it, but the sponsor is recorded so a
        // later Solana payment path can find it.
        let challenge = parse_header(REAL_HEADER).expect("the real header must parse");
        let solana = challenge
            .accepts
            .iter()
            .find(|leg| leg.network.starts_with("solana:"))
            .expect("the Solana leg must be present");
        assert!(solana.extra.fee_payer.is_some());
    }

    #[test]
    fn bad_base64_is_an_error_not_a_panic() {
        assert!(parse_header("not base64 at all !!!").is_err());
    }
}
