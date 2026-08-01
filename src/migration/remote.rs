use color_eyre::eyre::{Result, WrapErr, bail};
use serde_json::Value;

const S3_COMPATIBLE_DRIVERS: &[&str] = &["s3", "ks3", "cos"];

pub(super) fn supports_remote_validation(
    policy: &cloudreve_schema::storage_policies::Model,
) -> bool {
    S3_COMPATIBLE_DRIVERS.contains(&policy.r#type.as_str())
        && cloudreve_adapter::storage_policy_skip_reason(policy).is_none()
}

pub(super) async fn verify_object(
    policy: &cloudreve_schema::storage_policies::Model,
    storage_path: &str,
    expected_size: i64,
    entity_id: i64,
) -> Result<()> {
    if !supports_remote_validation(policy) {
        bail!(
            "Cloudreve storage policy {} ({}) is not an S3-compatible remote policy",
            policy.id,
            policy.r#type
        );
    }
    if storage_path.is_empty() {
        bail!("Cloudreve remote entity {entity_id} has an empty object key");
    }
    let client = client(policy)?;
    let bucket = required(&policy.bucket_name, "bucket", policy.id)?;
    let head = client
        .head_object()
        .bucket(bucket)
        .key(storage_path)
        .send()
        .await
        .wrap_err_with(|| format!("HeadObject for Cloudreve entity {entity_id}"))?;
    if head.content_length() != Some(expected_size) {
        bail!(
            "Cloudreve remote entity {entity_id} size mismatch: database={expected_size}, provider={:?}",
            head.content_length()
        );
    }
    if expected_size > 0 {
        let response = client
            .get_object()
            .bucket(bucket)
            .key(storage_path)
            .range("bytes=0-0")
            .send()
            .await
            .wrap_err_with(|| format!("range read for Cloudreve entity {entity_id}"))?;
        let bytes = response
            .body
            .collect()
            .await
            .wrap_err_with(|| format!("read range response for Cloudreve entity {entity_id}"))?
            .into_bytes();
        if bytes.len() != 1 {
            bail!(
                "Cloudreve remote entity {entity_id} range read returned {} bytes instead of 1",
                bytes.len()
            );
        }
    }
    Ok(())
}

fn client(policy: &cloudreve_schema::storage_policies::Model) -> Result<aws_sdk_s3::Client> {
    let endpoint = required(&policy.server, "endpoint", policy.id)?;
    let bucket = required(&policy.bucket_name, "bucket", policy.id)?;
    let access_key = required(&policy.access_key, "access key", policy.id)?;
    let secret_key = required(&policy.secret_key, "secret key", policy.id)?;
    let settings = policy.settings.as_ref().cloned().unwrap_or(Value::Null);
    let region = storage_region(policy, endpoint, &settings)?;
    let path_style = force_path_style(policy, &settings);
    let endpoint = if policy.r#type == "cos" {
        cos_s3_endpoint(endpoint, bucket)?
    } else {
        endpoint.to_string()
    };
    let config = aws_sdk_s3::Config::builder()
        .behavior_version_latest()
        .region(aws_sdk_s3::config::Region::new(region.to_string()))
        .endpoint_url(endpoint)
        .force_path_style(path_style)
        .credentials_provider(aws_sdk_s3::config::Credentials::new(
            access_key,
            secret_key,
            None,
            None,
            "cloudreve-migration",
        ))
        .build();
    Ok(aws_sdk_s3::Client::from_conf(config))
}

fn cos_s3_endpoint(endpoint: &str, bucket: &str) -> Result<String> {
    let mut endpoint = url::Url::parse(endpoint).wrap_err("parse Cloudreve COS endpoint")?;
    let host = endpoint
        .host_str()
        .ok_or_else(|| color_eyre::eyre::eyre!("Cloudreve COS endpoint has no host"))?
        .to_string();
    let bucket_prefix = format!("{}.", bucket.trim());
    if host
        .get(..bucket_prefix.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(&bucket_prefix))
    {
        endpoint
            .set_host(Some(&host[bucket_prefix.len()..]))
            .map_err(|_| color_eyre::eyre::eyre!("build Cloudreve COS S3 endpoint"))?;
    }
    endpoint.set_query(None);
    endpoint.set_fragment(None);
    Ok(String::from(endpoint).trim_end_matches('/').to_string())
}

fn storage_region(
    policy: &cloudreve_schema::storage_policies::Model,
    endpoint: &str,
    settings: &Value,
) -> Result<String> {
    if let Some(region) = cloudreve_adapter::storage_region_setting(settings)
        .map_err(|message| color_eyre::eyre::eyre!(message))?
    {
        return Ok(region);
    }
    if policy.r#type == "cos" {
        return cos_region_from_endpoint(endpoint).ok_or_else(|| {
            color_eyre::eyre::eyre!(
                "Cloudreve COS policy {} needs settings.region (or settings.s3_region); it could not be derived from endpoint {endpoint}",
                policy.id
            )
        });
    }
    Ok("us-east-1".to_string())
}

fn force_path_style(policy: &cloudreve_schema::storage_policies::Model, settings: &Value) -> bool {
    if policy.r#type == "cos" {
        // Tencent COS recommends virtual-hosted-style URLs, and newer buckets require them.
        false
    } else {
        settings
            .get("s3_path_style")
            .and_then(Value::as_bool)
            .unwrap_or(true)
    }
}

fn cos_region_from_endpoint(endpoint: &str) -> Option<String> {
    let authority = endpoint
        .trim()
        .strip_prefix("https://")
        .or_else(|| endpoint.trim().strip_prefix("http://"))
        .unwrap_or(endpoint)
        .split('/')
        .next()?
        .split(':')
        .next()?;
    let labels = authority.split('.').collect::<Vec<_>>();
    let cos_index = labels.iter().position(|label| *label == "cos")?;
    let region = *labels.get(cos_index + 1)?;
    let suffix = labels.get(cos_index + 2..)?;
    (suffix == ["myqcloud", "com"] && !region.is_empty()).then(|| region.to_string())
}

fn required<'a>(value: &'a Option<String>, name: &str, policy_id: i64) -> Result<&'a str> {
    value
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| color_eyre::eyre::eyre!("Cloudreve remote policy {policy_id} has no {name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(policy_type: &str) -> cloudreve_schema::storage_policies::Model {
        cloudreve_schema::storage_policies::Model {
            id: 1,
            created_at: chrono::Utc::now().fixed_offset(),
            updated_at: chrono::Utc::now().fixed_offset(),
            deleted_at: None,
            name: "test".to_string(),
            r#type: policy_type.to_string(),
            server: Some("https://s3.example.test".to_string()),
            bucket_name: Some("bucket".to_string()),
            is_private: None,
            access_key: Some("access".to_string()),
            secret_key: Some("secret".to_string()),
            max_size: None,
            dir_name_rule: None,
            file_name_rule: None,
            settings: None,
            node_id: None,
        }
    }

    #[test]
    fn remote_validation_support_matches_the_adapter_contract() {
        for policy_type in ["s3", "ks3"] {
            assert!(supports_remote_validation(&policy(policy_type)));
        }

        let mut cos = policy("cos");
        cos.server = Some("https://bucket.cos.ap-guangzhou.myqcloud.com".to_string());
        assert!(supports_remote_validation(&cos));

        for policy_type in [
            "local", "oss", "obs", "qiniu", "upyun", "onedrive", "remote",
        ] {
            assert!(!supports_remote_validation(&policy(policy_type)));
        }

        let mut encrypted = policy("s3");
        encrypted.settings = Some(serde_json::json!({"encryption": true}));
        assert!(!supports_remote_validation(&encrypted));

        let mut incomplete = policy("s3");
        incomplete.secret_key = None;
        assert!(!supports_remote_validation(&incomplete));

        let mut custom_domain_cos = policy("cos");
        custom_domain_cos.server = Some("https://cdn.example.test".to_string());
        assert!(!supports_remote_validation(&custom_domain_cos));
    }

    #[test]
    fn resolves_explicit_alias_default_and_cos_regions() -> Result<()> {
        let s3 = policy("s3");
        assert_eq!(
            storage_region(
                &s3,
                "https://s3.example.test",
                &serde_json::json!({
                    "region": "  ap-shanghai  "
                })
            )?,
            "ap-shanghai"
        );
        assert_eq!(
            storage_region(
                &s3,
                "https://s3.example.test",
                &serde_json::json!({
                    "region": "",
                    "s3_region": "eu-central-1"
                })
            )?,
            "eu-central-1"
        );
        assert_eq!(
            storage_region(&s3, "https://s3.example.test", &Value::Null)?,
            "us-east-1"
        );

        let cos = policy("cos");
        assert_eq!(
            storage_region(
                &cos,
                "https://bucket.cos.ap-guangzhou.myqcloud.com/path",
                &Value::Null,
            )?,
            "ap-guangzhou"
        );
        assert!(storage_region(&cos, "https://cos.invalid.example", &Value::Null).is_err());
        Ok(())
    }

    #[test]
    fn derives_cos_region_only_from_standard_endpoint_hosts() {
        for (endpoint, expected) in [
            (
                "https://bucket.cos.ap-guangzhou.myqcloud.com",
                Some("ap-guangzhou"),
            ),
            (
                "bucket.cos.ap-shanghai.myqcloud.com:443/path",
                Some("ap-shanghai"),
            ),
            ("https://bucket.cos..myqcloud.com", None),
            ("https://bucket.cos.ap-guangzhou.example.com", None),
            ("https://cdn.example.test", None),
            ("", None),
        ] {
            assert_eq!(cos_region_from_endpoint(endpoint).as_deref(), expected);
        }
    }

    #[test]
    fn normalizes_bucket_qualified_cos_endpoint_for_aws_sdk() -> Result<()> {
        assert_eq!(
            cos_s3_endpoint("https://bucket.cos.ap-guangzhou.myqcloud.com", "bucket")?,
            "https://cos.ap-guangzhou.myqcloud.com"
        );
        assert_eq!(
            cos_s3_endpoint(
                "https://Bucket.cos.ap-guangzhou.myqcloud.com/prefix/?ignored=yes#fragment",
                "bucket"
            )?,
            "https://cos.ap-guangzhou.myqcloud.com/prefix"
        );
        assert_eq!(
            cos_s3_endpoint("https://cos.ap-guangzhou.myqcloud.com", "bucket")?,
            "https://cos.ap-guangzhou.myqcloud.com"
        );
        Ok(())
    }

    #[test]
    fn resolves_path_style_for_s3_and_forces_cos_virtual_hosting() {
        let s3 = policy("s3");
        assert!(force_path_style(&s3, &Value::Null));
        assert!(!force_path_style(
            &s3,
            &serde_json::json!({"s3_path_style": false})
        ));
        assert!(force_path_style(
            &s3,
            &serde_json::json!({"s3_path_style": true})
        ));

        let cos = policy("cos");
        assert!(!force_path_style(
            &cos,
            &serde_json::json!({"s3_path_style": true})
        ));
    }

    #[test]
    fn required_rejects_missing_empty_and_whitespace_values() {
        assert_eq!(
            required(&Some(" value ".to_string()), "field", 1).expect("value"),
            " value "
        );
        for value in [None, Some(String::new()), Some("   ".to_string())] {
            assert!(required(&value, "field", 7).is_err());
        }
    }
}
