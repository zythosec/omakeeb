//! Finding a VIA keyboard on Linux hidraw, and exchanging reports with it.
//!
//! The interface is raw HID usage page `0xFF60`, usage `0x61`. Opening it
//! through hidraw leaves the keyboard's normal input interface attached, so
//! the board keeps typing while it is being configured. A libusb open would
//! detach that driver, so this crate does not use one.

use std::ffi::CString;
use std::path::Path;

use hidapi::{HidApi, HidDevice, HidError};

use crate::error::{Error, Result};
use crate::protocol::{self, Report};
use crate::transport::Link;

const VIA_USAGE_PAGE: u16 = 0xFF60;
const VIA_USAGE: u16 = 0x61;

/// A keyboard the scan found and has not opened yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Discovered {
    pub path: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub manufacturer: String,
    pub product: String,
    pub interface: i32,
}

impl Discovered {
    pub fn label(&self) -> String {
        if self.product.is_empty() {
            format!("{:04X}:{:04X}", self.vendor_id, self.product_id)
        } else if self.manufacturer.is_empty() {
            self.product.clone()
        } else {
            format!("{} {}", self.manufacturer, self.product)
        }
    }
}

/// List raw-HID interfaces that advertise VIA's usage page.
pub fn discover() -> Result<Vec<Discovered>> {
    let api = HidApi::new().map_err(|err| Error::message(format!("HID: {err}")))?;
    let mut found = Vec::new();
    for device in api.device_list() {
        let path = device.path().to_string_lossy().into_owned();
        if !is_via(device.usage_page(), device.usage(), &path) {
            continue;
        }
        found.push(Discovered {
            path,
            vendor_id: device.vendor_id(),
            product_id: device.product_id(),
            manufacturer: device.manufacturer_string().unwrap_or("").to_owned(),
            product: device.product_string().unwrap_or("").to_owned(),
            interface: device.interface_number(),
        });
    }
    found.sort_by(|a, b| {
        (&a.vendor_id, &a.product_id, &a.path).cmp(&(&b.vendor_id, &b.product_id, &b.path))
    });
    found.dedup_by(|a, b| a.path == b.path);
    Ok(found)
}

fn is_via(usage_page: u16, usage: u16, path: &str) -> bool {
    if usage_page == VIA_USAGE_PAGE && usage == VIA_USAGE {
        return true;
    }
    // hidraw sometimes reports a usage page of zero. The report descriptor in
    // sysfs still names the collection.
    if usage_page != 0 && usage_page != VIA_USAGE_PAGE {
        return false;
    }
    descriptor_is_via(path)
}

fn descriptor_is_via(path: &str) -> bool {
    let Some(name) = Path::new(path).file_name() else {
        return false;
    };
    let descriptor = Path::new("/sys/class/hidraw")
        .join(name)
        .join("device/report_descriptor");
    let Ok(bytes) = std::fs::read(descriptor) else {
        return false;
    };
    let page = bytes.windows(3).any(|window| window == [0x06, 0x60, 0xFF]);
    let usage = bytes.windows(2).any(|window| window == [0x09, 0x61]);
    page && usage
}

/// An open VIA interface.
#[derive(Debug)]
pub struct HidLink {
    device: HidDevice,
    path: String,
}

impl HidLink {
    pub fn open(path: &str) -> Result<Self> {
        let api = HidApi::new().map_err(|err| Error::message(format!("HID: {err}")))?;
        let c_path = CString::new(path)
            .map_err(|_| Error::message(format!("HID path {path} is not valid")))?;
        let device = api.open_path(&c_path).map_err(|err| classify(path, &err))?;
        // A timeout on read is what bounds a missing keyboard; blocking mode
        // would sit forever on a device that never answers.
        let _ = device.set_blocking_mode(false);
        Ok(Self {
            device,
            path: path.to_owned(),
        })
    }

    pub fn path(&self) -> &str {
        &self.path
    }
}

impl Link for HidLink {
    fn transact(&mut self, report: Report) -> Result<Report> {
        write_report(&self.device, &self.path, &report)?;
        read_report(&self.device, &self.path)
    }
}

fn write_report(device: &HidDevice, path: &str, report: &Report) -> Result<()> {
    // hidraw wants the payload. A report-id prefix is a different backend's
    // convention; try the 32-byte frame first, then the prefixed one.
    if device.write(report).is_ok() {
        return Ok(());
    }
    let mut prefixed = [0_u8; protocol::REPORT_LEN + 1];
    prefixed[1..].copy_from_slice(report);
    device
        .write(&prefixed)
        .map(|_| ())
        .map_err(|err| classify(path, &err))
}

fn read_report(device: &HidDevice, path: &str) -> Result<Report> {
    let mut buffer = [0_u8; 64];
    let size = device
        .read_timeout(&mut buffer, 500)
        .map_err(|err| classify(path, &err))?;
    if size == 0 {
        return Err(Error::message(format!("{path} did not answer")));
    }
    let start = usize::from(size > protocol::REPORT_LEN && buffer[0] == 0);
    if size - start < protocol::REPORT_LEN {
        return Err(Error::message(format!(
            "{path} answered with {size} bytes, expected {}",
            protocol::REPORT_LEN
        )));
    }
    let mut report = [0_u8; protocol::REPORT_LEN];
    report.copy_from_slice(&buffer[start..start + protocol::REPORT_LEN]);
    Ok(report)
}

fn classify(path: &str, err: &HidError) -> Error {
    let detail = err.to_string();
    let folded = detail.to_ascii_lowercase();
    if folded.contains("permission")
        || folded.contains("access")
        || folded.contains("not permitted")
        || folded.contains("denied")
    {
        Error::Permission {
            path: path.to_owned(),
            detail,
        }
    } else {
        Error::message(format!("{path}: {detail}"))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_via_descriptor_is_recognized_and_a_keyboard_descriptor_is_not() {
        let via = [0x05, 0x01, 0x06, 0x60, 0xFF, 0x09, 0x61, 0x95, 0x20];
        assert!(via.windows(3).any(|window| window == [0x06, 0x60, 0xFF]));
        assert!(via.windows(2).any(|window| window == [0x09, 0x61]));
        let keyboard = [0x05, 0x01, 0x09, 0x06];
        assert!(
            !keyboard
                .windows(3)
                .any(|window| window == [0x06, 0x60, 0xFF])
        );
    }
}
