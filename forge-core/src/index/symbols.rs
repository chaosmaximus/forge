use serde::Serialize;
use sha2::{Sha256, Digest};

#[derive(Serialize)]
pub struct Symbol {
    pub kind: String,
    pub id: String,
    pub name: String,
    pub file_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_start: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_end: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_id: Option<String>,
}

impl Symbol {
    fn make_id(kind: &str, file_path: &str, name: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(format!("{kind}:{file_path}:{name}"));
        let hash = hasher.finalize();
        format!("{kind}-{}", hex::encode(&hash[..6]))
    }

    pub fn file(path: &str, name: &str, language: &str, size: usize) -> Self {
        Self {
            kind: "file".into(), id: Self::make_id("file", path, name),
            name: name.into(), file_path: path.into(),
            language: Some(language.into()), size_bytes: Some(size),
            line_start: None, line_end: None, signature: None, class_id: None,
        }
    }

    pub fn function(name: &str, fp: &str, ls: usize, le: usize, sig: &str) -> Self {
        Self {
            kind: "function".into(), id: Self::make_id("function", fp, name),
            name: name.into(), file_path: fp.into(),
            language: None, size_bytes: None,
            line_start: Some(ls), line_end: Some(le),
            signature: Some(sig.into()), class_id: None,
        }
    }

    pub fn class(name: &str, fp: &str, ls: usize, le: usize) -> Self {
        Self {
            kind: "class".into(), id: Self::make_id("class", fp, name),
            name: name.into(), file_path: fp.into(),
            language: None, size_bytes: None,
            line_start: Some(ls), line_end: Some(le),
            signature: None, class_id: None,
        }
    }

    pub fn method(name: &str, class_name: &str, fp: &str, ls: usize, le: usize, sig: &str) -> Self {
        let class_id = Self::make_id("class", fp, class_name);
        Self {
            kind: "method".into(),
            id: Self::make_id("method", fp, &format!("{class_name}.{name}")),
            name: name.into(), file_path: fp.into(),
            language: None, size_bytes: None,
            line_start: Some(ls), line_end: Some(le),
            signature: Some(sig.into()), class_id: Some(class_id),
        }
    }
}
