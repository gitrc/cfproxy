use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct MockRule {
    pub path_pattern: String,  // exact match or prefix with *
    pub method: Option<String>,  // None = match any method
    pub status: u16,
    pub content_type: String,
    pub body: String,
    pub hit_count: u64,
}

impl MockRule {
    /// Parse from CLI format: "METHOD /path:status:body" or "/path:status:body"
    pub fn parse(input: &str) -> Option<Self> {
        let input = input.trim();
        if input.is_empty() {
            return None;
        }

        // Determine if there's a method prefix (e.g. "POST /path:status:body")
        let (method, rest) = if let Some(space_idx) = input.find(' ') {
            let potential_method = &input[..space_idx];
            // Only treat it as a method if it's all uppercase letters
            if potential_method.chars().all(|c| c.is_ascii_uppercase()) && !potential_method.is_empty() {
                (Some(potential_method.to_string()), input[space_idx + 1..].trim())
            } else {
                (None, input)
            }
        } else {
            (None, input)
        };

        // rest should be "/path:status[:body]"
        if !rest.starts_with('/') {
            return None;
        }

        // Split on ':' - first part is path, second is status, rest is body
        let mut parts = rest.splitn(3, ':');
        let path_pattern = parts.next()?.to_string();
        let status_str = parts.next()?;
        let status: u16 = status_str.parse().ok()?;
        let body = parts.next().unwrap_or("").to_string();

        // Infer content_type from body
        let content_type = if body.starts_with('{') || body.starts_with('[') {
            "application/json".to_string()
        } else {
            "text/plain".to_string()
        };

        Some(Self {
            path_pattern,
            method,
            status,
            content_type,
            body,
            hit_count: 0,
        })
    }

    /// Check if this rule matches the given method and path
    pub fn matches(&self, method: &str, path: &str) -> bool {
        // Check method filter
        if let Some(ref m) = self.method {
            if !m.eq_ignore_ascii_case(method) {
                return false;
            }
        }

        // Check path pattern
        if self.path_pattern.ends_with('*') {
            let prefix = &self.path_pattern[..self.path_pattern.len() - 1];
            path.starts_with(prefix)
        } else {
            path == self.path_pattern
        }
    }
}

pub type MockRules = Arc<RwLock<Vec<MockRule>>>;

pub fn new_rules() -> MockRules {
    Arc::new(RwLock::new(Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_rule() {
        let rule = MockRule::parse("/api/test:200:OK").unwrap();
        assert_eq!(rule.path_pattern, "/api/test");
        assert_eq!(rule.status, 200);
        assert_eq!(rule.body, "OK");
    }

    #[test]
    fn parse_rule_with_method() {
        let rule = MockRule::parse("POST /api/test:201:{\"id\":1}").unwrap();
        assert_eq!(rule.method, Some("POST".to_string()));
        assert_eq!(rule.status, 201);
    }

    #[test]
    fn parse_rule_no_body() {
        let rule = MockRule::parse("/health:200").unwrap();
        assert_eq!(rule.body, "");
    }

    #[test]
    fn parse_invalid_rule() {
        assert!(MockRule::parse("garbage").is_none());
        assert!(MockRule::parse("").is_none());
    }

    #[test]
    fn wildcard_matching() {
        let rule = MockRule::parse("/api/*:200:OK").unwrap();
        assert!(rule.matches("GET", "/api/users"));
        assert!(rule.matches("POST", "/api/test/123"));
        assert!(!rule.matches("GET", "/health"));
    }

    #[test]
    fn exact_matching() {
        let rule = MockRule::parse("/health:200:OK").unwrap();
        assert!(rule.matches("GET", "/health"));
        assert!(!rule.matches("GET", "/health/check"));
    }

    #[test]
    fn method_matching() {
        let rule = MockRule::parse("POST /api/test:200:OK").unwrap();
        assert!(rule.matches("POST", "/api/test"));
        assert!(!rule.matches("GET", "/api/test"));
    }
}
