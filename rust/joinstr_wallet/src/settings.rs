use std::{
    ffi::{c_char, c_int, CStr, CString},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    ptr,
    str::FromStr,
};

use libc::malloc;

use joinstr::{bip39::Mnemonic, serde_json};
use serde::{Deserialize, Serialize};
use url::Url;

type MutStrPtr = *mut *mut c_char;
type ConstStr = *const c_char;

fn datadir() -> PathBuf {
    #[cfg(target_os = "linux")]
    let mut dir = {
        let mut dir = dirs::home_dir().unwrap();
        dir.push(".joinstr");
        dir
    };

    #[cfg(not(target_os = "linux"))]
    let mut dir = {
        let mut dir = dirs::config_dir().unwrap();
        dir.push("Joinstr");
        dir
    };

    maybe_create_dir(&dir);

    dir.push("joinstr.conf");

    dir
}

fn maybe_create_dir(dir: &PathBuf) {
    if !dir.exists() {
        #[cfg(unix)]
        {
            use std::fs::DirBuilder;
            use std::os::unix::fs::DirBuilderExt;

            let mut builder = DirBuilder::new();
            builder.mode(0o700).recursive(true).create(dir).unwrap();
        }

        #[cfg(not(unix))]
        std::fs::create_dir_all(dir).unwrap();
    }
}

#[no_mangle]
#[allow(clippy::missing_safety_doc)]
pub unsafe extern "C" fn is_mnemonic_valid(mnemonic: ConstStr) -> c_int {
    let cstr = unsafe { CStr::from_ptr(mnemonic) };
    let mnemonic = match cstr.to_str() {
        Ok(r) => r,
        Err(_) => return -1,
    };
    match Mnemonic::from_str(mnemonic).is_ok() {
        true => 0,
        false => -2,
    }
}

#[no_mangle]
#[allow(clippy::missing_safety_doc)]
pub unsafe extern "C" fn is_electrum_valid(addr: ConstStr) -> c_int {
    let cstr = unsafe { CStr::from_ptr(addr) };
    let electrum = match cstr.to_str() {
        Ok(r) => r,
        Err(_) => return -1,
    };
    let separators = electrum.chars().filter(|c| *c == ':').count();
    if separators != 1 {
        return -2;
    }
    let (url, port) = electrum.split_once(':').expect("checked");
    let port = u16::from_str(port).is_ok();
    let url = Url::parse(url).is_ok();
    if !url {
        -3
    } else if !port {
        -4
    } else {
        0
    }
}

#[no_mangle]
#[allow(clippy::missing_safety_doc)]
pub unsafe extern "C" fn is_relay_valid(addr: ConstStr) -> c_int {
    let cstr = unsafe { CStr::from_ptr(addr) };
    let url = match cstr.to_str() {
        Ok(r) => r,
        Err(_) => return -1,
    };
    match Url::parse(url).is_ok() {
        true => 0,
        false => -2,
    }
}

#[no_mangle]
#[allow(clippy::missing_safety_doc)]
pub unsafe extern "C" fn save_settings(
    mnemonics: ConstStr,
    electrum: ConstStr,
    relay: ConstStr,
) -> c_int {
    if is_mnemonic_valid(mnemonics) != 0 {
        return -1;
    }
    if is_electrum_valid(electrum) != 0 {
        return -2;
    }
    if is_relay_valid(relay) != 0 {
        return -3;
    }
    let mnemonics = unsafe { CStr::from_ptr(mnemonics) }.to_str();
    let electrum = unsafe { CStr::from_ptr(electrum) }.to_str();
    let relay = unsafe { CStr::from_ptr(relay) }.to_str();

    if let (Ok(mnemonics), Ok(electrum), Ok(relay)) = (mnemonics, electrum, relay) {
        if Settings::new(mnemonics, electrum, relay).to_file(&datadir()) != 0 {
            -4
        } else {
            0
        }
    } else {
        -5
    }
}

unsafe fn write_string(src: &str, dst: *mut *mut c_char) -> c_int {
    let c_str = match CString::new(src) {
        Ok(r) => r,
        Err(_) => return -1,
    };
    let len = c_str.as_bytes_with_nul().len();
    let mem = malloc(len) as *mut c_char;
    if mem.is_null() {
        return -2;
    }
    ptr::copy_nonoverlapping(c_str.as_ptr(), mem, len);
    *dst = mem;
    0
}

#[no_mangle]
#[allow(clippy::missing_safety_doc)]
pub unsafe extern "C" fn load_settings(
    mnemonics: MutStrPtr,
    electrum: MutStrPtr,
    relay: MutStrPtr,
) -> c_int {
    if mnemonics.is_null() || electrum.is_null() || relay.is_null() {
        return -1;
    }
    let settings = match Settings::from_file(&datadir()) {
        Some(s) => s,
        None => return -2,
    };

    if write_string(&settings.mnemonics, mnemonics) != 0 {
        return -3;
    }
    if write_string(&settings.electrum, electrum) != 0 {
        return -4;
    }
    if write_string(&settings.relay, relay) != 0 {
        return -5;
    }

    0
}

#[derive(Debug, Serialize, Deserialize)]
struct Settings {
    pub mnemonics: String,
    pub electrum: String,
    pub relay: String,
}

impl Settings {
    pub fn new(mnemonics: &str, electrum: &str, relay: &str) -> Self {
        Settings {
            mnemonics: mnemonics.into(),
            electrum: electrum.into(),
            relay: relay.into(),
        }
    }

    pub fn to_file(&self, path: &Path) -> c_int {
        let path: &str = &path.to_string_lossy();
        let file = match File::create(path) {
            Ok(f) => f,
            Err(_) => return -1,
        };
        if serde_json::to_writer_pretty(file, self).is_ok() {
            0
        } else {
            -2
        }
    }

    pub fn from_file(path: &Path) -> Option<Self> {
        if !path.exists() || !path.is_file() {
            return None;
        }

        let mut file = File::open(path).ok()?;
        let mut settings_str = String::new();
        let _conf_size = file.read_to_string(&mut settings_str).ok()?;
        let conf: Self = serde_json::from_str(&settings_str).ok()?;
        Some(conf)
    }
}
