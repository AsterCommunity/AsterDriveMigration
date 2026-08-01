use super::*;

impl AsterDriveWriter<'_> {
    pub async fn write_direct_link(
        &self,
        resolved: ResolvedDirectLink,
        secret: &str,
    ) -> Result<WrittenDirectLink> {
        let ResolvedDirectLink {
            direct_link,
            target_file_id,
            target_owner_id,
        } = resolved;
        let url = direct_link_url(
            target_file_id,
            target_owner_id,
            &direct_link.file_name,
            secret,
        )?;
        let property_id = self
            .write_property(ResolvedProperty {
                source_metadata_id: direct_link.source_id,
                target: ResolvedEntityTarget::File {
                    target_id: target_file_id,
                },
                namespace: "cloudreve.direct_links".to_string(),
                name: direct_link.source_id.to_string(),
                value: Some(
                    json!({
                        "url": url.clone(),
                        "source_direct_link_id": direct_link.source_id,
                        "source_file_id": direct_link.file_source_id,
                        "source_name": direct_link.source_name,
                        "source_downloads": direct_link.source_downloads,
                        "source_speed_limit": direct_link.source_speed_limit,
                    })
                    .to_string(),
                ),
            })
            .await?;
        Ok(WrittenDirectLink { property_id, url })
    }
}

fn direct_link_url(
    file_id: i64,
    owner_user_id: i64,
    file_name: &str,
    secret: &str,
) -> Result<String> {
    let file_id = u64::try_from(file_id).wrap_err("AD direct link file ID must be non-negative")?;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|error| color_eyre::eyre::eyre!("initialize direct link HMAC: {error}"))?;
    mac.update(b"direct_link:v2:");
    mac.update(format!("user:{owner_user_id}").as_bytes());
    mac.update(b":");
    mac.update(file_id.to_string().as_bytes());
    let signature =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    Ok(format!(
        "/d/v2.{}.{}/{}",
        encode_base62(file_id)?,
        signature,
        urlencoding::encode(file_name)
    ))
}

fn encode_base62(mut value: u64) -> Result<String> {
    const BASE62: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    if value == 0 {
        return Ok("a".to_string());
    }
    let mut encoded = Vec::new();
    while value > 0 {
        let index = usize::try_from(value % 62).wrap_err("base62 digit index exceeds usize")?;
        encoded.push(char::from(BASE62[index]));
        value /= 62;
    }
    Ok(encoded.iter().rev().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base62_encodes_zero_digit_boundaries_and_u64_max() -> Result<()> {
        assert_eq!(encode_base62(0)?, "a");
        assert_eq!(encode_base62(1)?, "b");
        assert_eq!(encode_base62(61)?, "9");
        assert_eq!(encode_base62(62)?, "ba");
        assert_eq!(encode_base62(u64::MAX)?, "v8QrKbgkrIp");
        Ok(())
    }

    #[test]
    fn direct_link_url_accepts_zero_and_encodes_empty_or_reserved_names() -> Result<()> {
        let empty = direct_link_url(0, 0, "", "secret")?;
        assert!(empty.starts_with("/d/v2.a."));
        assert!(empty.ends_with('/'));

        let reserved = direct_link_url(1, -1, "a/b c?.txt", "secret")?;
        assert!(reserved.ends_with("/a%2Fb%20c%3F.txt"));
        assert_eq!(reserved, direct_link_url(1, -1, "a/b c?.txt", "secret")?);
        Ok(())
    }

    #[test]
    fn direct_link_url_rejects_negative_file_ids() {
        let error = direct_link_url(-1, 1, "file.txt", "secret")
            .expect_err("negative file ID must be rejected");
        assert!(error.to_string().contains("must be non-negative"));
    }
}
