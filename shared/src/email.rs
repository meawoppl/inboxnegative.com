use std::{borrow::Cow, collections::HashSet};

use chrono::{DateTime, Utc};
use log;
use mail_parser::{Address, Message, MessageParser, MimeHeaders};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Attachment {
    pub filename: Option<String>,
    pub content_type: String,
    pub size: usize,
    pub data: Vec<u8>,
    pub attachment_id: String, // Unique identifier for this attachment
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Email {
    pub sender: String,
    pub destination: String,
    pub uuid: Uuid,
    pub time_of_receipt: DateTime<Utc>,
    pub raw: String, // New field for email data
    #[serde(default)]
    pub attachments: Vec<Attachment>, // List of attachments
}

fn clean_html(input: String) -> String {
    let tag_blacklist: HashSet<&str> = HashSet::from_iter(vec!["script", "iframe"]);

    let mut builder = ammonia::Builder::new();

    let cleaner = builder
        .clean_content_tags(tag_blacklist)
        .add_tag_attributes("img", &["src"])
        .add_tag_attributes("a", &["href"])
        .add_generic_attributes(vec!["class", "style", "id", "href", "src", "dir"])
        .add_tags(vec!["img", "style", "section", "a"])
        .add_url_schemes(vec!["data", "cid", "http", "https", "mailto"]);

    // Clean the HTML content
    let cleaned = cleaner.clean(&input).to_string();

    // Fix common email issues that cause double newlines
    // 1. Remove excessive newlines between tags
    let re_newlines = regex::Regex::new(r"\n\s*\n").unwrap();
    let fixed = re_newlines.replace_all(&cleaned, "\n").to_string();

    // Wrap content in a container to ensure proper styling
    if !fixed.trim().starts_with("<") || !fixed.trim().contains("<body") {
        // If it's just a fragment, wrap it in a basic structure
        format!(r#"<div class="email-content-wrapper">{}</div>"#, fixed)
    } else {
        fixed
    }
}

fn format_address(address: &Address) -> String {
    match address {
        Address::Group(mb) => {
            format!("{:?}", mb)
        }
        Address::List(add) => add
            .iter()
            .map(|addr| {
                let cp = addr.clone();
                format!(
                    "{} <{}>",
                    cp.name.unwrap_or(Cow::from("")),
                    cp.address.unwrap_or(Cow::from(""))
                )
            })
            .collect::<Vec<String>>()
            .join(", "),
    }
}

const MAX_ATTACHMENT_SIZE: usize = 10 * 1024 * 1024; // 10 MB per file
const MAX_TOTAL_ATTACHMENTS_SIZE: usize = 50 * 1024 * 1024; // 50 MB total

fn extract_attachments_from_message(message: &Message, email_uuid: &Uuid) -> Vec<Attachment> {
    let mut attachments = Vec::new();
    let mut attachment_index = 0;
    let mut total_size = 0;

    log::info!("Starting attachment extraction for email {}", email_uuid);

    // Iterate through all attachments in the message
    for attachment in message.attachments() {
        log::info!("Found attachment: {:?}", attachment.attachment_name());
        let filename = attachment.attachment_name().map(|s| s.to_string());

        // Get the content type
        let content_type_str = if let Some(ct) = attachment.content_type() {
            format!(
                "{}/{}",
                ct.c_type,
                ct.c_subtype
                    .as_ref()
                    .map(|s| s.as_ref())
                    .unwrap_or("octet-stream")
            )
        } else {
            "application/octet-stream".to_string()
        };

        // Get the attachment body data
        let data = match &attachment.body {
            mail_parser::PartType::Text(t) => t.as_bytes().to_vec(),
            mail_parser::PartType::Binary(b) => b.to_vec(),
            mail_parser::PartType::Html(h) => h.as_bytes().to_vec(),
            _ => continue, // Skip multipart or message parts
        };

        let size = data.len();

        // Skip if single attachment is too large
        if size > MAX_ATTACHMENT_SIZE {
            log::warn!(
                "Skipping attachment {} - size {} exceeds max {} bytes",
                filename
                    .as_ref()
                    .unwrap_or(&format!("attachment_{}", attachment_index)),
                size,
                MAX_ATTACHMENT_SIZE
            );
            attachment_index += 1;
            continue;
        }

        // Check if adding this attachment would exceed total size limit
        if total_size + size > MAX_TOTAL_ATTACHMENTS_SIZE {
            log::warn!(
                "Skipping attachment {} - would exceed total size limit of {} bytes (current: {}, adding: {})",
                filename.as_ref().unwrap_or(&format!("attachment_{}", attachment_index)),
                MAX_TOTAL_ATTACHMENTS_SIZE,
                total_size,
                size
            );
            attachment_index += 1;
            continue;
        }

        total_size += size;

        // Create unique ID for this attachment
        let attachment_id = format!("{}-{}", email_uuid, attachment_index);

        log::info!(
            "Adding attachment: {} ({} bytes, type: {})",
            filename.as_ref().unwrap_or(&"unnamed".to_string()),
            size,
            content_type_str
        );

        attachments.push(Attachment {
            filename: filename.or_else(|| Some(format!("attachment_{}", attachment_index))),
            content_type: content_type_str,
            size,
            data,
            attachment_id,
        });

        attachment_index += 1;
    }

    log::info!("Extracted {} attachment(s)", attachments.len());

    attachments
}

impl Email {
    pub fn new(sender: String, destination: String, raw: String) -> Self {
        let mut email = Email {
            sender,
            destination,
            uuid: Uuid::new_v4(),
            time_of_receipt: Utc::now(),
            raw,
            attachments: Vec::new(),
        };

        // Extract attachments during construction
        email.extract_attachments();
        email
    }

    fn extract_attachments(&mut self) {
        if let Some(parsed) = self.parsed() {
            self.attachments = extract_attachments_from_message(&parsed, &self.uuid);
        }
    }

    pub fn parsed(&self) -> Option<Message<'_>> {
        let b = self.raw.as_bytes();
        MessageParser::default().parse(b)
    }

    pub fn from_address(&self) -> Option<String> {
        let message = self.parsed()?;
        message.from().map(format_address)
    }

    pub fn subject(&self) -> Option<String> {
        self.parsed()?.subject().map(|s| s.to_string())
    }

    pub fn cleansed_html(&self) -> Option<String> {
        let parsed = self.parsed()?;
        let html = parsed.body_html(0)?;

        // Process the HTML content
        let html_str = html.to_string();

        // Clean HTML content using ammonia
        let cleaned = clean_html(html_str);

        // Wrap the content in a div with proper styling if not already wrapped
        Some(cleaned)
    }

    pub fn plaintext(&self) -> Option<String> {
        let parsed = self.parsed()?;
        let plaintext = parsed.body_text(0)?;
        Some(plaintext.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialization_roundtrip() {
        let email = Email {
            sender: String::from("foo.bar@baz.qux"),
            destination: String::from("example@example.com"),
            uuid: Uuid::new_v4(),
            time_of_receipt: Utc::now(),
            raw: String::from("Raw email data"),
            attachments: Vec::new(),
        };

        let serialized = serde_json::to_string(&email).unwrap();
        let deserialized: Email = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized.destination, "example@example.com");
        assert_eq!(deserialized.raw, "Raw email data");
        assert_eq!(deserialized.attachments.len(), 0);
    }

    #[test]
    fn test_clean_html_script_tags() {
        let input = r#"<script>alert("Hello, World!")</script>"#;
        let expected = r#"<div class="email-content-wrapper"></div>"#;

        let cleaned = clean_html(input.to_string());
        assert_eq!(cleaned, expected);
    }

    #[test]
    fn test_clean_html_onaction() {
        {
            let input = r#"<img onclick="alert('Hello, World!')">"#;
            let expected = r#"<div class="email-content-wrapper"><img></div>"#;

            let cleaned = clean_html(input.to_string());
            assert_eq!(cleaned, expected);
        }

        {
            let input = r#"<div onmouseover="alert('Hello, World!')"></div>"#;
            let expected = r#"<div class="email-content-wrapper"><div></div></div>"#;

            let cleaned = clean_html(input.to_string());
            assert_eq!(cleaned, expected);
        }
    }

    #[test]
    fn test_clean_img_tags_data_url_kept() {
        // These are great, no http, keep them
        let input = r#"<img src="data:image/png;base64,base64encodedimage">"#;
        let expected = r#"<div class="email-content-wrapper"><img src="data:image/png;base64,base64encodedimage"></div>"#;

        let cleaned = clean_html(input.to_string());
        assert_eq!(cleaned, expected);
    }

    #[test]
    fn test_clean_image_tags_http_kept() {
        // HTTP URLs should now be kept for images
        let input = r#"<img src="http://foo.bar/baz">"#;
        let expected = r#"<div class="email-content-wrapper"><img src="http://foo.bar/baz"></div>"#;

        let cleaned = clean_html(input.to_string());
        assert_eq!(cleaned, expected);
    }

    #[test]
    fn test_clean_links_preserved() {
        // Links should be preserved with their href attributes
        // Note: ammonia adds rel="noopener noreferrer" for security
        let input = r#"<a href="https://example.com">Click here</a>"#;
        let expected = r#"<div class="email-content-wrapper"><a href="https://example.com" rel="noopener noreferrer">Click here</a></div>"#;

        let cleaned = clean_html(input.to_string());
        assert_eq!(cleaned, expected);
    }

    #[test]
    fn test_clean_mailto_links_preserved() {
        // Mailto links should also be preserved
        // Note: ammonia adds rel="noopener noreferrer" for security
        let input = r#"<a href="mailto:test@example.com">Email me</a>"#;
        let expected = r#"<div class="email-content-wrapper"><a href="mailto:test@example.com" rel="noopener noreferrer">Email me</a></div>"#;

        let cleaned = clean_html(input.to_string());
        assert_eq!(cleaned, expected);
    }

    #[test]
    fn test_email_with_attachments() {
        let email_with_attachment = r#"From: sender@example.com
To: recipient@example.com
Subject: Test with attachment
MIME-Version: 1.0
Content-Type: multipart/mixed; boundary="boundary123"

--boundary123
Content-Type: text/plain; charset="UTF-8"

This is the email body.

--boundary123
Content-Type: application/pdf; name="test.pdf"
Content-Disposition: attachment; filename="test.pdf"
Content-Transfer-Encoding: base64

JVBERi0xLjQKJeLjz9MKMSAwIG9iago8PC9UeXBlL0NhdGFsb2cvUGFnZXMgMiAwIFI+PgplbmRvYmoKMiAwIG9iago8PC9UeXBlL1BhZ2VzL0NvdW50IDEvS2lkc1szIDAgUl0+PgplbmRvYmoKMyAwIG9iago8PC9UeXBlL1BhZ2UvTWVkaWFCb3hbMCAwIDMgM10+PgplbmRvYmoKeHJlZgowIDQKMDAwMDAwMDAwMCA2NTUzNSBmDQowMDAwMDAwMDA5IDAwMDAwIG4NCjAwMDAwMDAwNTIgMDAwMDAgbg0KMDAwMDAwMDEwOSAwMDAwMCBuDQp0cmFpbGVyCjw8L1NpemUgNC9Sb290IDEgMCBSPj4Kc3RhcnR4cmVmCjE1NgolJUVPRgo=

--boundary123--
"#;

        let email = Email::new(
            "sender@example.com".to_string(),
            "recipient@example.com".to_string(),
            email_with_attachment.to_string(),
        );

        assert_eq!(email.attachments.len(), 1);
        assert_eq!(email.attachments[0].filename, Some("test.pdf".to_string()));
        assert_eq!(email.attachments[0].content_type, "application/pdf");
        assert!(email.attachments[0].size > 0);
    }

    #[test]
    fn test_attachment_size_limits() {
        // Test that attachments within limits are accepted
        let small_attachment = "X".repeat(100); // 100 bytes - well within limit

        let email_with_small_attachment = format!(
            r#"From: sender@example.com
To: recipient@example.com
Subject: Test with small attachment
MIME-Version: 1.0
Content-Type: multipart/mixed; boundary="boundary456"

--boundary456
Content-Type: text/plain; charset="UTF-8"

This is the email body.

--boundary456
Content-Type: text/plain; name="small.txt"
Content-Disposition: attachment; filename="small.txt"

{}

--boundary456--
"#,
            small_attachment
        );

        let email = Email::new(
            "sender@example.com".to_string(),
            "recipient@example.com".to_string(),
            email_with_small_attachment,
        );

        assert_eq!(email.attachments.len(), 1);
        assert_eq!(email.attachments[0].filename, Some("small.txt".to_string()));
    }

    #[test]
    fn test_real_email_content() {
        let content = r#"Received: by mail-lf1-f42.google.com with SMTP id 2adb3069b0e04-53959a88668so4322824e87.2
for <414a50ed6b56d4b9@inboxnegative.com>; Mon, 30 Sep 2024 22:05:48 -0700 (PDT)
DKIM-Signature: v=1; a=rsa-sha256; c=relaxed/relaxed;
d=exclosure-io.20230601.gappssmtp.com; s=20230601; t=1727759147; x=1728363947; darn=inboxnegative.com;
h=to:subject:message-id:date:from:mime-version:from:to:cc:subject
 :date:message-id:reply-to;
bh=VggzhhrX8PlUHNMp/+1Gac/i5DdCNE+36OJVBHmoaZ8=;
b=mspGSdohb/dhTLA7hJ/RvfAJR0Aqe556aSSegE3qvxr+9W8lJMAw/0GMRcUui8BaeI
 5cqKO6w47qrZDqEXt302XdV8lhZrMSljlEqgnn+v8RXeBM9pW8Hde0H1ce1V8P9erQP6
 yr2n9isYLWsSd3LWe7mnaRqS19PSJy/15+QKtOGbf5uDVVXigEquQnJJnXLSiyn6PTjk
 bkTUJxLWCgMONclAmbAZX2j+sHIE0H8C8vrAl+nGTeFcHrUwIAynSu5NnhJzlc5NZ8VO
 JFNkaduTV7f7pxIM50tMlxG35CY9ITYnN4syUKAPeYc8zKz225xhPBofxdDcNVvgP90Q
 0pSA==
X-Google-DKIM-Signature: v=1; a=rsa-sha256; c=relaxed/relaxed;
d=1e100.net; s=20230601; t=1727759147; x=1728363947;
h=to:subject:message-id:date:from:mime-version:x-gm-message-state
 :from:to:cc:subject:date:message-id:reply-to;
bh=VggzhhrX8PlUHNMp/+1Gac/i5DdCNE+36OJVBHmoaZ8=;
b=HqKlsxxAamkHj95hbACLgqeFUMykGzT47i+0FhgfiysEl4vJUbJI/5sWK3SbN+vWFt
 Dm0/+ZfMAerDujiTWKL1cFPU+5CMjreYITA8nNiXN2LCVtARGTzgi7js0C1RgVG514YW
 Ox6fTFCBvtk61prfQpG6rRNd8QziEokXzftVpf91mcHNTjFoXjUZWmYfuyXeYdjUW0Ka
 asMaEneSFjm2wsvi2k9gWlIPSGQfgXHeX7k2kZsh4v5i2Mgv/OmmtdnqPVTx9nwCpN/Y
 D9lGrlz6Fnw7eV/GMvCkQIrOVr/BY2VauGDVu5H9f+U+9HS8JZNtRjnhNqNr5pXZK/Kf
 skNQ==
X-Gm-Message-State: AOJu0Yw3S/m4jh0J33HM32PPsWNBO+uLVqnhDV1qppz5NZRiwG6/WPSK
dwmwVmqRd5bup3QGdv8W3tw3DskrCaqDxN+7Y6bF1QdfrVuMYEoyTIktyTsmRacDNNnX9uzwLsK
gJFnh8L3OzDAnUnznbmVZph0bR7pxlkbGKoNQSYutxUltOUwPHPI=
X-Google-Smtp-Source: AGHT+IHVcADRYGz+ze80zfysfasP6MxtASv2Oo1iLXnxK5cHkNL/4yh+shhmvkjf154YO3dst2Vv5zpWEs/xZpBhwkw=
X-Received: by 2002:a05:6512:31c1:b0:536:73b5:d971 with SMTP id
2adb3069b0e04-5389fc7feadmr6925019e87.38.1727759147121; Mon, 30 Sep 2024
22:05:47 -0700 (PDT)
MIME-Version: 1.0
From: Matthew Goodman <matt@exclosure.io>
Date: Mon, 30 Sep 2024 22:05:36 -0700
Message-ID: <CA+jJzteLwtEgdztJfZifB9um-u7m-fq+j+gUEEnVi2vc4LJ-8A@mail.gmail.com>
Subject: Recieved!
To: 414a50ed6b56d4b9@inboxnegative.com
Content-Type: multipart/alternative; boundary="00000000000038d5e006236346fe"

--00000000000038d5e006236346fe
Content-Type: text/plain; charset="UTF-8"

Test--MattyG

--00000000000038d5e006236346fe
Content-Type: text/html; charset="UTF-8"

<div dir="ltr"><div><div dir="ltr" class="gmail_signature" data-smartmail="gmail_signature"><div dir="ltr">Test--MattyG</div></div></div></div>

--00000000000038d5e006236346fe--
"#;

        let email = Email::new(
            "foo@bar.baz".to_string(),
            "qux@foo.bar".to_string(),
            content.to_string(),
        );

        assert!(email.parsed().is_some());
        let p = email.parsed().unwrap();
        let _html = p.body_html(0);

        let clensed = email.cleansed_html();
        assert!(clensed.unwrap().contains("Test--MattyG"));
    }
}
