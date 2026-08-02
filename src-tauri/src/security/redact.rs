use regex::{Regex, RegexBuilder};

use crate::error::{AppError, ErrorDomain};

const MAX_PATTERNS: usize = 32;
const MAX_PATTERN_CHARS: usize = 512;
const REGEX_SIZE_LIMIT: usize = 1_048_576;
const REPLACEMENT: &str = "[REDACTED]";

const MANDATORY_PATTERN: &str = concat!(
    r"(?is:-----BEGIN [^-\r\n]*PRIVATE KEY-----.*?-----END [^-\r\n]*PRIVATE KEY-----)",
    r"|(?i:\bAuthorization\s*:\s*(?:Bearer\s+)?[^\s,;]+)",
    r"|(?i:\bBearer\s+[a-z0-9._~+/-]{10,}={0,2})",
    r"|(?i:\bsk-(?:ant-)?[a-z0-9_-]{10,})",
    r"|(?i:\b(?:gh[pousr]_[a-z0-9]{10,}|github_pat_[a-z0-9_]{10,}))",
    r"|\b(?:AKIA|ASIA)[A-Z0-9]{16}\b",
    r"|\bAIza[0-9A-Za-z_-]{10,}\b",
    r"|(?i:https?://[^\s]*(?:webhook|hooks?)[^\s]*)",
    r"|(?i:[?&](?:access_token|key|secret)=[^&#\s]+)",
    r#"|(?i:\b[a-z0-9_]*(?:secret|token|password|credential)[a-z0-9_]*\s*=\s*(?:"[^"\r\n]*"|'[^'\r\n]*'|[^\r\n]*))"#,
);

#[derive(Debug)]
pub struct Redactor {
    pattern: Regex,
}

impl Redactor {
    pub fn compile(patterns: &[String]) -> Result<Self, AppError> {
        if patterns.len() > MAX_PATTERNS
            || patterns
                .iter()
                .any(|pattern| pattern.chars().count() > MAX_PATTERN_CHARS)
        {
            return Err(invalid_pattern());
        }

        for pattern in patterns {
            let compiled = bounded_regex(pattern)?;
            if compiled.is_match("") {
                return Err(invalid_pattern());
            }
        }

        let mut combined = String::from(MANDATORY_PATTERN);
        for pattern in patterns {
            combined.push_str("|(?:");
            combined.push_str(pattern);
            combined.push(')');
        }

        Ok(Self {
            pattern: bounded_regex(&combined)?,
        })
    }

    pub fn redact(&self, input: &str) -> String {
        self.pattern.replace_all(input, REPLACEMENT).into_owned()
    }
}

fn bounded_regex(pattern: &str) -> Result<Regex, AppError> {
    RegexBuilder::new(pattern)
        .size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|_| invalid_pattern())
}

fn invalid_pattern() -> AppError {
    AppError {
        domain: ErrorDomain::Configuration,
        code: "configuration.redaction_pattern_invalid".into(),
        message: "redaction pattern configuration is invalid".into(),
        suggested_action: None,
    }
}

#[cfg(test)]
mod tests {
    use super::Redactor;

    #[test]
    fn removes_tokens_webhooks_private_keys_and_named_secrets() {
        let input = concat!(
            "Authorization: Bearer abc.def.ghi\n",
            "Bearer standalone.jwt.token\n",
            "OPENAI_API_KEY=sk-test-1234567890\n",
            "ANTHROPIC_API_KEY=sk-ant-api03-fake-secret\n",
            "GITHUB_TOKEN=github_pat_11AA22BB33CC44DD55\n",
            "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE\n",
            "AZURE_CLIENT_SECRET=azure-secret-value\n",
            "GOOGLE_API_KEY=AIzaSyA-fake-cloud-key\n",
            "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=fake-webhook-secret\n",
            "https://example.test/callback?access_token=access-value&key=key-value&secret=secret-value\n",
            "databasePassword=database-password\n",
            "plainPassword=correct horse battery staple\n",
            "service_CREDENTIAL=service-credential\n",
            "-----BEGIN PRIVATE KEY-----\nfake-private-material\n-----END PRIVATE KEY-----"
        );

        let output = Redactor::compile(&[]).unwrap().redact(input);

        for secret in [
            "abc.def.ghi",
            "standalone.jwt.token",
            "sk-test",
            "sk-ant-api03",
            "github_pat_",
            "AKIAIOSFODNN7EXAMPLE",
            "azure-secret-value",
            "AIzaSyA-fake-cloud-key",
            "fake-webhook-secret",
            "access-value",
            "key-value",
            "secret-value",
            "database-password",
            "horse battery staple",
            "service-credential",
            "BEGIN PRIVATE KEY",
            "fake-private-material",
        ] {
            assert!(!output.contains(secret), "secret family leaked: {secret}");
        }
        assert!(output.contains("[REDACTED]"));
    }

    #[test]
    fn applies_custom_patterns_without_echoing_invalid_input() {
        let redactor = Redactor::compile(&[r"customer-\d+".into()]).unwrap();
        assert_eq!(
            redactor.redact("ticket customer-12345 is ready"),
            "ticket [REDACTED] is ready"
        );

        let secret_pattern = "(?P<private_customer_value>";
        let error = Redactor::compile(&[secret_pattern.into()]).unwrap_err();
        assert_eq!(error.code, "configuration.redaction_pattern_invalid");
        assert!(!error.message.contains(secret_pattern));
    }

    #[test]
    fn bounds_custom_pattern_count_and_unicode_length() {
        assert!(Redactor::compile(&vec!["safe".into(); 32]).is_ok());
        assert!(Redactor::compile(&["密".repeat(512)]).is_ok());

        let count_error = Redactor::compile(&vec!["safe".into(); 33]).unwrap_err();
        assert_eq!(count_error.code, "configuration.redaction_pattern_invalid");

        let length_error = Redactor::compile(&["密".repeat(513)]).unwrap_err();
        assert_eq!(length_error.code, "configuration.redaction_pattern_invalid");
    }
}
