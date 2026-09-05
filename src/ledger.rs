//! Markdown records are the persistent truth. Transactions serialize server
//! operations; each file replacement is atomic, not a multi-file transaction.
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::Value;

use crate::error::Error;
use crate::frontmatter::{Document, join_document, split_document};

const COLLECTIONS: &[&str] = &["tasks", "products", "runs", "archive", "claim_receipts"];

pub struct Store {
    root: PathBuf,
    mutex: Mutex<()>,
    // Kept open for the entire lifetime; closing releases the OS lock.
    _process_lock: File,
}

pub struct StoreAccess<'a> {
    store: &'a Store,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        if path.as_os_str().is_empty() || path.to_string_lossy().trim().is_empty() {
            return Err(Error::Invalid("ledger directory must not be blank".into()));
        }
        fs::create_dir_all(path)?;
        let root = fs::canonicalize(path)?;
        let lock_path = root.join(".lock");
        reject_symlink(&lock_path)?;
        let process_lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        process_lock.try_lock().map_err(|error| {
            Error::Conflict(format!(
                "ledger is already open or cannot be locked: {error}"
            ))
        })?;
        for collection in COLLECTIONS {
            let path = root.join(collection);
            reject_symlink(&path)?;
            fs::create_dir_all(path)?;
        }
        Ok(Self {
            root,
            mutex: Mutex::new(()),
            _process_lock: process_lock,
        })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn transaction<T>(
        &self,
        operation: impl FnOnce(&StoreAccess<'_>) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let _guard = self
            .mutex
            .lock()
            .map_err(|_| Error::Conflict("ledger writer lock poisoned".into()))?;
        operation(&StoreAccess { store: self })
    }

    pub fn list(&self, collection: &str) -> Result<Vec<Value>, Error> {
        self.transaction(|access| access.list(collection))
    }
    pub fn get(&self, collection: &str, id: &str) -> Result<Value, Error> {
        self.transaction(|access| access.get(collection, id))
    }
    pub fn put(&self, collection: &str, id: &str, value: Value) -> Result<Value, Error> {
        self.transaction(|access| access.put(collection, id, value))
    }
    pub fn create(&self, collection: &str, id: &str, value: Value) -> Result<Value, Error> {
        self.transaction(|access| access.create(collection, id, value))
    }
    pub fn update(
        &self,
        collection: &str,
        id: &str,
        operation: impl FnOnce(&mut Value) -> Result<(), Error>,
    ) -> Result<Value, Error> {
        self.transaction(|access| access.update(collection, id, operation))
    }
    pub fn remove(&self, collection: &str, id: &str) -> Result<Value, Error> {
        self.transaction(|access| access.remove(collection, id))
    }
}

impl StoreAccess<'_> {
    fn directory(&self, collection: &str) -> Result<PathBuf, Error> {
        if !COLLECTIONS.contains(&collection) {
            return Err(Error::Invalid(format!("invalid collection: {collection}")));
        }
        let path = self.store.root.join(collection);
        reject_symlink(&path)?;
        Ok(path)
    }

    fn path(&self, collection: &str, id: &str) -> Result<PathBuf, Error> {
        if id.is_empty() {
            return Err(Error::Invalid("record id must not be empty".into()));
        }
        let path = self
            .directory(collection)?
            .join(format!("{}.md", encode_id(id)));
        reject_symlink(&path)?;
        Ok(path)
    }

    pub fn list(&self, collection: &str) -> Result<Vec<Value>, Error> {
        let mut paths = fs::read_dir(self.directory(collection)?)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()?;
        paths.sort();
        let mut records = Vec::new();
        for path in paths {
            if path.extension().is_none_or(|extension| extension != "md") {
                continue;
            }
            reject_symlink(&path)?;
            let stem = path
                .file_stem()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    Error::Invalid(format!("invalid record filename: {}", path.display()))
                })?;
            let id = decode_id(stem)?;
            records.push(read_record(&path, &id)?);
        }
        Ok(records)
    }

    pub fn get(&self, collection: &str, id: &str) -> Result<Value, Error> {
        read_record(&self.path(collection, id)?, id)
    }

    pub fn put(&self, collection: &str, id: &str, mut value: Value) -> Result<Value, Error> {
        let path = self.path(collection, id)?;
        normalize_record(&mut value, id)?;
        let mut metadata = value.as_object().expect("normalized object").clone();
        let body = metadata
            .remove("body")
            .expect("normalized body")
            .as_str()
            .expect("string body")
            .as_bytes()
            .to_vec();
        let frontmatter = serde_norway::to_value(metadata)?;
        let serde_norway::Value::Mapping(frontmatter) = frontmatter else {
            unreachable!("object encodes as mapping")
        };
        let bytes = join_document(&Document { frontmatter, body })?;
        atomic_write(&path, &bytes)?;
        Ok(value)
    }

    pub fn create(&self, collection: &str, id: &str, value: Value) -> Result<Value, Error> {
        if self.path(collection, id)?.try_exists()? {
            return Err(Error::Conflict(format!(
                "record already exists: {collection}/{id}"
            )));
        }
        self.put(collection, id, value)
    }

    pub fn update(
        &self,
        collection: &str,
        id: &str,
        operation: impl FnOnce(&mut Value) -> Result<(), Error>,
    ) -> Result<Value, Error> {
        let mut value = self.get(collection, id)?;
        operation(&mut value)?;
        self.put(collection, id, value)
    }

    pub fn remove(&self, collection: &str, id: &str) -> Result<Value, Error> {
        let value = self.get(collection, id)?;
        let path = self.path(collection, id)?;
        fs::remove_file(&path)?;
        File::open(path.parent().expect("record directory"))?.sync_all()?;
        Ok(value)
    }
}

fn reject_symlink(path: &Path) -> Result<(), Error> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(Error::Invalid(format!(
            "ledger symlinks are not supported: {}",
            path.display()
        ))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn read_record(path: &Path, id: &str) -> Result<Value, Error> {
    let bytes = fs::read(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Error::NotFound(format!("record not found: {id}"))
        } else {
            error.into()
        }
    })?;
    let document = split_document(&bytes)?;
    let mut value = serde_json::to_value(document.frontmatter)?;
    let body = String::from_utf8(document.body)
        .map_err(|error| Error::Invalid(format!("record body must be UTF-8: {error}")))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| Error::Invalid("record metadata must be an object".into()))?;
    if object.contains_key("body") {
        return Err(Error::Invalid(format!(
            "body belongs after the frontmatter: {id}"
        )));
    }
    object.insert("body".into(), Value::String(body));
    normalize_record(&mut value, id)?;
    Ok(value)
}

fn normalize_record(value: &mut Value, id: &str) -> Result<(), Error> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| Error::Invalid("record must be an object".into()))?;
    if let Some(existing) = object.get("id") {
        let matches = match existing {
            Value::String(text) => text == id,
            Value::Number(number) => number.to_string() == id,
            _ => false,
        };
        if !matches {
            return Err(Error::Invalid(format!(
                "record id differs from filename: {id}"
            )));
        }
    } else {
        object.insert("id".into(), Value::String(id.into()));
    }
    match object.get("body") {
        Some(Value::String(_)) => {}
        None => {
            object.insert("body".into(), Value::String(String::new()));
        }
        Some(_) => return Err(Error::Invalid("record body must be a string".into())),
    }
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let parent = path.parent().expect("record directory");
    let temporary = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<(), Error> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn encode_id(id: &str) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::new();
    for byte in id.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            write!(encoded, "%{byte:02X}").expect("String write");
        }
    }
    encoded
}

fn decode_id(encoded: &str) -> Result<String, Error> {
    let invalid = || Error::Invalid(format!("invalid encoded record filename: {encoded}"));
    let mut bytes = Vec::new();
    let raw = encoded.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' {
            let pair = encoded.get(i + 1..i + 3).ok_or_else(invalid)?;
            bytes.push(u8::from_str_radix(pair, 16).map_err(|_| invalid())?);
            i += 3;
        } else {
            bytes.push(raw[i]);
            i += 1;
        }
    }
    let id = String::from_utf8(bytes).map_err(|_| invalid())?;
    if id.is_empty() || encode_id(&id) != encoded {
        return Err(invalid());
    }
    Ok(id)
}
