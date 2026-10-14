//! **Smart-contract wallets, checked by their own contract.**
//!
//! A plain key's signature is arithmetic (`auth.rs`). A smart wallet's is
//! not: Safe, Coinbase Smart Wallet, Ambire, and the EIP-7702 wallets most
//! ELO holders carry are checked by the wallet's own contract (ERC-1271), or,
//! while one is not deployed on a chain yet, by simulating that deployment
//! first (ERC-6492). Both need the chain, so this is the one place the shard
//! asks it about a signature, and it asks only after the key check said no.
//!
//! The question is one `eth_call` of the ERC-6492 validator: deployless
//! bytecode that checks a plain key, a deployed smart wallet and an
//! undeployed one in the same call, and changes nothing on chain. It is viem
//! 2.56.9's `erc6492SignatureValidatorByteCode` byte for byte, the same bytes
//! the platform asks with (`meter/sigcheck.py` in AnthonE/scry-forge); the
//! test below pins its hash so the two copies cannot drift apart quietly.
//!
//! **Off unless `shard.toml` names RPCs** (`sig_rpc`), so a shard that names
//! none keeps the no-network handshake it always had. The public shard names
//! Robinhood Chain, then Base and Ethereum: a smart wallet connected on
//! another chain signs for that chain. Bounded three ways, because the bytes
//! are a stranger's: at most [`IN_FLIGHT`] checks at once, a timeout per call,
//! and a deadline on the whole.

use std::sync::Arc;
use std::time::Duration;

use protocol::Address;
use tokio::sync::Semaphore;

/// Most RPCs `sig_rpc` may name: each is an `eth_call` a refused stranger
/// costs us.
pub const MAX_RPCS: usize = 4;
/// Checks in flight at once, shard-wide. A fifth waits briefly, then is
/// told to try again rather than queueing behind noise.
pub const IN_FLIGHT: usize = 4;
const WAIT_FOR_SLOT: Duration = Duration::from_secs(2);
const CALL_TIMEOUT: Duration = Duration::from_secs(4);
const DEADLINE: Duration = Duration::from_secs(10);
/// An `eth_call` answer is a few dozen bytes; a body past this is not one.
const MAX_RESPONSE_BYTES: usize = 4096;
/// Some RPC edges refuse a library's default agent.
const USER_AGENT: &str = "gates-shard (+https://elopros.com)";

/// The ERC-6492 validator's creation code, hex, without `0x`.
const VALIDATOR: &[&str] = &[
    "608060405234801561001057600080fd5b5060405161069438038061069483398101604081905261002f9161051e565b",
    "600061003c848484610048565b9050806000526001601ff35b60007f6492649264926492649264926492649264926492",
    "6492649264926492649264926100748361040c565b036101e75760006060808480602001905181019061009291906105",
    "77565b60405192955090935091506000906001600160a01b038516906100b69085906105dd565b600060405180830381",
    "6000865af19150503d80600081146100f3576040519150601f19603f3d011682016040523d82523d6000602084013e61",
    "00f8565b606091505b50509050876001600160a01b03163b60000361016057806101605760405162461bcd60e51b8152",
    "60206004820152601e60248201527f5369676e617475726556616c696461746f723a206465706c6f796d656e74000060",
    "448201526064015b60405180910390fd5b604051630b135d3f60e11b808252906001600160a01b038a1690631626ba7e",
    "90610190908b9087906004016105f9565b602060405180830381865afa1580156101ad573d6000803e3d6000fd5b5050",
    "50506040513d601f19601f820116820180604052508101906101d19190610633565b6001600160e01b03191614945050",
    "505050610405565b6001600160a01b0384163b1561027a57604051630b135d3f60e11b808252906001600160a01b0386",
    "1690631626ba7e9061022790879087906004016105f9565b602060405180830381865afa158015610244573d6000803e",
    "3d6000fd5b505050506040513d601f19601f820116820180604052508101906102689190610633565b6001600160e01b",
    "031916149050610405565b81516041146102df5760405162461bcd60e51b815260206004820152603a60248201526000",
    "8051602061067483398151915260448201527f3a20696e76616c6964207369676e6174757265206c656e677468000000",
    "0000006064820152608401610157565b6102e7610425565b506020820151604080840151845185939260009185919081",
    "1061030c5761030c61065d565b016020015160f81c9050601b811480159061032b57508060ff16601c14155b1561038c",
    "5760405162461bcd60e51b815260206004820152603b602482015260008051602061067483398151915260448201527f",
    "3a20696e76616c6964207369676e617475726520762076616c756500000000006064820152608401610157565b604080",
    "51600081526020810180835289905260ff83169181019190915260608101849052608081018390526001600160a01b03",
    "89169060019060a0016020604051602081039080840390855afa1580156103ea573d6000803e3d6000fd5b5050506020",
    "60405103516001600160a01b0316149450505050505b9392505050565b600060208251101561041d57600080fd5b5080",
    "51015190565b60405180606001604052806003906020820280368337509192915050565b6001600160a01b0381168114",
    "61045857600080fd5b50565b634e487b7160e01b600052604160045260246000fd5b60005b8381101561048c57818101",
    "5183820152602001610474565b50506000910152565b600082601f8301126104a657600080fd5b81516001600160401b",
    "038111156104bf576104bf61045b565b604051601f8201601f19908116603f011681016001600160401b038111828210",
    "17156104ed576104ed61045b565b60405281815283820160200185101561050557600080fd5b61051682602083016020",
    "8701610471565b949350505050565b60008060006060848603121561053357600080fd5b835161053e81610443565b60",
    "20850151604086015191945092506001600160401b0381111561056157600080fd5b61056d86828701610495565b9150",
    "509250925092565b60008060006060848603121561058c57600080fd5b835161059781610443565b6020850151909350",
    "6001600160401b038111156105b357600080fd5b6105bf86828701610495565b60408601519093509050600160016040",
    "1b0381111561056157600080fd5b600082516105ef818460208701610471565b9190910192915050565b828152604060",
    "208201526000825180604084015261061e816060850160208701610471565b601f01601f191691909101606001939250",
    "5050565b60006020828403121561064557600080fd5b81516001600160e01b03198116811461040557600080fd5b634e",
    "487b7160e01b600052603260045260246000fdfe5369676e617475726556616c696461746f72237265636f7665725369",
    "676e6572",
];

/// Where a shard asks, and the gate in front of it. `Clone` is cheap: every
/// handshake task holds one.
#[derive(Clone)]
pub struct Config {
    rpcs: Arc<[String]>,
    gate: Arc<Semaphore>,
}

impl Config {
    /// No chain: a smart wallet's signature is refused, as before v109.
    pub fn off() -> Self {
        Self::new(Vec::new())
    }

    pub fn new(rpcs: Vec<String>) -> Self {
        Self {
            rpcs: rpcs.into(),
            gate: Arc::new(Semaphore::new(IN_FLIGHT)),
        }
    }

    pub fn armed(&self) -> bool {
        !self.rpcs.is_empty()
    }

    pub fn rpcs(&self) -> &[String] {
        &self.rpcs
    }
}

impl core::fmt::Debug for Config {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("chainsig::Config")
            .field("rpcs", &self.rpcs)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// A chain's validator said this is the wallet's signature.
    Valid,
    /// Every chain that answered said no.
    Invalid,
    /// No chain answered (or the gate was full): not a no, a try-again.
    Unreachable,
}

/// Ask the chains whether `sig` is `address`'s signature over any of
/// `digests` (one per name the shard answers to, `auth::digests`).
pub async fn check(
    cfg: &Config,
    address: Address,
    digests: Vec<[u8; 32]>,
    sig: Vec<u8>,
) -> Verdict {
    if !cfg.armed() || digests.is_empty() || sig.is_empty() {
        return Verdict::Invalid;
    }
    let Ok(Ok(permit)) =
        tokio::time::timeout(WAIT_FOR_SLOT, cfg.gate.clone().acquire_owned()).await
    else {
        return Verdict::Unreachable;
    };
    let rpcs = cfg.rpcs.clone();
    // `ureq` is synchronous, so the calls run on a blocking thread: a slow
    // RPC must not stall the handshakes sharing this worker (`entitle.rs`'s
    // reason, same tool).
    let work = tokio::task::spawn_blocking(move || {
        let _held = permit;
        ask_each(&rpcs, &address, &digests, &sig, post)
    });
    match tokio::time::timeout(DEADLINE, work).await {
        Ok(Ok(v)) => v,
        _ => Verdict::Unreachable,
    }
}

/// Each RPC in order, each digest in order; the first yes wins. An RPC that
/// did not answer is skipped whole rather than asked once per digest.
pub(crate) fn ask_each(
    rpcs: &[String],
    address: &Address,
    digests: &[[u8; 32]],
    sig: &[u8],
    post: impl Fn(&str, &str) -> Option<String>,
) -> Verdict {
    let mut answered = false;
    for url in rpcs {
        for d in digests {
            match post(url, &request(address, d, sig))
                .as_deref()
                .and_then(answer)
            {
                Some(true) => return Verdict::Valid,
                Some(false) => answered = true,
                None => break,
            }
        }
    }
    if answered {
        Verdict::Invalid
    } else {
        Verdict::Unreachable
    }
}

/// The JSON-RPC body: `eth_call` with no `to`, data = the validator's
/// creation code followed by its constructor arguments,
/// `abi.encode(address signer, bytes32 hash, bytes signature)`.
pub(crate) fn request(address: &Address, digest: &[u8; 32], sig: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(200 + VALIDATOR.len() * 96 + 256 + sig.len() * 2);
    s.push_str(r#"{"jsonrpc":"2.0","id":1,"method":"eth_call","params":[{"data":"0x"#);
    for part in VALIDATOR {
        s.push_str(part);
    }
    let mut push = |b: &[u8]| {
        for x in b {
            s.push(HEX[(x >> 4) as usize] as char);
            s.push(HEX[(x & 0xf) as usize] as char);
        }
    };
    let mut word = [0u8; 32];
    word[12..].copy_from_slice(&address.0);
    push(&word); // signer
    push(digest); // hash
    let mut word = [0u8; 32];
    word[31] = 0x60;
    push(&word); // where the bytes start: after these three words
    let mut word = [0u8; 32];
    word[24..].copy_from_slice(&(sig.len() as u64).to_be_bytes());
    push(&word); // their length
    push(sig);
    push(&vec![0u8; (32 - sig.len() % 32) % 32]); // padded to a word
    s.push_str(r#""},"latest"]}"#);
    s
}

/// `Some(true)` for a yes, `Some(false)` for a no (a zero, or a revert, which
/// is the validator refusing), `None` for anything that is not an answer.
pub(crate) fn answer(body: &str) -> Option<bool> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    if let Some(err) = v.get("error") {
        let code = err.get("code").and_then(|c| c.as_i64());
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        return (code == Some(3) || msg.contains("revert")).then_some(false);
    }
    let r = v.get("result")?.as_str()?;
    let hex = r.strip_prefix("0x")?;
    if hex.is_empty() {
        return Some(false);
    }
    Some(hex.trim_start_matches('0') == "1")
}

fn post(url: &str, body: &str) -> Option<String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(CALL_TIMEOUT))
        .build()
        .into();
    let mut res = agent
        .post(url)
        .header("user-agent", USER_AGENT)
        .content_type("application/json")
        .send(body)
        .ok()?;
    res.body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES as u64)
        .read_to_string()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// An undeployed Coinbase Smart Wallet on Robinhood Chain whose owner is
    /// a key derived from a fixed phrase, and its ERC-6492 signature over
    /// `FIXTURE_DIGEST` — made and checked against the chain with the
    /// platform's `sigcheck.py`. Read by the live test only.
    const FIXTURE_ACCOUNT: &str = "d0521b67128afa05f2f105c18e61e7c7f97109a6";
    const FIXTURE_DIGEST: &str = "e4914200a19eb406152a1e3e0f8f17aeb99c15ab5afe456f14b45725023a28c0";
    const FIXTURE_SIG: &str = "0000000000000000000000000ba5ed0c6aa8c49038f819e587e2633c4a9f428a0000000000000000000000000000000000000000000000000000000000000060000000000000000000000000000000000000000000000000000000000000016000000000000000000000000000000000000000000000000000000000000000c43ffba36f000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000200000000000000000000000008994a5294271f50386ea5faca80162cde473ec790000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000e0000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000004188ddab7a051b23ee93df8f78f5307562f83eea6be57e0b51b9d46b52b1a489086fcdeb6a86cca16908770b0e4a1189a219ceeb87d8d0258979813686d66c1e681c000000000000000000000000000000000000000000000000000000000000006492649264926492649264926492649264926492649264926492649264926492";

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
            .collect()
    }

    fn fixture() -> (Address, [u8; 32], Vec<u8>) {
        let mut a = [0u8; 20];
        a.copy_from_slice(&unhex(FIXTURE_ACCOUNT));
        let mut d = [0u8; 32];
        d.copy_from_slice(&unhex(FIXTURE_DIGEST));
        (Address(a), d, unhex(FIXTURE_SIG))
    }

    /// The platform's copy and this one are the same bytes: `sigcheck.py`
    /// pins keccak256 of the same hex text.
    #[test]
    fn the_validator_is_viems_byte_for_byte() {
        use tiny_keccak::{Hasher, Keccak};
        let mut k = Keccak::v256();
        for part in VALIDATOR {
            k.update(part.as_bytes());
        }
        let mut out = [0u8; 32];
        k.finalize(&mut out);
        let got: String = out.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            got,
            "0c6ba7a6d39e479d663e00ca084c478da90421378d518e673939f1870060e6db"
        );
        assert_eq!(VALIDATOR.iter().map(|p| p.len()).sum::<usize>(), 1684 * 2);
    }

    /// The constructor arguments are `abi.encode(address, bytes32, bytes)`,
    /// word for word, and the bytes are padded to a word.
    #[test]
    fn the_request_is_the_validator_and_its_three_arguments() {
        let (a, d, sig) = fixture();
        let body = request(&a, &d, &sig);
        let data = body
            .split("\"data\":\"0x")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .expect("data");
        let code: String = VALIDATOR.concat();
        assert!(data.starts_with(&code));
        let args = &data[code.len()..];
        assert_eq!(&args[..64], &format!("{:0>64}", FIXTURE_ACCOUNT));
        assert_eq!(&args[64..128], FIXTURE_DIGEST);
        assert_eq!(&args[128..192], &format!("{:0>64}", "60"));
        assert_eq!(&args[192..256], &format!("{:0>64x}", sig.len()));
        assert_eq!(args.len(), 256 + sig.len().div_ceil(32) * 64);
        assert!(body.ends_with(r#""},"latest"]}"#));
    }

    #[test]
    fn answers_are_read_strictly() {
        assert_eq!(answer(r#"{"result":"0x01"}"#), Some(true));
        assert_eq!(
            answer(
                r#"{"result":"0x0000000000000000000000000000000000000000000000000000000000000001"}"#
            ),
            Some(true)
        );
        assert_eq!(answer(r#"{"result":"0x00"}"#), Some(false));
        assert_eq!(answer(r#"{"result":"0x"}"#), Some(false));
        assert_eq!(answer(r#"{"result":"0x02"}"#), Some(false));
        assert_eq!(
            answer(r#"{"error":{"code":3,"message":"execution reverted"}}"#),
            Some(false)
        );
        assert_eq!(
            answer(r#"{"error":{"code":-32005,"message":"rate limited"}}"#),
            None
        );
        assert_eq!(answer("<html>403</html>"), None);
        assert_eq!(answer(r#"{"result":7}"#), None);
    }

    /// First yes wins; an RPC that does not answer is skipped whole; "no"
    /// only when somebody answered.
    #[test]
    fn each_rpc_is_asked_in_order() {
        let (a, d, sig) = fixture();
        let rpcs: Vec<String> = ["rh", "base", "eth"].map(String::from).to_vec();
        let asked = RefCell::new(Vec::new());
        let script = |answers: &'static [(&'static str, Option<&'static str>)]| {
            let asked = &asked;
            move |url: &str, _: &str| {
                asked.borrow_mut().push(url.to_string());
                answers
                    .iter()
                    .find(|(u, _)| *u == url)
                    .and_then(|(_, b)| b.map(String::from))
            }
        };
        let two = [d, d];

        asked.borrow_mut().clear();
        let v = ask_each(
            &rpcs,
            &a,
            &[d],
            &sig,
            script(&[("rh", Some(r#"{"result":"0x01"}"#))]),
        );
        assert_eq!(
            (v, asked.borrow().len()),
            (Verdict::Valid, 1),
            "Robinhood Chain says yes: done"
        );

        asked.borrow_mut().clear();
        let v = ask_each(
            &rpcs,
            &a,
            &two,
            &sig,
            script(&[
                ("rh", None),
                ("base", Some(r#"{"result":"0x00"}"#)),
                ("eth", Some(r#"{"result":"0x01"}"#)),
            ]),
        );
        assert_eq!(v, Verdict::Valid);
        assert_eq!(
            *asked.borrow(),
            ["rh", "base", "base", "eth"],
            "a dead RPC is asked once, not per digest"
        );

        asked.borrow_mut().clear();
        let v = ask_each(
            &rpcs,
            &a,
            &[d],
            &sig,
            script(&[("rh", Some(r#"{"result":"0x00"}"#))]),
        );
        assert_eq!(v, Verdict::Invalid, "one no and two silences is a no");

        let v = ask_each(&rpcs, &a, &[d], &sig, script(&[]));
        assert_eq!(
            v,
            Verdict::Unreachable,
            "nobody answered: try again, never 'wrong signer'"
        );
    }

    #[tokio::test]
    async fn an_unarmed_shard_never_asks() {
        let (a, d, sig) = fixture();
        assert_eq!(
            check(&Config::off(), a, vec![d], sig).await,
            Verdict::Invalid
        );
    }

    /// The real thing: an undeployed Coinbase Smart Wallet, checked by
    /// Robinhood Chain, and the same signature refused for another digest.
    /// `cargo test -p server chainsig -- --ignored` (needs the network).
    #[tokio::test]
    #[ignore]
    async fn live_robinhood_chain() {
        let cfg = Config::new(vec!["https://rpc.mainnet.chain.robinhood.com".into()]);
        let (a, d, sig) = fixture();
        assert_eq!(check(&cfg, a, vec![d], sig.clone()).await, Verdict::Valid);
        let mut other = d;
        other[0] ^= 1;
        assert_eq!(check(&cfg, a, vec![other], sig).await, Verdict::Invalid);
    }
}
