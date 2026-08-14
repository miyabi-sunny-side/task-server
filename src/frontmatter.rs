use serde_norway::{Mapping, Value};

use crate::error::Error;

#[derive(Debug, Clone)]
pub struct Document {
    pub frontmatter: Mapping,
    pub body: Vec<u8>,
}

/// Split YAML frontmatter from the markdown body. Body bytes after the closing
/// fence are kept verbatim.
pub fn split_document(bytes: &[u8]) -> Result<Document, Error> {
    let after_open = if bytes.starts_with(b"---\r\n") {
        &bytes[5..]
    } else if bytes.starts_with(b"---\n") {
        &bytes[4..]
    } else {
        return Err(Error::Frontmatter("missing opening --- fence".into()));
    };

    if let Some(body) = body_after_closing_fence(after_open) {
        return document_from_yaml(b"", body);
    }

    let mut index = 0;
    while index < after_open.len() {
        if after_open[index] == b'\n' {
            let rest = &after_open[index + 1..];
            if let Some(body) = body_after_closing_fence(rest) {
                return document_from_yaml(&after_open[..=index], body);
            }
        }
        index += 1;
    }
    Err(Error::Frontmatter("missing closing --- fence".into()))
}

fn body_after_closing_fence(at_line_start: &[u8]) -> Option<&[u8]> {
    if at_line_start.starts_with(b"---\r\n") {
        Some(&at_line_start[5..])
    } else if at_line_start.starts_with(b"---\n") {
        Some(&at_line_start[4..])
    } else if at_line_start == b"---" {
        Some(&[])
    } else {
        None
    }
}

fn document_from_yaml(yaml: &[u8], body: &[u8]) -> Result<Document, Error> {
    let yaml_text = std::str::from_utf8(yaml).map_err(|err| Error::Frontmatter(err.to_string()))?;
    let value: Value = if yaml_text.trim().is_empty() {
        Value::Mapping(Mapping::new())
    } else {
        serde_norway::from_str(yaml_text)?
    };
    let frontmatter = match value {
        Value::Mapping(map) => map,
        Value::Null => Mapping::new(),
        _ => return Err(Error::Frontmatter("frontmatter is not a mapping".into())),
    };
    Ok(Document {
        frontmatter,
        body: body.to_vec(),
    })
}

/// Join as `---\n` + serialized YAML + `---\n` + original body bytes.
pub fn join_document(doc: &Document) -> Result<Vec<u8>, Error> {
    let yaml = serialize_mapping(&doc.frontmatter)?;
    let mut out = Vec::from(b"---\n");
    out.extend_from_slice(yaml.as_bytes());
    out.extend_from_slice(b"---\n");
    out.extend_from_slice(&doc.body);
    Ok(out)
}

fn serialize_mapping(map: &Mapping) -> Result<String, Error> {
    let mut yaml = serde_norway::to_string(&Value::Mapping(map.clone()))?;
    if let Some(rest) = yaml.strip_prefix("---\n") {
        yaml = rest.to_owned();
    }
    if !yaml.ends_with('\n') {
        yaml.push('\n');
    }
    Ok(yaml)
}

#[must_use]
pub fn get_str(map: &Mapping, key: &str) -> Option<String> {
    let value = map.get(Value::String(key.to_owned()))?;
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

pub fn set_str(map: &mut Mapping, key: &str, value: &str) {
    map.insert(
        Value::String(key.to_owned()),
        Value::String(value.to_owned()),
    );
}

#[must_use]
pub fn get_value<'a>(map: &'a Mapping, key: &str) -> Option<&'a Value> {
    map.get(Value::String(key.to_owned()))
}
