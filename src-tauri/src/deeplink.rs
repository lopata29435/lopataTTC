use anyhow::{anyhow, bail, Result};
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use serde_json::Value as JsonValue;
use std::collections::HashMap;

use crate::profiles::Profile;

/// Parse a `tt://?<payload>` deep-link URI into a Profile.
///
/// The TrustTunnel `tt://` deep-link payload format is not publicly documented in
/// detail; we try multiple decoders and report which ones failed in case nothing matches.
pub fn parse_tt_uri(uri: &str) -> Result<Profile> {
    let stripped = uri
        .strip_prefix("tt://")
        .ok_or_else(|| anyhow!("URI must start with tt:// (got: {})", short(uri, 60)))?;

    // accept "tt://?xxx" and "tt://xxx" and "tt:///?xxx"
    let payload = stripped.trim_start_matches('/').trim_start_matches('?');

    if payload.is_empty() {
        bail!("Пустой payload после tt://");
    }

    let mut tried: Vec<String> = Vec::new();

    if let Some(profile) = try_decode_payload(payload, &mut tried) {
        return Ok(profile);
    }

    // also try as query parameters (k=v&k=v)
    let params = parse_query(payload);
    if !params.is_empty() {
        tried.push(format!(
            "query params: {:?}",
            params.keys().collect::<Vec<_>>()
        ));
        for key in ["config", "c", "data", "payload"] {
            if let Some(val) = params.get(key) {
                if let Some(p) = try_decode_payload(val, &mut tried) {
                    return Ok(p);
                }
            }
        }
        if let Some(p) = profile_from_params(&params) {
            return Ok(p);
        }
    }

    bail!(
        "Не удалось декодировать tt:// payload.\n\nPayload (preview): {}\nДлина: {} симв.\n\nПопытки:\n - {}\n\nПришли эту ссылку в чат, я подгоню парсер под формат.",
        short(payload, 120),
        payload.len(),
        tried.join("\n - "),
    );
}

fn short(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n).collect();
        out.push('…');
        out
    }
}

fn try_decode_payload(payload: &str, tried: &mut Vec<String>) -> Option<Profile> {
    let url_decoded = urlencoding::decode(payload)
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| payload.to_string());

    if let Some(p) = parse_blob_as_profile(url_decoded.as_bytes()) {
        return Some(p);
    }
    tried.push("url-decoded → JSON/TOML: не подошло".to_string());

    macro_rules! try_engine {
        ($name:expr, $engine:expr, $input:expr) => {
            match $engine.decode($input.as_bytes()) {
                Ok(bytes) => {
                    if let Some(p) = parse_blob_as_profile(&bytes) {
                        return Some(p);
                    }
                    tried.push(format!(
                        "{}: декодировал {} байт, но это не JSON/TOML",
                        $name,
                        bytes.len()
                    ));
                }
                Err(e) => {
                    tried.push(format!("{}: {}", $name, e));
                }
            }
        };
    }

    try_engine!("base64 STANDARD (url-decoded)", &STANDARD, url_decoded);
    try_engine!("base64 URL_SAFE (url-decoded)", &URL_SAFE, url_decoded);
    try_engine!(
        "base64 STANDARD_NO_PAD (url-decoded)",
        &STANDARD_NO_PAD,
        url_decoded
    );
    try_engine!(
        "base64 URL_SAFE_NO_PAD (url-decoded)",
        &URL_SAFE_NO_PAD,
        url_decoded
    );

    if url_decoded != payload {
        try_engine!("base64 STANDARD (raw)", &STANDARD, payload);
        try_engine!("base64 URL_SAFE (raw)", &URL_SAFE, payload);
        try_engine!("base64 STANDARD_NO_PAD (raw)", &STANDARD_NO_PAD, payload);
        try_engine!("base64 URL_SAFE_NO_PAD (raw)", &URL_SAFE_NO_PAD, payload);
    }

    None
}

fn parse_blob_as_profile(bytes: &[u8]) -> Option<Profile> {
    // First try the binary AdGuard TrustTunnel format.
    if let Ok(p) = parse_adguard_binary(bytes) {
        if !p.hostname.is_empty() {
            return Some(p);
        }
    }
    // Then try JSON
    if let Ok(text) = std::str::from_utf8(bytes) {
        if let Ok(v) = serde_json::from_str::<JsonValue>(text) {
            if let Some(p) = profile_from_json(&v) {
                return Some(p);
            }
        }
        // Try TOML
        if let Ok(p) = Profile::from_toml_str(text, "Imported") {
            if !p.hostname.is_empty() {
                return Some(p);
            }
        }
    }
    None
}

/// Parse AdGuard TrustTunnel binary deeplink payload.
/// Format (observed): `00 01 <field>*` where each field is one of:
///   tag 0x01 (hostname list): `<count> [<len><utf8>]*`
///   tag 0x02 (address):       `<len><utf8>`        — may repeat for multi-address
///   tag 0x05 (username):      `<len><utf8>`
///   tag 0x06 (password):      `<len><utf8>`
///   tag 0x0C (display name):  `<len><utf8>`
///   tag 0x0D (dns upstreams): `<total_len> [<len><utf8>]*`
///   unknown tags are skipped as `<len><value>`.
fn parse_adguard_binary(bytes: &[u8]) -> Result<Profile> {
    if bytes.len() < 3 {
        bail!("payload too short");
    }
    if bytes[0] != 0x00 || bytes[1] != 0x01 {
        bail!("missing magic 00 01");
    }
    let mut i: usize = 2;

    let mut profile = Profile::new_blank("Imported");

    fn read_str(bytes: &[u8], i: &mut usize) -> Result<String> {
        if *i >= bytes.len() {
            bail!("truncated: expected length byte");
        }
        let len = bytes[*i] as usize;
        *i += 1;
        if *i + len > bytes.len() {
            bail!(
                "truncated: length {} exceeds remaining {}",
                len,
                bytes.len() - *i
            );
        }
        let s = std::str::from_utf8(&bytes[*i..*i + len])
            .map_err(|e| anyhow!("invalid utf-8: {}", e))?
            .to_string();
        *i += len;
        Ok(s)
    }

    while i < bytes.len() {
        let tag = bytes[i];
        i += 1;
        match tag {
            0x01 => {
                // count + list of strings
                if i >= bytes.len() {
                    bail!("truncated at field 0x01");
                }
                let count = bytes[i] as usize;
                i += 1;
                for n in 0..count {
                    let s = read_str(bytes, &mut i)?;
                    if n == 0 {
                        profile.hostname = s;
                    }
                    // additional hostnames could be stored if the protocol supports them
                }
            }
            0x02 => {
                let s = read_str(bytes, &mut i)?;
                profile.addresses.push(s);
            }
            0x05 => {
                profile.username = read_str(bytes, &mut i)?;
            }
            0x06 => {
                profile.password = read_str(bytes, &mut i)?;
            }
            0x0C => {
                profile.name = read_str(bytes, &mut i)?;
            }
            0x0D => {
                if i >= bytes.len() {
                    bail!("truncated at field 0x0D");
                }
                let total = bytes[i] as usize;
                i += 1;
                let end = i + total;
                if end > bytes.len() {
                    bail!("0x0D total exceeds buffer");
                }
                let mut dns = Vec::new();
                while i < end {
                    let s = read_str(bytes, &mut i)?;
                    dns.push(s);
                }
                if !dns.is_empty() {
                    profile.dns_upstreams = dns;
                }
            }
            _ => {
                // Unknown tag — skip as length-prefixed value.
                if i >= bytes.len() {
                    break;
                }
                let len = bytes[i] as usize;
                i += 1;
                if i + len > bytes.len() {
                    bail!(
                        "unknown tag 0x{:02X} has length {} exceeding buffer",
                        tag,
                        len
                    );
                }
                i += len;
            }
        }
    }

    if profile.hostname.is_empty() {
        bail!("hostname is empty after parsing");
    }
    // If no addresses parsed, default to "<hostname>:443" — TrustTunnel default.
    if profile.addresses.is_empty() {
        profile.addresses.push(format!("{}:443", profile.hostname));
    }
    Ok(profile)
}

fn profile_from_json(v: &JsonValue) -> Option<Profile> {
    let obj = v.as_object()?;
    // The endpoint fields may be nested under "endpoint" or at the top.
    let endpoint_obj = obj
        .get("endpoint")
        .and_then(|x| x.as_object())
        .unwrap_or(obj);

    let s = |k: &str| {
        endpoint_obj
            .get(k)
            .and_then(|x| x.as_str())
            .map(String::from)
    };
    let b = |k: &str| endpoint_obj.get(k).and_then(|x| x.as_bool());
    let arr = |k: &str| {
        endpoint_obj
            .get(k)
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };

    let hostname = s("hostname").or_else(|| s("host")).unwrap_or_default();
    if hostname.is_empty() {
        return None;
    }

    let name = obj
        .get("name")
        .and_then(|x| x.as_str())
        .map(String::from)
        .or_else(|| {
            endpoint_obj
                .get("name")
                .and_then(|x| x.as_str())
                .map(String::from)
        })
        .unwrap_or_else(|| hostname.clone());

    let mut profile = Profile::new_blank(&name);
    profile.hostname = hostname;
    profile.addresses = arr("addresses");
    profile.username = s("username").or_else(|| s("user")).unwrap_or_default();
    profile.password = s("password").or_else(|| s("pass")).unwrap_or_default();
    profile.custom_sni = s("custom_sni").or_else(|| s("sni")).unwrap_or_default();
    if let Some(v) = b("has_ipv6") {
        profile.has_ipv6 = v;
    }
    profile.upstream_protocol = s("upstream_protocol").unwrap_or_else(|| "http2".into());
    if let Some(v) = b("anti_dpi") {
        profile.anti_dpi = v;
    }
    let dns = arr("dns_upstreams");
    if !dns.is_empty() {
        profile.dns_upstreams = dns;
    }
    if let Some(v) = b("skip_verification") {
        profile.skip_verification = v;
    }
    Some(profile)
}

fn profile_from_params(params: &HashMap<String, String>) -> Option<Profile> {
    let hostname = params
        .get("hostname")
        .or_else(|| params.get("host"))?
        .clone();
    let name = params
        .get("name")
        .cloned()
        .unwrap_or_else(|| hostname.clone());
    let mut profile = Profile::new_blank(&name);
    profile.hostname = hostname;
    if let Some(p) = params.get("port") {
        profile.addresses = vec![format!("{}:{}", profile.hostname, p)];
    }
    if let Some(u) = params.get("username").or_else(|| params.get("user")) {
        profile.username = u.clone();
    }
    if let Some(p) = params.get("password").or_else(|| params.get("pass")) {
        profile.password = p.clone();
    }
    if let Some(s) = params.get("sni").or_else(|| params.get("custom_sni")) {
        profile.custom_sni = s.clone();
    }
    if let Some(p) = params
        .get("protocol")
        .or_else(|| params.get("upstream_protocol"))
    {
        profile.upstream_protocol = p.clone();
    }
    Some(profile)
}

fn parse_query(q: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for pair in q.split('&') {
        if pair.is_empty() {
            continue;
        }
        let mut it = pair.splitn(2, '=');
        let k = it.next().unwrap_or("");
        let v = it.next().unwrap_or("");
        let k = urlencoding::decode(k)
            .map(|c| c.into_owned())
            .unwrap_or_else(|_| k.to_string());
        let v = urlencoding::decode(v)
            .map(|c| c.into_owned())
            .unwrap_or_else(|_| v.to_string());
        if !k.is_empty() {
            out.insert(k, v);
        }
    }
    out
}

/// Look for tt:// URIs in a free-form text blob (e.g. clipboard contents).
pub fn extract_tt_uri(text: &str) -> Option<String> {
    for token in text.split(|c: char| c.is_whitespace() || c == '"' || c == '\'') {
        if token.starts_with("tt://") {
            return Some(token.to_string());
        }
    }
    None
}
