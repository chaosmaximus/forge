use std::io::{self, BufRead, Write};

use super::request::Request;
use super::response::Response;

/// Encode a Response to a JSON string.
/// NEW-3: Returns a JSON error object on serialization failure instead of panicking.
pub fn encode_response(response: &Response) -> String {
    serde_json::to_string(response)
        .unwrap_or_else(|e| format!(r#"{{"status":"error","message":"serialize error: {e}"}}"#))
}

/// Decode a JSON line into a Request.
pub fn decode_request(line: &str) -> Result<Request, String> {
    serde_json::from_str(line).map_err(|e| format!("decode_request error: {e}"))
}

/// Write a Response as a JSON line (JSON + newline) to a writer.
pub fn write_response<W: Write>(w: &mut W, response: &Response) -> io::Result<()> {
    let line = encode_response(response);
    w.write_all(line.as_bytes())?;
    w.write_all(b"\n")?;
    Ok(())
}

/// Read one line from a BufRead and decode it as a Request.
/// Returns Ok(None) on EOF, Ok(Some(req)) on success, Err on parse failure.
pub fn read_request<R: BufRead>(r: &mut R) -> Result<Option<Request>, String> {
    let mut line = String::new();
    let bytes_read = r
        .read_line(&mut line)
        .map_err(|e| format!("read_request IO error: {e}"))?;
    if bytes_read == 0 {
        return Ok(None);
    }
    let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
    if trimmed.is_empty() {
        return Ok(None);
    }
    decode_request(trimmed).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::request::Request;
    use crate::protocol::response::{Response, ResponseData};
    use crate::types::memory::MemoryType;

    #[test]
    fn test_request_roundtrip() {
        let req = Request::Remember {
            memory_type: MemoryType::Decision,
            title: "Use NDJSON".to_string(),
            content: "Newline-delimited JSON for IPC".to_string(),
            confidence: Some(0.95),
            tags: Some(vec!["protocol".to_string()]),
            project: Some("forge".to_string()),
            metadata: None,
        };

        let json = serde_json::to_string(&req).expect("serialize");
        let decoded: Request = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(req, decoded);

        // Verify specific fields via destructuring
        if let Request::Remember {
            title,
            confidence,
            project,
            tags,
            ..
        } = decoded
        {
            assert_eq!(title, "Use NDJSON");
            assert_eq!(confidence, Some(0.95));
            assert_eq!(project, Some("forge".to_string()));
            assert_eq!(tags, Some(vec!["protocol".to_string()]));
        } else {
            panic!("Expected Remember variant");
        }
    }

    #[test]
    fn test_response_roundtrip() {
        let resp = Response::Ok {
            data: ResponseData::Health {
                decisions: 10,
                lessons: 5,
                patterns: 3,
                preferences: 2,
                edges: 42,
            },
        };

        let encoded = encode_response(&resp);
        let decoded: Response = serde_json::from_str(&encoded).expect("deserialize");

        assert_eq!(resp, decoded);

        if let Response::Ok {
            data:
                ResponseData::Health {
                    decisions,
                    lessons,
                    edges,
                    ..
                },
        } = decoded
        {
            assert_eq!(decisions, 10);
            assert_eq!(lessons, 5);
            assert_eq!(edges, 42);
        } else {
            panic!("Expected Ok(Health) variant");
        }
    }

    #[test]
    fn test_codec_write_read() {
        // Test write_response: write a Shutdown response to a buffer
        let resp = Response::Ok {
            data: ResponseData::Shutdown,
        };
        let mut buf: Vec<u8> = Vec::new();
        write_response(&mut buf, &resp).expect("write_response");

        // Verify it ends with a newline
        assert_eq!(buf.last(), Some(&b'\n'));

        // Verify the written JSON round-trips
        let written_str = std::str::from_utf8(&buf).expect("utf8").trim_end();
        let decoded: Response = serde_json::from_str(written_str).expect("deserialize written");
        assert_eq!(resp, decoded);

        // Test read_request: decode a Health request from a string buffer
        let health_json = r#"{"method":"health"}"#;
        let mut cursor = std::io::Cursor::new(health_json.as_bytes());
        let req = read_request(&mut cursor)
            .expect("read_request ok")
            .expect("Some");
        assert_eq!(req, Request::Health);
    }

    #[test]
    fn test_stored_vs_forgotten_disambiguation() {
        // This test verifies that Stored and Forgotten are distinguishable
        // after the fix from untagged to internally tagged ResponseData.
        let stored = Response::Ok {
            data: ResponseData::Stored {
                id: "abc".to_string(),
            },
        };
        let forgotten = Response::Ok {
            data: ResponseData::Forgotten {
                id: "abc".to_string(),
            },
        };

        let stored_json = encode_response(&stored);
        let forgotten_json = encode_response(&forgotten);

        // They must produce different JSON
        assert_ne!(stored_json, forgotten_json);

        // Both must round-trip correctly
        let stored_decoded: Response = serde_json::from_str(&stored_json).unwrap();
        let forgotten_decoded: Response = serde_json::from_str(&forgotten_json).unwrap();

        assert_eq!(stored, stored_decoded);
        assert_eq!(forgotten, forgotten_decoded);

        // Verify the "kind" discriminator is present
        assert!(stored_json.contains("\"kind\":\"stored\""));
        assert!(forgotten_json.contains("\"kind\":\"forgotten\""));
    }
}
