//! Hash chain over gate.json: each stored receipt links to the one before it, and the
//! checkpoint links to the last receipt and to the contract file's digest.

use serde_json::{Map, Value};

use crate::sha256;

const GENESIS: &str = "genesis";

const CHECKPOINT_FIELDS: [&str; 8] = [
    "state",
    "candidate",
    "open_findings",
    "consumed_repair_rounds",
    "next_action",
    "continuations",
    "handback_refusals",
    "contract_id",
];

const ADDED_FIELDS: [&str; 2] = ["sessions", "accepted_candidate"];

/// Compact JSON with object keys sorted recursively. Written out rather than relying on
/// serde_json's map order, which a `preserve_order` feature elsewhere in the build would change.
pub fn canonical(v: &Value) -> String {
    match v {
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            let body: Vec<String> = entries
                .into_iter()
                .map(|(k, v)| format!("{}:{}", Value::from(k.as_str()), canonical(v)))
                .collect();
            format!("{{{}}}", body.join(","))
        }
        Value::Array(items) => {
            let body: Vec<String> = items.iter().map(canonical).collect();
            format!("[{}]", body.join(","))
        }
        scalar => scalar.to_string(),
    }
}

fn link(prev: &str, v: &Value) -> String {
    sha256::hex(format!("{prev}{}", canonical(v)).as_bytes())
}

fn checkpoint_input(gate: &Value, contract_sha256: &str) -> Value {
    let mut map: Map<String, Value> = CHECKPOINT_FIELDS
        .iter()
        .map(|k| {
            (
                (*k).to_owned(),
                gate.get(*k).cloned().unwrap_or(Value::Null),
            )
        })
        .collect();
    // Fields added after the chain existed: covered when present (save_gate always writes
    // them), skipped when absent so a gate.json from before them still verifies.
    for k in ADDED_FIELDS {
        if let Some(v) = gate.get(k) {
            map.insert(k.to_owned(), v.clone());
        }
    }
    map.insert("contract_sha256".to_owned(), contract_sha256.into());
    Value::Object(map)
}

fn without_links(receipt: &Value) -> Value {
    let mut body = receipt.clone();
    if let Some(o) = body.as_object_mut() {
        o.remove("prev");
        o.remove("hash");
    }
    body
}

/// Add `prev`/`hash` to every receipt and `checkpoint_hash` to the gate object.
pub(crate) fn seal(gate: &mut Value, contract_sha256: &str) {
    let mut prev = GENESIS.to_owned();
    if let Some(receipts) = gate.get_mut("receipts").and_then(Value::as_array_mut) {
        for receipt in receipts {
            let hash = link(&prev, &without_links(receipt));
            if let Some(o) = receipt.as_object_mut() {
                o.insert("prev".to_owned(), prev.clone().into());
                o.insert("hash".to_owned(), hash.clone().into());
            }
            prev = hash;
        }
    }
    let hash = link(&prev, &checkpoint_input(gate, contract_sha256));
    if let Some(o) = gate.as_object_mut() {
        o.insert("checkpoint_hash".to_owned(), hash.into());
    }
}

/// `Err(what)` names the first broken link: `receipt <index>` or `checkpoint`.
pub(crate) fn verify(gate: &Value, contract_sha256: &str) -> Result<(), String> {
    let receipts = gate
        .get("receipts")
        .and_then(Value::as_array)
        .ok_or("checkpoint")?;
    let mut prev = GENESIS.to_owned();
    for (i, receipt) in receipts.iter().enumerate() {
        let stored_prev = receipt.get("prev").and_then(Value::as_str);
        let stored_hash = receipt.get("hash").and_then(Value::as_str);
        match (stored_prev, stored_hash) {
            (Some(p), Some(h)) if p == prev && link(&prev, &without_links(receipt)) == h => {
                prev = h.to_owned();
            }
            _ => return Err(format!("receipt {i}")),
        }
    }
    match gate.get("checkpoint_hash").and_then(Value::as_str) {
        Some(h) if h == link(&prev, &checkpoint_input(gate, contract_sha256)) => Ok(()),
        _ => Err("checkpoint".to_owned()),
    }
}
