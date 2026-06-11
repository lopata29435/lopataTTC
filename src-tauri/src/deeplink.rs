use anyhow::{anyhow, bail, Result};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;

use crate::profiles::Profile;

// Parser for the official TrustTunnel deep-link format (DEEP_LINK.md in the
// upstream TrustTunnel repo): `tt://?<base64url payload>` where the payload
// is a sequence of TLV fields. Tag and Length are QUIC varints (RFC 9000 §16),
// values are field-specific.

/// Highest deep-link format version this parser understands.
const MAX_SUPPORTED_VERSION: u64 = 1;

const TAG_VERSION: u64 = 0x00;
const TAG_HOSTNAME: u64 = 0x01;
const TAG_ADDRESS: u64 = 0x02;
const TAG_CUSTOM_SNI: u64 = 0x03;
const TAG_HAS_IPV6: u64 = 0x04;
const TAG_USERNAME: u64 = 0x05;
const TAG_PASSWORD: u64 = 0x06;
const TAG_SKIP_VERIFICATION: u64 = 0x07;
const TAG_CERTIFICATE: u64 = 0x08;
const TAG_UPSTREAM_PROTOCOL: u64 = 0x09;
const TAG_ANTI_DPI: u64 = 0x0A;
const TAG_CLIENT_RANDOM_PREFIX: u64 = 0x0B;
const TAG_NAME: u64 = 0x0C;
const TAG_DNS_UPSTREAMS: u64 = 0x0D;

/// Parse a `tt://?<payload>` deep-link URI into a Profile.
pub fn parse_tt_uri(uri: &str) -> Result<Profile> {
    let stripped = uri
        .strip_prefix("tt://")
        .ok_or_else(|| anyhow!("ссылка должна начинаться с tt:// (получено: {})", short(uri, 60)))?;

    // accept "tt://?xxx" and "tt://xxx" and "tt:///?xxx"
    let payload = stripped.trim_start_matches('/').trim_start_matches('?');
    if payload.is_empty() {
        bail!("пустая ссылка: после tt:// ничего нет");
    }

    // Browsers may percent-encode the URI before handing it to the app.
    let payload = urlencoding::decode(payload)
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| payload.to_string());

    // Spec says base64url without padding; be lenient about padding and the
    // standard alphabet, since links get mangled by chats and copy-paste.
    let normalized: String = payload
        .trim_end_matches('=')
        .chars()
        .map(|c| match c {
            '+' => '-',
            '/' => '_',
            c => c,
        })
        .collect();

    let bytes = URL_SAFE_NO_PAD
        .decode(normalized.as_bytes())
        .map_err(|e| anyhow!("не удалось декодировать base64: {}", e))?;

    parse_tlv_payload(&bytes)
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

/// Read a QUIC variable-length integer (RFC 9000 §16): the two most
/// significant bits of the first byte encode the total size (1/2/4/8 bytes),
/// multi-byte values are big-endian.
fn read_varint(data: &[u8], i: &mut usize) -> Result<u64> {
    let first = *data
        .get(*i)
        .ok_or_else(|| anyhow!("обрезанные данные: ожидался varint на смещении {}", *i))?;
    let len = 1usize << (first >> 6);
    if *i + len > data.len() {
        bail!("обрезанный varint на смещении {}", *i);
    }
    let mut v = (first & 0x3F) as u64;
    for k in 1..len {
        v = (v << 8) | data[*i + k] as u64;
    }
    *i += len;
    Ok(v)
}

fn decode_string(value: &[u8]) -> Result<String> {
    Ok(std::str::from_utf8(value)
        .map_err(|e| anyhow!("строка не в UTF-8: {}", e))?
        .to_string())
}

fn decode_bool(value: &[u8]) -> Result<bool> {
    match value {
        [0x00] => Ok(false),
        [0x01] => Ok(true),
        _ => bail!("некорректное булево значение: {:02X?}", value),
    }
}

/// String[]: concatenation of varint-length-prefixed UTF-8 strings.
fn decode_string_array(value: &[u8]) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < value.len() {
        let len = read_varint(value, &mut i)? as usize;
        if i + len > value.len() {
            bail!("обрезанный элемент списка строк");
        }
        out.push(decode_string(&value[i..i + len])?);
        i += len;
    }
    Ok(out)
}

/// `prefix[/mask]`, both parts hex.
fn validate_client_random_prefix(s: &str) -> Result<()> {
    let (prefix, mask) = s.split_once('/').unwrap_or((s, ""));
    for (part, what) in [(prefix, "префикс"), (mask, "маска")] {
        if part.len() % 2 != 0 || !part.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!("client_random_prefix: {} должен быть hex-строкой", what);
        }
    }
    Ok(())
}

/// Convert one or more concatenated DER certificates into a PEM chain,
/// which is what the client config `certificate` field expects.
fn der_chain_to_pem(der: &[u8]) -> Result<String> {
    let mut out = String::new();
    let mut i = 0usize;
    while i < der.len() {
        let start = i;
        if der[i] != 0x30 {
            bail!("сертификат: ожидался DER SEQUENCE на смещении {}", i);
        }
        i += 1;
        let first_len = *der
            .get(i)
            .ok_or_else(|| anyhow!("сертификат: обрезанный DER"))?;
        i += 1;
        let body_len = if first_len & 0x80 == 0 {
            first_len as usize
        } else {
            let n = (first_len & 0x7F) as usize;
            if n == 0 || n > 4 || i + n > der.len() {
                bail!("сертификат: некорректная DER-длина");
            }
            let mut v = 0usize;
            for k in 0..n {
                v = (v << 8) | der[i + k] as usize;
            }
            i += n;
            v
        };
        let end = i + body_len;
        if end > der.len() {
            bail!("сертификат: обрезанные DER-данные");
        }
        out.push_str("-----BEGIN CERTIFICATE-----\n");
        let b64 = STANDARD.encode(&der[start..end]);
        for chunk in b64.as_bytes().chunks(64) {
            out.push_str(std::str::from_utf8(chunk).expect("base64 is ascii"));
            out.push('\n');
        }
        out.push_str("-----END CERTIFICATE-----\n");
        i = end;
    }
    if out.is_empty() {
        bail!("сертификат: пустое поле");
    }
    Ok(out)
}

fn parse_tlv_payload(bytes: &[u8]) -> Result<Profile> {
    let mut i = 0usize;

    let mut hostname: Option<String> = None;
    let mut addresses: Vec<String> = Vec::new();
    let mut username: Option<String> = None;
    let mut password: Option<String> = None;
    let mut name: Option<String> = None;

    let mut profile = Profile::new_blank("Imported");

    while i < bytes.len() {
        let tag = read_varint(bytes, &mut i)?;
        let len = read_varint(bytes, &mut i)? as usize;
        if i + len > bytes.len() {
            bail!(
                "обрезанное поле (тег 0x{:02X}): заявлено {} байт, осталось {}",
                tag,
                len,
                bytes.len() - i
            );
        }
        let value = &bytes[i..i + len];
        i += len;

        match tag {
            TAG_VERSION => {
                let mut vi = 0usize;
                let v = read_varint(value, &mut vi)?;
                if v > MAX_SUPPORTED_VERSION {
                    bail!(
                        "ссылка создана более новой версией TrustTunnel (формат v{}, поддерживается до v{}) — обновите приложение",
                        v,
                        MAX_SUPPORTED_VERSION
                    );
                }
            }
            TAG_HOSTNAME => hostname = Some(decode_string(value)?),
            TAG_ADDRESS => addresses.push(decode_string(value)?),
            TAG_CUSTOM_SNI => profile.custom_sni = decode_string(value)?,
            TAG_HAS_IPV6 => profile.has_ipv6 = decode_bool(value)?,
            TAG_USERNAME => username = Some(decode_string(value)?),
            TAG_PASSWORD => password = Some(decode_string(value)?),
            TAG_SKIP_VERIFICATION => profile.skip_verification = decode_bool(value)?,
            TAG_CERTIFICATE => profile.certificate = der_chain_to_pem(value)?,
            TAG_UPSTREAM_PROTOCOL => {
                profile.upstream_protocol = match value {
                    [0x01] => "http2".to_string(),
                    [0x02] => "http3".to_string(),
                    _ => bail!("неизвестный upstream_protocol: {:02X?}", value),
                };
            }
            TAG_ANTI_DPI => profile.anti_dpi = decode_bool(value)?,
            TAG_CLIENT_RANDOM_PREFIX => {
                let s = decode_string(value)?;
                validate_client_random_prefix(&s)?;
                profile.client_random = s;
            }
            TAG_NAME => name = Some(decode_string(value)?),
            TAG_DNS_UPSTREAMS => {
                let dns = decode_string_array(value)?;
                if !dns.is_empty() {
                    profile.dns_upstreams = dns;
                }
            }
            // Unknown tags are ignored per spec (forward compatibility).
            _ => {}
        }
    }

    // Required fields per spec.
    let hostname = hostname.ok_or_else(|| anyhow!("в ссылке нет обязательного поля hostname"))?;
    if addresses.is_empty() {
        bail!("в ссылке нет обязательного поля addresses");
    }
    profile.hostname = hostname;
    profile.addresses = addresses;
    profile.username = username.ok_or_else(|| anyhow!("в ссылке нет обязательного поля username"))?;
    profile.password = password.ok_or_else(|| anyhow!("в ссылке нет обязательного поля password"))?;
    profile.name = name.unwrap_or_else(|| profile.hostname.clone());

    Ok(profile)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn varint(v: u64) -> Vec<u8> {
        if v < 64 {
            vec![v as u8]
        } else if v < 16384 {
            vec![0x40 | (v >> 8) as u8, v as u8]
        } else {
            vec![
                0x80 | (v >> 24) as u8,
                (v >> 16) as u8,
                (v >> 8) as u8,
                v as u8,
            ]
        }
    }

    fn tlv(tag: u64, value: &[u8]) -> Vec<u8> {
        let mut out = varint(tag);
        out.extend(varint(value.len() as u64));
        out.extend_from_slice(value);
        out
    }

    fn tlv_str(tag: u64, s: &str) -> Vec<u8> {
        tlv(tag, s.as_bytes())
    }

    /// Minimal fake DER cert: SEQUENCE header + opaque body.
    fn fake_der_cert(body_len: usize) -> Vec<u8> {
        let mut out = vec![0x30];
        if body_len < 128 {
            out.push(body_len as u8);
        } else {
            out.push(0x82);
            out.push((body_len >> 8) as u8);
            out.push(body_len as u8);
        }
        out.extend(std::iter::repeat(0xAB).take(body_len));
        out
    }

    fn base_payload() -> Vec<u8> {
        let mut p = Vec::new();
        p.extend(tlv(TAG_VERSION, &[0x01]));
        p.extend(tlv_str(TAG_HOSTNAME, "vpn.example.com"));
        p.extend(tlv_str(TAG_ADDRESS, "vpn.example.com:12443"));
        p.extend(tlv_str(TAG_USERNAME, "alice"));
        p.extend(tlv_str(TAG_PASSWORD, "s3cret"));
        p
    }

    fn to_uri(payload: &[u8]) -> String {
        format!("tt://?{}", URL_SAFE_NO_PAD.encode(payload))
    }

    #[test]
    fn parses_minimal_link() {
        let p = parse_tt_uri(&to_uri(&base_payload())).unwrap();
        assert_eq!(p.hostname, "vpn.example.com");
        assert_eq!(p.addresses, vec!["vpn.example.com:12443"]);
        assert_eq!(p.username, "alice");
        assert_eq!(p.password, "s3cret");
        // defaults per spec
        assert!(p.has_ipv6);
        assert!(!p.skip_verification);
        assert!(!p.anti_dpi);
        assert_eq!(p.upstream_protocol, "http2");
        assert_eq!(p.name, "vpn.example.com");
        assert!(p.certificate.is_empty());
    }

    #[test]
    fn parses_full_link_with_certificate() {
        let mut payload = base_payload();
        // protocol http2 explicitly, like real server-generated links
        payload.extend(tlv(TAG_UPSTREAM_PROTOCOL, &[0x01]));
        // a 380-byte cert forces a 2-byte varint TLV length, the case the
        // old parser choked on
        let cert = fake_der_cert(376);
        assert_eq!(cert.len(), 380);
        payload.extend(tlv(TAG_CERTIFICATE, &cert));
        payload.extend(tlv_str(TAG_NAME, "My Server"));
        payload.extend(tlv(TAG_HAS_IPV6, &[0x00]));
        payload.extend(tlv(TAG_ANTI_DPI, &[0x01]));

        let p = parse_tt_uri(&to_uri(&payload)).unwrap();
        assert_eq!(p.name, "My Server");
        assert!(!p.has_ipv6);
        assert!(p.anti_dpi);
        assert_eq!(p.upstream_protocol, "http2");
        assert!(p.certificate.starts_with("-----BEGIN CERTIFICATE-----\n"));
        assert!(p.certificate.trim_end().ends_with("-----END CERTIFICATE-----"));
    }

    #[test]
    fn certificate_chain_yields_multiple_pem_blocks() {
        let mut chain = fake_der_cert(100);
        chain.extend(fake_der_cert(200));
        let pem = der_chain_to_pem(&chain).unwrap();
        assert_eq!(pem.matches("BEGIN CERTIFICATE").count(), 2);
    }

    #[test]
    fn parses_dns_upstreams_array() {
        let mut payload = base_payload();
        let mut arr = Vec::new();
        for s in ["1.1.1.1", "tls://dns.example.com"] {
            arr.extend(varint(s.len() as u64));
            arr.extend_from_slice(s.as_bytes());
        }
        payload.extend(tlv(TAG_DNS_UPSTREAMS, &arr));
        let p = parse_tt_uri(&to_uri(&payload)).unwrap();
        assert_eq!(p.dns_upstreams, vec!["1.1.1.1", "tls://dns.example.com"]);
    }

    #[test]
    fn ignores_unknown_tags() {
        let mut payload = base_payload();
        payload.extend(tlv(0x1F, &[0xDE, 0xAD]));
        assert!(parse_tt_uri(&to_uri(&payload)).is_ok());
    }

    #[test]
    fn rejects_newer_version() {
        let mut payload = Vec::new();
        payload.extend(tlv(TAG_VERSION, &[0x02]));
        payload.extend(&base_payload()[3..]); // skip the v1 version TLV
        let err = parse_tt_uri(&to_uri(&payload)).unwrap_err().to_string();
        assert!(err.contains("v2"), "{}", err);
    }

    #[test]
    fn rejects_missing_required_fields() {
        let mut payload = Vec::new();
        payload.extend(tlv_str(TAG_HOSTNAME, "vpn.example.com"));
        let err = parse_tt_uri(&to_uri(&payload)).unwrap_err().to_string();
        assert!(err.contains("addresses"), "{}", err);
    }

    #[test]
    fn rejects_truncated_payload() {
        let mut payload = base_payload();
        let cert = fake_der_cert(376);
        payload.extend(tlv(TAG_CERTIFICATE, &cert));
        payload.truncate(payload.len() - 10);
        assert!(parse_tt_uri(&to_uri(&payload)).is_err());
    }

    #[test]
    fn accepts_padded_and_standard_alphabet_base64() {
        let payload = base_payload();
        let standard = STANDARD.encode(&payload);
        let p = parse_tt_uri(&format!("tt://?{}", standard)).unwrap();
        assert_eq!(p.hostname, "vpn.example.com");
    }

    #[test]
    fn accepts_legacy_form_without_question_mark() {
        let payload = base_payload();
        let uri = format!("tt://{}", URL_SAFE_NO_PAD.encode(&payload));
        assert!(parse_tt_uri(&uri).is_ok());
    }

    #[test]
    fn client_random_prefix_with_mask() {
        let mut payload = base_payload();
        payload.extend(tlv_str(TAG_CLIENT_RANDOM_PREFIX, "5841/7a43"));
        let p = parse_tt_uri(&to_uri(&payload)).unwrap();
        assert_eq!(p.client_random, "5841/7a43");
    }

    #[test]
    fn varint_two_byte() {
        let data = [0x41, 0x7C];
        let mut i = 0;
        assert_eq!(read_varint(&data, &mut i).unwrap(), 380);
        assert_eq!(i, 2);
    }

    #[test]
    fn extracts_uri_from_text() {
        assert_eq!(
            extract_tt_uri("вот ссылка tt://?AAEB смотри").as_deref(),
            Some("tt://?AAEB")
        );
    }
}
