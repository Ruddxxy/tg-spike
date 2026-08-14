//! This module runs the x402 ask flow.
//!
//! The flow is: POST the question, read the 402 challenge, build and
//! sign an EIP-3009 authorisation, then POST again with the payment
//! header.
//!
//! ## What this module had to GUESS
//!
//! The node publishes the CHALLENGE shape, because it sends one. It
//! does not publish the shape of the payment it wants back. Two things
//! below are therefore a best reading of the x402 "exact" scheme for
//! EVM, not something read off this node:
//!
//! 1. The request header name, `X-PAYMENT`.
//! 2. The payload envelope: an outer object with `x402Version`,
//!    `scheme` and `network`, wrapping a `payload` that holds the
//!    signature and the authorisation fields.
//!
//! Both are marked with the word GUESS at their definition. One real
//! ask settles both, which is exactly why the run order is dry run,
//! then ONE ask, then the batch.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::challenge::{base64_encode, parse_header, Accept, Challenge};
use crate::eip712::{
    address_from_hex, domain_separator, keccak, signing_digest, to_hex, Authorization,
};
use crate::sign::Signer;

/// The chain this harness will pay on.
///
/// The harness refuses every other chain. This is a testnet-only tool
/// and the check is what keeps it that way.
pub const ALLOWED_CHAIN_ID: u64 = 84532;

/// GUESS: the header name that carries the payment.
///
/// The x402 "exact" scheme uses `X-PAYMENT` in its reference client.
/// The node's own 402 uses `payment-required` for the challenge, which
/// is consistent with that naming.
pub const PAYMENT_HEADER: &str = "X-PAYMENT";

/// How long an authorisation stays valid, in seconds.
///
/// The node states its own `maxTimeoutSeconds`. This value adds room
/// on top, so a slow round trip does not invalidate a good payment.
pub const VALIDITY_WINDOW_SECONDS: u64 = 600;

/// One prepared payment, before it is sent.
pub struct PreparedPayment {
    /// The leg this payment pays. The batch path reports it per ask.
    #[allow(dead_code)]
    pub leg: Accept,
    /// The EIP-712 domain separator this client built.
    pub domain_separator: [u8; 32],
    /// The EIP-712 struct hash of the authorisation.
    pub struct_hash: [u8; 32],
    /// The final digest that was signed.
    pub digest: [u8; 32],
    /// The 65 byte signature.
    pub signature: [u8; 65],
    /// The base64 header value to send.
    pub header_value: String,
    /// The payer address.
    pub from: [u8; 20],
    /// The amount, in the token's smallest unit.
    pub value: u64,
}

/// This function builds and signs a payment for one leg.
///
/// The function refuses any chain other than [`ALLOWED_CHAIN_ID`], and
/// it refuses a leg whose domain fields are absent.
pub fn prepare_payment(
    leg: &Accept,
    signer: &Signer,
    nonce_seed: &[u8],
) -> Result<PreparedPayment, String> {
    // The chain check runs BEFORE anything is signed.
    let wanted = format!("eip155:{ALLOWED_CHAIN_ID}");
    if leg.network != wanted {
        return Err(format!(
            "this harness signs for {wanted} only, but the leg is for {}",
            leg.network
        ));
    }
    if leg.scheme != "exact" {
        return Err(format!("the scheme {} is not supported", leg.scheme));
    }

    let asset = address_from_hex(&leg.asset)
        .ok_or_else(|| format!("the asset address is not valid: {}", leg.asset))?;
    let pay_to = address_from_hex(&leg.pay_to)
        .ok_or_else(|| format!("the payTo address is not valid: {}", leg.pay_to))?;
    let value: u64 = leg
        .amount
        .parse()
        .map_err(|_| format!("the amount is not a number: {}", leg.amount))?;

    let name = leg
        .extra
        .name
        .clone()
        .ok_or_else(|| "the leg gives no EIP-712 domain name".to_string())?;
    let version = leg
        .extra
        .version
        .clone()
        .ok_or_else(|| "the leg gives no EIP-712 domain version".to_string())?;

    let separator = domain_separator(&name, &version, ALLOWED_CHAIN_ID, &asset);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "the system clock is before the epoch".to_string())?
        .as_secs();

    // The nonce must be unused, not secret. A hash of the time and the
    // seed gives a value that never repeats and that a rerun can
    // reproduce for audit.
    let mut nonce_input = Vec::with_capacity(nonce_seed.len() + 8);
    nonce_input.extend_from_slice(&now.to_be_bytes());
    nonce_input.extend_from_slice(nonce_seed);
    let nonce = keccak(&nonce_input);

    let authorization = Authorization {
        from: signer.address(),
        to: pay_to,
        value,
        // validAfter of 0 means "valid from the start of time". The
        // validBefore field is what bounds the authorisation.
        valid_after: 0,
        valid_before: now + VALIDITY_WINDOW_SECONDS,
        nonce,
    };

    let struct_hash = authorization.struct_hash();
    let digest = signing_digest(&separator, &struct_hash);
    let signature = signer.sign_digest(&digest)?;

    // GUESS: the payment envelope. See the module doc comment.
    let payload = serde_json::json!({
        "x402Version": 2,
        "scheme": leg.scheme,
        "network": leg.network,
        "payload": {
            "signature": to_hex(&signature),
            "authorization": {
                "from": to_hex(&authorization.from),
                "to": to_hex(&authorization.to),
                "value": authorization.value.to_string(),
                "validAfter": authorization.valid_after.to_string(),
                "validBefore": authorization.valid_before.to_string(),
                "nonce": to_hex(&authorization.nonce),
            }
        }
    });
    let header_value = base64_encode(payload.to_string().as_bytes());

    Ok(PreparedPayment {
        leg: leg.clone(),
        domain_separator: separator,
        struct_hash,
        digest,
        signature,
        header_value,
        from: signer.address(),
        value,
    })
}

/// The outcome of one ask.
pub struct AskOutcome {
    /// The HTTP status of the paid request.
    pub status: u16,
    /// The response body.
    pub body: String,
    /// The payment this ask sent, if it sent one.
    pub spent_units: u64,
}

/// This function fetches the challenge for a question.
///
/// The function returns the challenge, or an error text. A node that
/// answers 200 without a challenge means the endpoint is not charging,
/// which is worth reporting rather than hiding.
pub fn fetch_challenge(endpoint: &str, question: &str, intent: &str) -> Result<Challenge, String> {
    let body = serde_json::json!({ "question": question, "intent": intent });
    let outcome = ureq::post(endpoint)
        .set("Content-Type", "application/json")
        .send_string(&body.to_string());

    match outcome {
        Ok(response) => Err(format!(
            "the node answered {} without a payment challenge",
            response.status()
        )),
        Err(ureq::Error::Status(402, response)) => {
            let header = response
                .header("payment-required")
                .ok_or_else(|| "the 402 carries no payment-required header".to_string())?;
            parse_header(header)
        }
        Err(ureq::Error::Status(status, response)) => {
            let text = response.into_string().unwrap_or_default();
            Err(format!("the node answered {status}: {text}"))
        }
        Err(error) => Err(format!("the request failed: {error}")),
    }
}

/// This function sends a paid ask.
///
/// The function never retries. A non-200 comes back as an error for
/// the caller to report, because a blind retry on a payment path can
/// spend twice.
pub fn send_paid_ask(
    endpoint: &str,
    question: &str,
    intent: &str,
    payment: &PreparedPayment,
) -> Result<AskOutcome, String> {
    let body = serde_json::json!({ "question": question, "intent": intent });
    let outcome = ureq::post(endpoint)
        .set("Content-Type", "application/json")
        .set(PAYMENT_HEADER, &payment.header_value)
        .send_string(&body.to_string());

    match outcome {
        Ok(response) => {
            let status = response.status();
            let text = response
                .into_string()
                .map_err(|error| format!("cannot read the response body: {error}"))?;
            Ok(AskOutcome {
                status,
                body: text,
                spent_units: payment.value,
            })
        }
        Err(ureq::Error::Status(status, response)) => {
            let text = response.into_string().unwrap_or_default();
            Err(format!("the paid ask answered {status}: {text}"))
        }
        Err(error) => Err(format!("the paid ask failed: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::challenge::parse_header;

    const REAL_HEADER: &str = "eyJ4NDAyVmVyc2lvbiI6MiwiZXJyb3IiOiJQYXltZW50IHJlcXVpcmVkIiwicmVzb3VyY2UiOnsidXJsIjoiaHR0cDovL2Rldm5vZGUudGVsZWdyYXBocHJvdG9jb2wuY29tL3YxL2FzayIsImRlc2NyaXB0aW9uIjoiUGF5bWVudCByZXF1aXJlZCBmb3IgTExNLXJvdXRlZCBpbmZlcmVuY2UuIiwibWltZVR5cGUiOiJhcHBsaWNhdGlvbi9qc29uIn0sImFjY2VwdHMiOlt7InNjaGVtZSI6ImV4YWN0IiwibmV0d29yayI6ImVpcDE1NTo4NDUzMiIsImFzc2V0IjoiMHgwMzZDYkQ1Mzg0MmM1NDI2NjM0ZTc5Mjk1NDFlQzIzMThmM2RDRjdlIiwiYW1vdW50IjoiMTAwMDAiLCJwYXlUbyI6IjB4NWEyMzI0YUExODYxM0ZBRDRlNDRiREYwZDZjNzNFYzFmNkQ4N2ZmOCIsIm1heFRpbWVvdXRTZWNvbmRzIjo2MCwiZXh0cmEiOnsibmFtZSI6IlVTREMiLCJ2ZXJzaW9uIjoiMiJ9fSx7InNjaGVtZSI6ImV4YWN0IiwibmV0d29yayI6InNvbGFuYTpFdFdUUkFCWmFZcTZpTWZlWUtvdVJ1MTY2VlUyeHFhMSIsImFzc2V0IjoiNHpNTUM5c3J0NVJpNVgxNEdBZ1hoYUhpaTNHblBBRUVSWVBKZ1pKRG5jRFUiLCJhbW91bnQiOiIxMDAwMCIsInBheVRvIjoiRzUzRWJlVFpTTnNBbjdiajZpTUZVUW5xM3pwRGRFYkhoS2tQUnl3bzhiaXgiLCJtYXhUaW1lb3V0U2Vjb25kcyI6NjAsImV4dHJhIjp7ImZlZVBheWVyIjoiMndLdXBMUjlxNndYWXBwdzhHcjJOdld4S0JVcW00UFBKS2tRZm94SERCZzQifX1dfQ==";

    /// A well known TEST key. It holds nothing.
    const TEST_KEY: &str = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";

    #[test]
    fn a_payment_is_prepared_for_the_base_sepolia_leg() {
        let challenge = parse_header(REAL_HEADER).expect("the header must parse");
        let leg = challenge.eip155_leg(84532).expect("the leg must be there");
        let signer = Signer::from_hex(TEST_KEY).expect("the test key must parse");
        let payment = prepare_payment(leg, &signer, b"test").expect("the payment must prepare");

        assert_eq!(payment.value, 10000);
        assert_eq!(payment.signature.len(), 65);
        // The domain separator must match the deployed contract.
        assert_eq!(
            to_hex(&payment.domain_separator),
            "0x71f17a3b2ff373b803d70a5a07c046c1a2bc8e89c09ef722fcb047abe94c9818"
        );
    }

    #[test]
    fn the_solana_leg_is_refused_before_signing() {
        let challenge = parse_header(REAL_HEADER).expect("the header must parse");
        let solana = challenge
            .accepts
            .iter()
            .find(|leg| leg.network.starts_with("solana:"))
            .expect("the Solana leg must be there");
        let signer = Signer::from_hex(TEST_KEY).expect("the test key must parse");
        assert!(prepare_payment(solana, &signer, b"test").is_err());
    }

    #[test]
    fn a_mainnet_leg_is_refused_before_signing() {
        // This is the guard that keeps the harness on testnet. A leg
        // that names Base mainnet must never be signed.
        let challenge = parse_header(REAL_HEADER).expect("the header must parse");
        let mut leg = challenge
            .eip155_leg(84532)
            .expect("the leg must be there")
            .clone();
        leg.network = "eip155:8453".to_string();
        let signer = Signer::from_hex(TEST_KEY).expect("the test key must parse");
        let outcome = prepare_payment(&leg, &signer, b"test");
        assert!(outcome.is_err(), "a mainnet leg must be refused");
    }

    #[test]
    fn the_payment_header_decodes_to_the_expected_envelope() {
        let challenge = parse_header(REAL_HEADER).expect("the header must parse");
        let leg = challenge.eip155_leg(84532).expect("the leg must be there");
        let signer = Signer::from_hex(TEST_KEY).expect("the test key must parse");
        let payment = prepare_payment(leg, &signer, b"test").expect("the payment must prepare");

        let raw = crate::challenge::base64_decode(&payment.header_value)
            .expect("the header must be base64");
        let text = String::from_utf8(raw).expect("the payload must be UTF-8");
        let value: serde_json::Value =
            serde_json::from_str(&text).expect("the payload must be JSON");

        assert_eq!(value["x402Version"], 2);
        assert_eq!(value["scheme"], "exact");
        assert_eq!(value["network"], "eip155:84532");
        assert_eq!(value["payload"]["authorization"]["value"], "10000");
        assert_eq!(
            value["payload"]["authorization"]["to"],
            "0x5a2324aa18613fad4e44bdf0d6c73ec1f6d87ff8"
        );
    }

    #[test]
    fn the_header_never_holds_the_private_key() {
        let challenge = parse_header(REAL_HEADER).expect("the header must parse");
        let leg = challenge.eip155_leg(84532).expect("the leg must be there");
        let signer = Signer::from_hex(TEST_KEY).expect("the test key must parse");
        let payment = prepare_payment(leg, &signer, b"test").expect("the payment must prepare");

        let bare_key = TEST_KEY.trim_start_matches("0x");
        assert!(
            !payment.header_value.contains(bare_key),
            "the payment header leaked the key"
        );
        let raw = crate::challenge::base64_decode(&payment.header_value)
            .expect("the header must be base64");
        let text = String::from_utf8(raw).expect("the payload must be UTF-8");
        assert!(!text.contains(bare_key), "the payload leaked the key");
    }
}
