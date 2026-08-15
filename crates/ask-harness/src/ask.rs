//! This module runs the x402 ask flow.
//!
//! The flow is: POST the question, read the 402 challenge, build and
//! sign an EIP-3009 authorisation, then POST again with the payment
//! header.
//!
//! ## The header names and the envelope come from the v2 spec
//!
//! An earlier version of this module GUESSED both, from the x402 v1
//! reference client. Both guesses were wrong, and the node answered
//! 402 a second time with no stated reason. The values below are now
//! read from the specification:
//!
//! - specs/x402-specification-v2.md in github.com/coinbase/x402
//! - docs.x402.org/core-concepts/http-402
//!
//! Three headers carry the protocol in v2, and none of them has the
//! `X-` prefix that v1 used:
//!
//! | direction       | header            | holds                  |
//! | --------------- | ----------------- | ---------------------- |
//! | server → client | `PAYMENT-REQUIRED`| the challenge          |
//! | client → server | `PAYMENT-SIGNATURE`| the signed payload    |
//! | server → client | `PAYMENT-RESPONSE`| the settlement result  |
//!
//! The envelope moved too. v1 wrapped the payload in a flat object
//! carrying `scheme` and `network`. v2 replaces those two fields with
//! `accepted`, holding the WHOLE leg the client chose, and adds
//! `resource` and `extensions`. A v1 envelope sent to a v2 node is
//! missing `accepted` entirely, so the node cannot tell which leg is
//! being paid.
//!
//! `resource` and `accepted` are echoed as the raw JSON that arrived,
//! not re-serialised from the typed structs, so a field this client
//! does not model still reaches the node unchanged.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::challenge::{base64_decode, base64_encode, parse_header, Accept, Challenge};
use crate::eip712::{
    address_from_hex, domain_separator, keccak, signing_digest, to_hex, Authorization,
};
use crate::sign::Signer;

/// The chain this harness will pay on.
///
/// The harness refuses every other chain. This is a testnet-only tool
/// and the check is what keeps it that way.
pub const ALLOWED_CHAIN_ID: u64 = 84532;

/// The request header that carries the signed payment payload.
///
/// x402 v2. The v1 name was `X-PAYMENT`; sending that to a v2 node
/// gets a second 402 rather than a useful error.
pub const PAYMENT_HEADER: &str = "PAYMENT-SIGNATURE";

/// The response header that carries the settlement result.
///
/// The node sets it whether settlement succeeded or failed, so it is
/// read on BOTH paths. It is the only place a failure states its
/// reason; without it a rejected payment is an opaque non-200.
pub const SETTLEMENT_HEADER: &str = "PAYMENT-RESPONSE";

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

/// This function builds and signs a payment for one leg of a challenge.
///
/// The function refuses any chain other than [`ALLOWED_CHAIN_ID`], and
/// it refuses a leg whose domain fields are absent.
///
/// The function takes the whole challenge rather than one leg, because
/// the v2 payload echoes the challenge's `resource` object and the
/// chosen leg back to the node.
pub fn prepare_payment(
    challenge: &Challenge,
    leg_index: usize,
    signer: &Signer,
    nonce_seed: &[u8],
) -> Result<PreparedPayment, String> {
    let leg = challenge
        .accepts
        .get(leg_index)
        .ok_or_else(|| format!("the challenge has no leg at position {leg_index}"))?;

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

    // The v2 PaymentPayload envelope. See the module doc comment for
    // the specification sections this follows.
    //
    // `accepted` is the chosen leg echoed back verbatim. Re-serialising
    // it from the typed Accept would silently drop any field this
    // client does not model, and the node validates what it sent.
    let accepted = challenge
        .raw_leg(leg_index)
        .cloned()
        .ok_or_else(|| "the challenge carries no raw JSON to echo back".to_string())?;

    let mut payload = serde_json::json!({
        "x402Version": challenge.x402_version,
        "accepted": accepted,
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
        },
        "extensions": {}
    });

    // `resource` is echoed when the challenge carried one. A missing
    // resource is left out, never sent as a null.
    if let Some(resource) = challenge.raw_resource() {
        payload["resource"] = resource.clone();
    }

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

/// The node's settlement result, from the `PAYMENT-RESPONSE` header.
///
/// Every field is optional, because a failure path may carry only a
/// reason. The raw JSON is kept so a field this client does not model
/// still reaches the report.
#[derive(Debug)]
pub struct Settlement {
    /// Whether the node settled the payment.
    pub success: Option<bool>,
    /// The settlement transaction hash, when there is one.
    pub transaction: Option<String>,
    /// The network the settlement happened on.
    pub network: Option<String>,
    /// The address the node believes paid.
    pub payer: Option<String>,
    /// The stated failure reason, when the node gives one.
    pub error_reason: Option<String>,
    /// The whole decoded document.
    pub raw: serde_json::Value,
}

impl Settlement {
    /// This function decodes a settlement out of a base64 header.
    pub fn from_header(header_value: &str) -> Result<Self, String> {
        let bytes = base64_decode(header_value.trim())
            .ok_or_else(|| format!("the {SETTLEMENT_HEADER} header is not valid base64"))?;
        let text = String::from_utf8(bytes)
            .map_err(|_| format!("the {SETTLEMENT_HEADER} header is not valid UTF-8"))?;
        let raw: serde_json::Value = serde_json::from_str(&text).map_err(|error| {
            format!("the {SETTLEMENT_HEADER} header is not valid JSON: {error}")
        })?;

        /// This helper reads an optional text field.
        fn text_field(value: &serde_json::Value, name: &str) -> Option<String> {
            value.get(name)?.as_str().map(str::to_string)
        }

        Ok(Settlement {
            success: raw.get("success").and_then(serde_json::Value::as_bool),
            transaction: text_field(&raw, "transaction"),
            network: text_field(&raw, "network"),
            payer: text_field(&raw, "payer"),
            // The spec names no single failure field, so both spellings
            // seen in the wild are read.
            error_reason: text_field(&raw, "errorReason").or_else(|| text_field(&raw, "error")),
            raw,
        })
    }
}

/// The outcome of one ask.
pub struct AskOutcome {
    /// The HTTP status of the paid request.
    pub status: u16,
    /// The response body.
    pub body: String,
    /// The amount this ask AUTHORISED, in the token's smallest unit.
    ///
    /// This is not proof of spending. `settlement.success` is what says
    /// whether the node took the money.
    pub authorized_units: u64,
    /// The decoded `PAYMENT-RESPONSE`, when the node sent one.
    ///
    /// The `Err` case means the header was present but unreadable,
    /// which is worth reporting rather than hiding.
    pub settlement: Option<Result<Settlement, String>>,
}

/// This function builds the auto-routed ask body.
///
/// The Engine's auto-routed endpoint takes ONE required field,
/// `query`, holding a natural-language question. Its LLM router
/// classifies the query into an intent and picks the miner.
///
/// An earlier version sent `question` and `intent`. That is not the
/// documented shape, and the node answered 400 "invalid request body"
/// AFTER the payment gate passed, which reads like a payment failure
/// and is not one.
///
/// The auto-routed path is also the correct one here for a second
/// reason. `POST /engine/v1/ask/{miner_id}` calls a named miner
/// directly. This harness must never do that: choosing the miner would
/// destroy the routing distribution the probe exists to measure, and
/// targeting a miner is what hackathon rule 04 forbids.
fn ask_body(query: &str) -> serde_json::Value {
    serde_json::json!({ "query": query })
}

/// This function fetches the challenge for a query.
///
/// The function returns the challenge, or an error text. A node that
/// answers 200 without a challenge means the endpoint is not charging,
/// which is worth reporting rather than hiding.
pub fn fetch_challenge(endpoint: &str, query: &str) -> Result<Challenge, String> {
    let body = ask_body(query);
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
/// The function never retries. A blind retry on a payment path can
/// spend twice.
///
/// A non-2xx status is NOT an error here. It comes back as a normal
/// outcome carrying its status, its body, and its decoded
/// `PAYMENT-RESPONSE`, because the failure path is exactly where the
/// settlement reason lives. Only a transport failure gives `Err`.
pub fn send_paid_ask(
    endpoint: &str,
    query: &str,
    payment: &PreparedPayment,
) -> Result<AskOutcome, String> {
    let body = ask_body(query);
    let outcome = ureq::post(endpoint)
        .set("Content-Type", "application/json")
        .set(PAYMENT_HEADER, &payment.header_value)
        .send_string(&body.to_string());

    let response = match outcome {
        Ok(response) => response,
        Err(ureq::Error::Status(_, response)) => response,
        Err(error) => return Err(format!("the paid ask failed: {error}")),
    };

    let status = response.status();
    // The header has to be read BEFORE the body, because reading the
    // body consumes the response.
    let settlement = response
        .header(SETTLEMENT_HEADER)
        .map(Settlement::from_header);
    let text = response
        .into_string()
        .map_err(|error| format!("cannot read the response body: {error}"))?;

    Ok(AskOutcome {
        status,
        body: text,
        authorized_units: payment.value,
        settlement,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::challenge::parse_header;

    const REAL_HEADER: &str = "eyJ4NDAyVmVyc2lvbiI6MiwiZXJyb3IiOiJQYXltZW50IHJlcXVpcmVkIiwicmVzb3VyY2UiOnsidXJsIjoiaHR0cDovL2Rldm5vZGUudGVsZWdyYXBocHJvdG9jb2wuY29tL3YxL2FzayIsImRlc2NyaXB0aW9uIjoiUGF5bWVudCByZXF1aXJlZCBmb3IgTExNLXJvdXRlZCBpbmZlcmVuY2UuIiwibWltZVR5cGUiOiJhcHBsaWNhdGlvbi9qc29uIn0sImFjY2VwdHMiOlt7InNjaGVtZSI6ImV4YWN0IiwibmV0d29yayI6ImVpcDE1NTo4NDUzMiIsImFzc2V0IjoiMHgwMzZDYkQ1Mzg0MmM1NDI2NjM0ZTc5Mjk1NDFlQzIzMThmM2RDRjdlIiwiYW1vdW50IjoiMTAwMDAiLCJwYXlUbyI6IjB4NWEyMzI0YUExODYxM0ZBRDRlNDRiREYwZDZjNzNFYzFmNkQ4N2ZmOCIsIm1heFRpbWVvdXRTZWNvbmRzIjo2MCwiZXh0cmEiOnsibmFtZSI6IlVTREMiLCJ2ZXJzaW9uIjoiMiJ9fSx7InNjaGVtZSI6ImV4YWN0IiwibmV0d29yayI6InNvbGFuYTpFdFdUUkFCWmFZcTZpTWZlWUtvdVJ1MTY2VlUyeHFhMSIsImFzc2V0IjoiNHpNTUM5c3J0NVJpNVgxNEdBZ1hoYUhpaTNHblBBRUVSWVBKZ1pKRG5jRFUiLCJhbW91bnQiOiIxMDAwMCIsInBheVRvIjoiRzUzRWJlVFpTTnNBbjdiajZpTUZVUW5xM3pwRGRFYkhoS2tQUnl3bzhiaXgiLCJtYXhUaW1lb3V0U2Vjb25kcyI6NjAsImV4dHJhIjp7ImZlZVBheWVyIjoiMndLdXBMUjlxNndYWXBwdzhHcjJOdld4S0JVcW00UFBKS2tRZm94SERCZzQifX1dfQ==";

    /// A well known TEST key. It holds nothing.
    const TEST_KEY: &str = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";

    /// This helper prepares a payment from the real challenge.
    fn prepare_from_real_header() -> PreparedPayment {
        let challenge = parse_header(REAL_HEADER).expect("the header must parse");
        let index = challenge
            .eip155_leg_index(84532)
            .expect("the leg must be there");
        let signer = Signer::from_hex(TEST_KEY).expect("the test key must parse");
        prepare_payment(&challenge, index, &signer, b"test").expect("the payment must prepare")
    }

    /// This helper decodes a payment header back to JSON.
    fn decode(header_value: &str) -> serde_json::Value {
        let raw = base64_decode(header_value).expect("the header must be base64");
        let text = String::from_utf8(raw).expect("the payload must be UTF-8");
        serde_json::from_str(&text).expect("the payload must be JSON")
    }

    #[test]
    fn a_payment_is_prepared_for_the_base_sepolia_leg() {
        let payment = prepare_from_real_header();
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
            .position(|leg| leg.network.starts_with("solana:"))
            .expect("the Solana leg must be there");
        let signer = Signer::from_hex(TEST_KEY).expect("the test key must parse");
        assert!(prepare_payment(&challenge, solana, &signer, b"test").is_err());
    }

    #[test]
    fn a_mainnet_leg_is_refused_before_signing() {
        // This is the guard that keeps the harness on testnet. A leg
        // that names Base mainnet must never be signed.
        let mut challenge = parse_header(REAL_HEADER).expect("the header must parse");
        let index = challenge
            .eip155_leg_index(84532)
            .expect("the leg must be there");
        challenge.accepts[index].network = "eip155:8453".to_string();
        let signer = Signer::from_hex(TEST_KEY).expect("the test key must parse");
        let outcome = prepare_payment(&challenge, index, &signer, b"test");
        assert!(outcome.is_err(), "a mainnet leg must be refused");
    }

    #[test]
    fn the_payment_header_carries_the_v2_envelope() {
        // The v1 envelope carried flat `scheme` and `network` fields.
        // v2 replaces both with `accepted`, holding the whole leg. A v1
        // envelope reaching a v2 node gets a second 402 with no reason,
        // which is the defect this test pins.
        let value = decode(&prepare_from_real_header().header_value);

        assert_eq!(value["x402Version"], 2);
        assert!(
            value.get("scheme").is_none(),
            "scheme is a v1 field and must not be sent"
        );
        assert!(
            value.get("network").is_none(),
            "network is a v1 field and must not be sent"
        );
        assert!(
            value.get("accepted").is_some(),
            "accepted is required in v2"
        );
        assert!(value.get("extensions").is_some());
        assert_eq!(value["payload"]["authorization"]["value"], "10000");
        assert_eq!(
            value["payload"]["authorization"]["to"],
            "0x5a2324aa18613fad4e44bdf0d6c73ec1f6d87ff8"
        );
    }

    #[test]
    fn the_accepted_leg_is_echoed_back_exactly_as_it_arrived() {
        // Re-serialising from the typed Accept would drop any field
        // this client does not model, and the node validates what it
        // sent. So the echo has to be the raw JSON.
        let challenge = parse_header(REAL_HEADER).expect("the header must parse");
        let index = challenge
            .eip155_leg_index(84532)
            .expect("the leg must be there");
        let original = challenge.raw_leg(index).expect("the raw leg must be there");

        let value = decode(&prepare_from_real_header().header_value);
        assert_eq!(&value["accepted"], original);
    }

    #[test]
    fn the_resource_object_is_echoed_when_the_challenge_carries_one() {
        let challenge = parse_header(REAL_HEADER).expect("the header must parse");
        let resource = challenge.raw_resource().expect("this challenge has one");
        let value = decode(&prepare_from_real_header().header_value);
        assert_eq!(&value["resource"], resource);
    }

    #[test]
    fn a_challenge_without_a_resource_omits_the_field_rather_than_sending_null() {
        let mut challenge = parse_header(REAL_HEADER).expect("the header must parse");
        challenge
            .raw
            .as_object_mut()
            .expect("the challenge is an object")
            .remove("resource");
        let signer = Signer::from_hex(TEST_KEY).expect("the test key must parse");
        let index = challenge
            .eip155_leg_index(84532)
            .expect("the leg must be there");
        let payment =
            prepare_payment(&challenge, index, &signer, b"test").expect("the payment must prepare");

        let value = decode(&payment.header_value);
        assert!(
            value.get("resource").is_none(),
            "a missing resource must be omitted, never sent as null"
        );
    }

    #[test]
    fn the_header_never_holds_the_private_key() {
        let payment = prepare_from_real_header();
        let bare_key = TEST_KEY.trim_start_matches("0x");
        assert!(
            !payment.header_value.contains(bare_key),
            "the payment header leaked the key"
        );
        let raw = base64_decode(&payment.header_value).expect("the header must be base64");
        let text = String::from_utf8(raw).expect("the payload must be UTF-8");
        assert!(!text.contains(bare_key), "the payload leaked the key");
    }

    #[test]
    fn a_successful_settlement_header_decodes() {
        let document = r#"{"success":true,"transaction":"0xabc","network":"eip155:84532",
             "payer":"0x2c7536e3605d9c16a7a3d7b1898e529396a65c23"}"#;
        let encoded = base64_encode(document.as_bytes());
        let settlement = Settlement::from_header(&encoded).expect("it must decode");
        assert_eq!(settlement.success, Some(true));
        assert_eq!(settlement.transaction.as_deref(), Some("0xabc"));
        assert_eq!(settlement.network.as_deref(), Some("eip155:84532"));
        assert!(settlement.error_reason.is_none());
    }

    #[test]
    fn a_failed_settlement_header_states_its_reason() {
        // This is the whole point of reading the header: a refusal that
        // says why, instead of an opaque non-200.
        let document = r#"{"success":false,"errorReason":"insufficient_funds"}"#;
        let encoded = base64_encode(document.as_bytes());
        let settlement = Settlement::from_header(&encoded).expect("it must decode");
        assert_eq!(settlement.success, Some(false));
        assert_eq!(
            settlement.error_reason.as_deref(),
            Some("insufficient_funds")
        );
    }

    #[test]
    fn a_settlement_header_that_is_not_base64_is_an_error_not_a_panic() {
        assert!(Settlement::from_header("not base64 !!!").is_err());
    }

    #[test]
    fn a_settlement_header_that_is_not_json_is_an_error_not_a_panic() {
        let encoded = base64_encode(b"this is not json");
        assert!(Settlement::from_header(&encoded).is_err());
    }

    #[test]
    fn the_ask_body_is_the_documented_auto_routed_shape() {
        // The documented auto-routed body has ONE required field.
        // Sending question/intent got a 400 AFTER payment passed, which
        // is the most misleading failure shape available.
        let body = ask_body("What is the weather in Tokyo?");
        assert_eq!(body["query"], "What is the weather in Tokyo?");
        assert!(
            body.get("question").is_none(),
            "question is not a field of this endpoint"
        );
        assert!(
            body.get("intent").is_none(),
            "the router classifies the intent; the client must not send one"
        );
        assert_eq!(
            body.as_object().expect("an object").len(),
            1,
            "the auto-routed body carries query and nothing else"
        );
    }

    #[test]
    fn the_v2_header_names_are_not_the_v1_names() {
        // A rename back to the v1 names is the exact defect that cost a
        // failed ask, so it is pinned.
        assert_eq!(PAYMENT_HEADER, "PAYMENT-SIGNATURE");
        assert_eq!(SETTLEMENT_HEADER, "PAYMENT-RESPONSE");
        assert_ne!(PAYMENT_HEADER, "X-PAYMENT");
    }
}
