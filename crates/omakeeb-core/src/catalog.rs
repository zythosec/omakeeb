//! Definitions the user keeps on disk.
//!
//! A VIA keyboard does not send its layout. The first time one is plugged in,
//! omakeeb asks for the manufacturer's JSON file and writes a copy to
//! `~/.config/omakeeb/definitions`, named by that keyboard's USB ids. The next
//! plug-in finds the copy and does not ask again. A file that does not parse
//! is reported and skipped, so one bad download does not hide the rest.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::layout::Definition;

pub fn definitions_dir() -> PathBuf {
    config_dir().join("definitions")
}

pub fn keymaps_dir() -> PathBuf {
    config_dir().join("keymaps")
}

fn config_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| home.join(".config"));
    base.join("omakeeb")
}

/// One definition file that parsed.
#[derive(Debug)]
pub struct Stored {
    pub path: PathBuf,
    pub definition: Definition,
}

#[derive(Debug)]
pub struct Catalog {
    pub entries: Vec<Stored>,
    pub errors: Vec<String>,
}

impl Catalog {
    pub fn load(dir: &Path) -> Self {
        let mut catalog = Self {
            entries: Vec::new(),
            errors: Vec::new(),
        };
        let Ok(entries) = fs::read_dir(dir) else {
            return catalog;
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .collect();
        paths.sort();
        for path in paths {
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            match fs::read_to_string(&path)
                .map_err(|err| Error::message(err.to_string()))
                .and_then(|text| Definition::parse(&text))
            {
                Ok(definition) => catalog.entries.push(Stored { path, definition }),
                Err(err) => catalog.errors.push(format!("{}: {err}", path.display())),
            }
        }
        catalog
    }

    /// The layout saved for this USB device, preferring the file omakeeb wrote
    /// for it over another JSON that happens to carry the same ids.
    pub fn find(&self, vendor_id: u16, product_id: u16) -> Option<&Definition> {
        let canonical = file_name(vendor_id, product_id);
        let mut fallback = None;
        for entry in &self.entries {
            if entry.definition.vendor_id != vendor_id || entry.definition.product_id != product_id
            {
                continue;
            }
            let is_canonical =
                entry.path.file_name().and_then(|name| name.to_str()) == Some(canonical.as_str());
            if is_canonical {
                return Some(&entry.definition);
            }
            if fallback.is_none() {
                fallback = Some(&entry.definition);
            }
        }
        fallback
    }
}

/// File name of the definition bound to one USB device.
pub fn file_name(vendor_id: u16, product_id: u16) -> String {
    format!("{vendor_id:04X}-{product_id:04X}.json")
}

/// Remember a VIA definition for the keyboard that is plugged in.
///
/// The copy is stamped with that keyboard's USB ids, so a draft file whose
/// own ids are missing or still say "example" still matches this device next
/// time. Returns the definition that was written.
pub fn remember(json: &str, vendor_id: u16, product_id: u16) -> Result<(Definition, PathBuf)> {
    remember_in(&definitions_dir(), json, vendor_id, product_id)
}

pub fn remember_in(
    dir: &Path,
    json: &str,
    vendor_id: u16,
    product_id: u16,
) -> Result<(Definition, PathBuf)> {
    let mut value: serde_json::Value = serde_json::from_str(json)
        .map_err(|err| Error::message(format!("definition JSON: {err}")))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| Error::message("a VIA definition is a JSON object"))?;
    object.insert(
        "vendorId".to_owned(),
        serde_json::Value::String(format!("0x{vendor_id:04X}")),
    );
    object.insert(
        "productId".to_owned(),
        serde_json::Value::String(format!("0x{product_id:04X}")),
    );
    let stamped = serde_json::to_string_pretty(&value)
        .map_err(|err| Error::message(format!("definition JSON: {err}")))?;
    let definition = Definition::parse(&stamped)?;
    ensure_dir(dir)?;
    let path = dir.join(file_name(vendor_id, product_id));
    fs::write(&path, &stamped)
        .map_err(|err| Error::message(format!("cannot write {}: {err}", path.display())))?;
    Ok((definition, path))
}

pub fn ensure_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)
        .map_err(|err| Error::message(format!("cannot create {}: {err}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_directory_of_definitions_skips_a_broken_file() {
        let dir = std::env::temp_dir().join(format!("omakeeb-catalog-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("good.json"),
            r#"{"name":"Good","vendorId":"0x1111","productId":"0x2222","matrix":{"rows":1,"cols":1},"layouts":{"keymap":[["0,0"]]}}"#,
        )
        .unwrap();
        fs::write(dir.join("bad.json"), "{").unwrap();
        fs::write(dir.join("notes.txt"), "ignore me").unwrap();
        let catalog = Catalog::load(&dir);
        assert_eq!(catalog.entries.len(), 1);
        assert_eq!(catalog.errors.len(), 1);
        assert!(catalog.find(0x1111, 0x2222).is_some());
        assert!(catalog.find(0x1111, 0x0000).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_provided_layout_is_remembered_under_the_keyboards_ids() {
        let dir = std::env::temp_dir().join(format!("omakeeb-remember-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let source = r#"{"name":"Draft","vendorId":"0x0000","productId":"0x0000","matrix":{"rows":1,"cols":1},"layouts":{"keymap":[["0,0"]]}}"#;
        let (definition, path) = remember_in(&dir, source, 0xFEED, 0x0001).unwrap();
        assert_eq!(definition.vendor_id, 0xFEED);
        assert_eq!(definition.product_id, 0x0001);
        assert!(path.ends_with("FEED-0001.json"));
        let again = Catalog::load(&dir);
        assert_eq!(again.find(0xFEED, 0x0001).unwrap().name, "Draft");
        // A second copy with the same ids does not hide the one we wrote.
        fs::write(
            dir.join("other.json"),
            r#"{"name":"Other","vendorId":"0xFEED","productId":"0x0001","matrix":{"rows":1,"cols":2},"layouts":{"keymap":[["0,0","0,1"]]}}"#,
        )
        .unwrap();
        let catalog = Catalog::load(&dir);
        assert_eq!(catalog.find(0xFEED, 0x0001).unwrap().name, "Draft");
        let _ = fs::remove_dir_all(&dir);
    }
}
