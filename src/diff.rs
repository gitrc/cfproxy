use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum DiffLine {
    Same(String),
    Added(String),
    Removed(String),
}

/// Simple line-by-line diff using longest common subsequence.
pub fn diff_lines(left: &str, right: &str) -> Vec<DiffLine> {
    let left_lines: Vec<&str> = if left.is_empty() {
        Vec::new()
    } else {
        left.lines().collect()
    };
    let right_lines: Vec<&str> = if right.is_empty() {
        Vec::new()
    } else {
        right.lines().collect()
    };

    let n = left_lines.len();
    let m = right_lines.len();

    if n == 0 && m == 0 {
        return Vec::new();
    }

    // Build LCS table
    let mut table = vec![vec![0u32; m + 1]; n + 1];
    for i in 1..=n {
        for j in 1..=m {
            if left_lines[i - 1] == right_lines[j - 1] {
                table[i][j] = table[i - 1][j - 1] + 1;
            } else {
                table[i][j] = table[i - 1][j].max(table[i][j - 1]);
            }
        }
    }

    // Backtrack to produce diff
    let mut result = Vec::new();
    let mut i = n;
    let mut j = m;
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && left_lines[i - 1] == right_lines[j - 1] {
            result.push(DiffLine::Same(left_lines[i - 1].to_string()));
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || table[i][j - 1] >= table[i - 1][j]) {
            result.push(DiffLine::Added(right_lines[j - 1].to_string()));
            j -= 1;
        } else {
            result.push(DiffLine::Removed(left_lines[i - 1].to_string()));
            i -= 1;
        }
    }

    result.reverse();
    result
}

/// Generate a diff summary of two HTTP requests.
pub fn diff_requests(a: &crate::event::HttpRequest, b: &crate::event::HttpRequest) -> Vec<DiffLine> {
    let mut result = Vec::new();

    // Compare method + path
    if a.method != b.method || a.path != b.path {
        result.push(DiffLine::Removed(format!("{} {}", a.method, a.path)));
        result.push(DiffLine::Added(format!("{} {}", b.method, b.path)));
    } else {
        result.push(DiffLine::Same(format!("{} {}", a.method, a.path)));
    }

    result.push(DiffLine::Same(String::new()));

    // Compare status
    if a.status != b.status {
        result.push(DiffLine::Removed(format!("Status: {}", a.status)));
        result.push(DiffLine::Added(format!("Status: {}", b.status)));
    } else {
        result.push(DiffLine::Same(format!("Status: {}", a.status)));
    }

    result.push(DiffLine::Same(String::new()));

    // Compare duration
    let a_dur = format!("Duration: {}ms", a.duration.as_millis());
    let b_dur = format!("Duration: {}ms", b.duration.as_millis());
    if a_dur != b_dur {
        result.push(DiffLine::Removed(a_dur));
        result.push(DiffLine::Added(b_dur));
    } else {
        result.push(DiffLine::Same(a_dur));
    }

    result.push(DiffLine::Same(String::new()));
    result.push(DiffLine::Same("── Request Headers ──".to_string()));

    // Build header maps for request headers
    let a_req_headers: BTreeMap<&str, &str> = a
        .request_headers
        .iter()
        .map(|h| (h.name.as_str(), h.value.as_str()))
        .collect();
    let b_req_headers: BTreeMap<&str, &str> = b
        .request_headers
        .iter()
        .map(|h| (h.name.as_str(), h.value.as_str()))
        .collect();

    diff_header_maps(&a_req_headers, &b_req_headers, &mut result);

    result.push(DiffLine::Same(String::new()));
    result.push(DiffLine::Same("── Response Headers ──".to_string()));

    let a_resp_headers: BTreeMap<&str, &str> = a
        .response_headers
        .iter()
        .map(|h| (h.name.as_str(), h.value.as_str()))
        .collect();
    let b_resp_headers: BTreeMap<&str, &str> = b
        .response_headers
        .iter()
        .map(|h| (h.name.as_str(), h.value.as_str()))
        .collect();

    diff_header_maps(&a_resp_headers, &b_resp_headers, &mut result);

    // Compare bodies
    result.push(DiffLine::Same(String::new()));
    result.push(DiffLine::Same("── Request Body ──".to_string()));
    let left_body = a.request_body.as_deref().unwrap_or("");
    let right_body = b.request_body.as_deref().unwrap_or("");
    result.extend(diff_lines(left_body, right_body));

    result.push(DiffLine::Same(String::new()));
    result.push(DiffLine::Same("── Response Body ──".to_string()));
    let left_resp = a.response_body.as_deref().unwrap_or("");
    let right_resp = b.response_body.as_deref().unwrap_or("");
    result.extend(diff_lines(left_resp, right_resp));

    result
}

fn diff_header_maps<'a>(
    a: &BTreeMap<&'a str, &'a str>,
    b: &BTreeMap<&'a str, &'a str>,
    result: &mut Vec<DiffLine>,
) {
    // Collect all keys in sorted order
    let mut all_keys: Vec<&str> = a.keys().chain(b.keys()).copied().collect();
    all_keys.sort();
    all_keys.dedup();

    for key in all_keys {
        match (a.get(key), b.get(key)) {
            (Some(va), Some(vb)) if va == vb => {
                result.push(DiffLine::Same(format!("{}: {}", key, va)));
            }
            (Some(va), Some(vb)) => {
                result.push(DiffLine::Removed(format!("{}: {}", key, va)));
                result.push(DiffLine::Added(format!("{}: {}", key, vb)));
            }
            (Some(va), None) => {
                result.push(DiffLine::Removed(format!("{}: {}", key, va)));
            }
            (None, Some(vb)) => {
                result.push(DiffLine::Added(format!("{}: {}", key, vb)));
            }
            (None, None) => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_identical() {
        let result = diff_lines("hello\nworld", "hello\nworld");
        assert!(result.iter().all(|d| matches!(d, DiffLine::Same(_))));
    }

    #[test]
    fn diff_added_line() {
        let result = diff_lines("a\nb", "a\nb\nc");
        assert!(result.contains(&DiffLine::Added("c".to_string())));
    }

    #[test]
    fn diff_removed_line() {
        let result = diff_lines("a\nb\nc", "a\nc");
        assert!(result.contains(&DiffLine::Removed("b".to_string())));
    }

    #[test]
    fn diff_changed_line() {
        let result = diff_lines("a\nold\nc", "a\nnew\nc");
        assert!(result.contains(&DiffLine::Removed("old".to_string())));
        assert!(result.contains(&DiffLine::Added("new".to_string())));
    }

    #[test]
    fn diff_empty_inputs() {
        let result = diff_lines("", "");
        assert!(result.is_empty() || result.iter().all(|d| matches!(d, DiffLine::Same(_))));
    }

    #[test]
    fn diff_requests_same_status() {
        use crate::event::HttpRequest;
        use std::time::Duration;
        let req = HttpRequest {
            method: "GET".into(),
            path: "/test".into(),
            status: 200,
            duration: Duration::from_millis(10),
            remote_ip: None,
            country: None,
            user_agent: None,
            request_headers: Vec::new(),
            response_headers: Vec::new(),
            request_size: 0,
            response_size: 0,
            request_body: None,
            response_body: None,
            is_websocket: false,
            ws_frames: Vec::new(),
            is_mock: false,
            timestamp: chrono::Local::now(),
        };
        let diff = diff_requests(&req, &req);
        assert!(diff.iter().all(|d| matches!(d, DiffLine::Same(_))));
    }

    #[test]
    fn diff_requests_different_status() {
        use crate::event::HttpRequest;
        use std::time::Duration;
        let req_a = HttpRequest {
            method: "GET".into(),
            path: "/test".into(),
            status: 200,
            duration: Duration::from_millis(10),
            remote_ip: None,
            country: None,
            user_agent: None,
            request_headers: Vec::new(),
            response_headers: Vec::new(),
            request_size: 0,
            response_size: 0,
            request_body: None,
            response_body: None,
            is_websocket: false,
            ws_frames: Vec::new(),
            is_mock: false,
            timestamp: chrono::Local::now(),
        };
        let mut req_b = req_a.clone();
        req_b.status = 404;
        let diff = diff_requests(&req_a, &req_b);
        assert!(diff.iter().any(|d| matches!(d, DiffLine::Removed(s) if s.contains("200"))));
        assert!(diff.iter().any(|d| matches!(d, DiffLine::Added(s) if s.contains("404"))));
    }
}
