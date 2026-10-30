//! The provider seam: the narrow trait `effects.rs` sends through, the S3
//! client (Signature Version 4 by hand over `ring`, path-style requests at
//! the provider's endpoint, virtual-hosted presigned URLs moved onto the
//! environment's private address) and the provider that refuses everything
//! when the Secret is absent. A provider answers with a `BoxError` that
//! carries its own verdict: `definitive(..)` for a parsed 4xx, the
//! receiver's answer; `TransportError::wrap(..)` for everything whose
//! outcome is unknowable (a transport error, a 5xx, a timeout).

use std::future::Future;

use basable_core::{BoxError, Ctx};
use basable_externaleffect::{TransportError, definitive};
use chrono::{DateTime, Utc};
use reqwest::StatusCode;
use ring::{digest, hmac};

use crate::effects::*;

/// Exactly the calls this nanoservice makes — never a wrapper around the
/// whole provider API. The futures are `Send` because a worker dispatches
/// them.
pub trait Provider: Send + Sync + 'static {
    /// Deletes a key; a missing key is success.
    fn delete_object(&self, ctx: Ctx, args: DeleteObjectArgs) -> impl Future<Output = Result<DeleteObjectResult, BoxError>> + Send;
    /// The keys under a prefix, at most `limit`, in key order.
    fn list_keys(&self, ctx: Ctx, prefix: String, limit: usize) -> impl Future<Output = Result<Vec<String>, BoxError>> + Send;
    /// A presigned URL a browser may use as it is.
    fn presign(&self, args: PresignArgs) -> Result<String, BoxError>;
}

// --- Signature Version 4 ---

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_hex(data: &[u8]) -> String {
    hex(digest::digest(&digest::SHA256, data).as_ref())
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), data).as_ref().to_vec()
}

/// RFC 3986 encoding as S3 canonicalises: unreserved characters kept, `/`
/// kept in a path and encoded in a query value.
pub(crate) fn uri_encode(s: &str, encode_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b'/' if !encode_slash => out.push('/'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// A sorted, encoded query string.
fn canonical_query(params: &[(String, String)]) -> String {
    let mut encoded: Vec<(String, String)> = params.iter().map(|(k, v)| (uri_encode(k, true), uri_encode(v, true))).collect();
    encoded.sort();
    encoded.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&")
}

/// The signing material of one request: what SigV4 hashes.
struct Canonical {
    request: String,
    signed_headers: String,
}

fn canonical(method: &str, uri: &str, query: &str, headers: &[(String, String)], payload_hash: &str) -> Canonical {
    let mut headers: Vec<(String, String)> = headers.iter().map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_owned())).collect();
    headers.sort();
    let canonical_headers: String = headers.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let signed_headers = headers.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(";");
    Canonical {
        request: format!("{method}\n{uri}\n{query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"),
        signed_headers,
    }
}

const UNSIGNED_PAYLOAD: &str = "UNSIGNED-PAYLOAD";

/// The credentials and the arithmetic over them.
struct Signer {
    access_key: String,
    secret_key: String,
    region: String,
}

impl Signer {
    fn scope(&self, date: &str) -> String {
        format!("{date}/{}/s3/aws4_request", self.region)
    }

    fn signature(&self, date: &str, amz_date: &str, canonical_request: &str) -> String {
        let string_to_sign = format!("AWS4-HMAC-SHA256\n{amz_date}\n{}\n{}", self.scope(date), sha256_hex(canonical_request.as_bytes()));
        let k_date = hmac_sha256(format!("AWS4{}", self.secret_key).as_bytes(), date.as_bytes());
        let k_region = hmac_sha256(&k_date, self.region.as_bytes());
        let k_service = hmac_sha256(&k_region, b"s3");
        let k_signing = hmac_sha256(&k_service, b"aws4_request");
        hex(&hmac_sha256(&k_signing, string_to_sign.as_bytes()))
    }
}

fn amz_dates(now: DateTime<Utc>) -> (String, String) {
    (now.format("%Y%m%dT%H%M%SZ").to_string(), now.format("%Y%m%d").to_string())
}

// --- the S3 client ---

/// The environment's private bucket over its S3-compatible endpoint.
pub struct S3Client {
    /// `https://host[:port]`, the provider's endpoint, never shown to a
    /// browser.
    endpoint: String,
    endpoint_host: String,
    bucket: String,
    /// `https://<words>.private.s3...`: what a presigned URL's origin is
    /// replaced with; it forwards to the bucket with nothing changed but
    /// the host.
    private_url: String,
    signer: Signer,
    client: reqwest::Client,
}

impl S3Client {
    /// The client from the `object-storage` Secret's variables, `None` when
    /// `AWS_ENDPOINT_URL_S3` is absent, an error when the set is incomplete.
    pub fn from_env() -> Result<Option<S3Client>, BoxError> {
        let var = |name: &str| std::env::var(name).ok().filter(|s| !s.is_empty());
        let Some(endpoint) = var("AWS_ENDPOINT_URL_S3") else {
            return Ok(None);
        };
        let need = |name: &str| var(name).ok_or_else(|| BoxError::from(format!("{name} is not set")));
        let region = var("AWS_REGION").or_else(|| var("AWS_DEFAULT_REGION")).unwrap_or_else(|| "us-east-1".to_owned());
        Ok(Some(S3Client::new(
            &endpoint,
            region,
            need("S3_BUCKET")?,
            need("AWS_ACCESS_KEY_ID")?,
            need("AWS_SECRET_ACCESS_KEY")?,
            &need("S3_PRIVATE_URL")?,
        )?))
    }

    pub fn new(endpoint: &str, region: String, bucket: String, access_key: String, secret_key: String, private_url: &str) -> Result<S3Client, BoxError> {
        let endpoint = endpoint.trim_end_matches('/').to_owned();
        let endpoint_host = endpoint
            .split_once("://")
            .map(|(_, rest)| rest.to_owned())
            .ok_or_else(|| BoxError::from("AWS_ENDPOINT_URL_S3: not a URL"))?;
        if endpoint_host.is_empty() || endpoint_host.contains('/') {
            return Err("AWS_ENDPOINT_URL_S3: scheme and host only".into());
        }
        Ok(S3Client {
            endpoint,
            endpoint_host,
            bucket,
            private_url: private_url.trim_end_matches('/').to_owned(),
            signer: Signer { access_key, secret_key, region },
            client: reqwest::Client::new(),
        })
    }

    /// A virtual-hosted presigned URL, its origin replaced by the private
    /// address (the gateway forwards it with nothing changed but the host,
    /// so the signature holds). A PUT is signed for its exact length.
    fn presign_url(&self, args: &PresignArgs) -> String {
        let (amz_date, date) = amz_dates(args.now);
        let (method, mut headers) = match &args.method {
            PresignMethod::Put { size } => ("PUT", vec![("content-length".to_owned(), size.to_string())]),
            PresignMethod::Get => ("GET", Vec::new()),
        };
        headers.push(("host".to_owned(), format!("{}.{}", self.bucket, self.endpoint_host)));
        let signed_headers = {
            let mut names: Vec<&str> = headers.iter().map(|(k, _)| k.as_str()).collect();
            names.sort_unstable();
            names.join(";")
        };
        let params = vec![
            ("X-Amz-Algorithm".to_owned(), "AWS4-HMAC-SHA256".to_owned()),
            ("X-Amz-Credential".to_owned(), format!("{}/{}", self.signer.access_key, self.signer.scope(&date))),
            ("X-Amz-Date".to_owned(), amz_date.clone()),
            ("X-Amz-Expires".to_owned(), args.expires.as_secs().to_string()),
            ("X-Amz-SignedHeaders".to_owned(), signed_headers),
        ];
        let query = canonical_query(&params);
        let uri = format!("/{}", uri_encode(&args.key, false));
        let c = canonical(method, &uri, &query, &headers, UNSIGNED_PAYLOAD);
        let signature = self.signer.signature(&date, &amz_date, &c.request);
        format!("{}{uri}?{query}&X-Amz-Signature={signature}", self.private_url)
    }

    /// A signed path-style request at the endpoint.
    async fn signed(&self, method: reqwest::Method, path: &str, params: &[(String, String)]) -> Result<reqwest::Response, BoxError> {
        let (amz_date, date) = amz_dates(Utc::now());
        let headers = vec![
            ("host".to_owned(), self.endpoint_host.clone()),
            ("x-amz-content-sha256".to_owned(), UNSIGNED_PAYLOAD.to_owned()),
            ("x-amz-date".to_owned(), amz_date.clone()),
        ];
        let query = canonical_query(params);
        let c = canonical(method.as_str(), path, &query, &headers, UNSIGNED_PAYLOAD);
        let signature = self.signer.signature(&date, &amz_date, &c.request);
        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={signature}",
            self.signer.access_key,
            self.signer.scope(&date),
            c.signed_headers
        );
        let url = if query.is_empty() { format!("{}{path}", self.endpoint) } else { format!("{}{path}?{query}", self.endpoint) };
        self.client
            .request(method, url)
            .header("x-amz-content-sha256", UNSIGNED_PAYLOAD)
            .header("x-amz-date", amz_date)
            .header("authorization", authorization)
            .send()
            .await
            .map_err(TransportError::wrap)
    }
}

/// The verdict on a status the caller did not already handle.
fn refused(what: &str, status: StatusCode, body: &str) -> BoxError {
    let message = format!("object storage {what}: {status}: {body}");
    if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
        TransportError::wrap(message)
    } else {
        definitive(message)
    }
}

/// The text of every `<Key>` element of a ListObjectsV2 answer, in order.
/// The keys this application writes carry no XML-special character; the
/// five entities are decoded all the same.
pub(crate) fn list_keys_of(xml: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<Key>") {
        let after = &rest[start + "<Key>".len()..];
        let Some(end) = after.find("</Key>") else { break };
        keys.push(
            after[..end]
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&quot;", "\"")
                .replace("&apos;", "'")
                .replace("&amp;", "&"),
        );
        rest = &after[end + "</Key>".len()..];
    }
    keys
}

impl Provider for S3Client {
    async fn delete_object(&self, _ctx: Ctx, args: DeleteObjectArgs) -> Result<DeleteObjectResult, BoxError> {
        let path = format!("/{}/{}", uri_encode(&self.bucket, true), uri_encode(&args.key, false));
        let response = self.signed(reqwest::Method::DELETE, &path, &[]).await?;
        match response.status() {
            StatusCode::NO_CONTENT | StatusCode::OK | StatusCode::NOT_FOUND => Ok(DeleteObjectResult { key: args.key }),
            status => {
                let body = response.text().await.unwrap_or_default();
                Err(refused("delete", status, &body))
            }
        }
    }

    async fn list_keys(&self, _ctx: Ctx, prefix: String, limit: usize) -> Result<Vec<String>, BoxError> {
        let path = format!("/{}", uri_encode(&self.bucket, true));
        let params = vec![
            ("list-type".to_owned(), "2".to_owned()),
            ("max-keys".to_owned(), limit.clamp(1, 1000).to_string()),
            ("prefix".to_owned(), prefix),
        ];
        let response = self.signed(reqwest::Method::GET, &path, &params).await?;
        let status = response.status();
        let body = response.text().await.map_err(TransportError::wrap)?;
        if !status.is_success() {
            return Err(refused("list", status, &body));
        }
        Ok(list_keys_of(&body))
    }

    fn presign(&self, args: PresignArgs) -> Result<String, BoxError> {
        Ok(self.presign_url(&args))
    }
}

/// No `object-storage` Secret in the environment: every call is refused,
/// so nothing is sent and nothing is ambiguous.
pub struct Unconfigured;

impl Provider for Unconfigured {
    async fn delete_object(&self, _ctx: Ctx, _args: DeleteObjectArgs) -> Result<DeleteObjectResult, BoxError> {
        Err(definitive("storage: object storage not configured (the object-storage Secret)"))
    }

    async fn list_keys(&self, _ctx: Ctx, _prefix: String, _limit: usize) -> Result<Vec<String>, BoxError> {
        Err("storage: object storage not configured (the object-storage Secret)".into())
    }

    fn presign(&self, _args: PresignArgs) -> Result<String, BoxError> {
        Err("storage: object storage not configured (the object-storage Secret)".into())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn client() -> S3Client {
        S3Client::new(
            "https://s3.example.test",
            "garage".into(),
            "words-private".into(),
            "AKIDEXAMPLE".into(),
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
            "https://words.private.s3.env.example/",
        )
        .unwrap()
    }

    #[test]
    fn encoding_keeps_unreserved_and_paths() {
        assert_eq!(uri_encode("g/ab-c_d.e~f", false), "g/ab-c_d.e~f");
        assert_eq!(uri_encode("g/a b", true), "g%2Fa%20b");
    }

    #[test]
    fn signing_key_derivation_matches_the_aws_example() {
        // The SigV4 reference vector: AWS4<secret>, 20120215, us-east-1, iam.
        let signer = Signer { access_key: "AKIDEXAMPLE".into(), secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(), region: "us-east-1".into() };
        let k_date = hmac_sha256(format!("AWS4{}", signer.secret_key).as_bytes(), b"20120215");
        assert_eq!(hex(&k_date), "969fbb94feb542b71ede6f87fe4d5fa29c789342b0f407474670f0c2489e0a0d");
    }

    #[test]
    fn a_presigned_url_lives_on_the_private_address_and_is_signed() {
        let now = DateTime::parse_from_rfc3339("2026-10-30T12:00:00Z").unwrap().with_timezone(&Utc);
        let url = client()
            .presign(PresignArgs { method: PresignMethod::Put { size: 42 }, key: "g/x/o/y/full".into(), expires: Duration::from_secs(900), now })
            .unwrap();
        assert!(url.starts_with("https://words.private.s3.env.example/g/x/o/y/full?"), "{url}");
        assert!(url.contains("X-Amz-SignedHeaders=content-length%3Bhost"), "{url}");
        assert!(url.contains("X-Amz-Expires=900"), "{url}");
        assert!(url.contains("X-Amz-Date=20261030T120000Z"), "{url}");
        let signature = url.rsplit("X-Amz-Signature=").next().unwrap();
        assert_eq!(signature.len(), 64);
        assert!(!url.contains("s3.example.test"), "the provider's address never reaches a browser");
        let again = client()
            .presign(PresignArgs { method: PresignMethod::Put { size: 42 }, key: "g/x/o/y/full".into(), expires: Duration::from_secs(900), now })
            .unwrap();
        assert_eq!(url, again, "deterministic for the same inputs");
        let get = client().presign(PresignArgs { method: PresignMethod::Get, key: "g/x/o/y/full".into(), expires: Duration::from_secs(300), now }).unwrap();
        assert!(get.contains("X-Amz-SignedHeaders=host&"), "{get}");
        assert_ne!(get, url);
    }

    #[test]
    fn list_keys_are_read_from_the_xml() {
        let xml = r#"<?xml version="1.0"?><ListBucketResult><IsTruncated>false</IsTruncated><Contents><Key>g/a/o/b/full</Key><Size>1</Size></Contents><Contents><Key>g/a/o/b/file&amp;x</Key></Contents></ListBucketResult>"#;
        assert_eq!(list_keys_of(xml), vec!["g/a/o/b/full", "g/a/o/b/file&x"]);
        assert!(list_keys_of("<ListBucketResult></ListBucketResult>").is_empty());
    }
}
