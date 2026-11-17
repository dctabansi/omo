use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const BASE64_TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encodes `input` into standard RFC 4648 Base64.
pub fn base64_encode(input: &[u8]) -> String {
    if input.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(BASE64_TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(BASE64_TABLE[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(BASE64_TABLE[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(BASE64_TABLE[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Formats an ANSI OSC 52 clipboard escape sequence (`\x1b]52;c;<base64>\x07`).
pub fn encode_osc52(text: &str) -> String {
    let encoded = base64_encode(text.as_bytes());
    format!("\x1b]52;c;{encoded}\x07")
}

fn is_wsl() -> bool {
    std::env::var_os("WSL_DISTRO_NAME").is_some()
        || Path::new("/proc/sys/fs/binfmt_misc/WSLInterop").exists()
}

fn try_cli_copy(text: &str) -> bool {
    let candidates: &[(&str, &[&str])] = if is_wsl() {
        &[
            ("clip.exe", &[]),
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ]
    } else {
        &[
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
            ("clip.exe", &[]),
        ]
    };

    for &(program, args) in candidates {
        let Ok(mut child) = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        if let Ok(status) = child.wait()
            && status.success()
        {
            return true;
        }
    }
    false
}

fn try_cli_paste() -> Option<String> {
    let candidates: &[(&str, &[&str])] = if is_wsl() {
        &[
            (
                "powershell.exe",
                &["-NoProfile", "-Command", "Get-Clipboard"],
            ),
            ("wl-paste", &["--no-newline"]),
            ("xclip", &["-selection", "clipboard", "-o"]),
            ("xsel", &["--clipboard", "--output"]),
        ]
    } else {
        &[
            ("wl-paste", &["--no-newline"]),
            ("xclip", &["-selection", "clipboard", "-o"]),
            ("xsel", &["--clipboard", "--output"]),
        ]
    };

    for &(program, args) in candidates {
        let Ok(output) = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
        else {
            continue;
        };
        if output.status.success()
            && let Ok(raw) = String::from_utf8(output.stdout)
        {
            let normalized = raw.replace("\r\n", "\n");
            let cleaned = if program == "powershell.exe" {
                normalized
                    .strip_suffix('\n')
                    .unwrap_or(&normalized)
                    .to_string()
            } else {
                normalized
            };
            return Some(cleaned);
        }
    }
    None
}

/// Multi-tier clipboard manager supporting `arboard`, WSL2/Linux CLI tools,
/// ANSI OSC 52 escape sequences, and an in-memory fallback register.
pub struct ClipboardManager {
    system: Option<arboard::Clipboard>,
    internal: String,
    pending_osc52: Option<String>,
}

impl Default for ClipboardManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ClipboardManager {
    /// Creates a new `ClipboardManager`.
    pub fn new() -> Self {
        let system = if cfg!(test) {
            None
        } else {
            arboard::Clipboard::new().ok()
        };
        Self {
            system,
            internal: String::new(),
            pending_osc52: None,
        }
    }

    /// Copies `text` to the internal register, queues an OSC 52 escape sequence,
    /// and attempts to write to `arboard` or native WSL2/Linux clipboard CLI tools.
    pub fn set_text(&mut self, text: String) {
        self.pending_osc52 = Some(encode_osc52(&text));
        self.internal = text.clone();

        if cfg!(test) {
            return;
        }

        let mut copied_to_system = false;
        if let Some(sys) = &mut self.system
            && sys.set_text(text.clone()).is_ok()
        {
            copied_to_system = true;
        }
        if !copied_to_system {
            let _ = try_cli_copy(&text);
        }
    }

    /// Retrieves clipboard text from `arboard`, native WSL2/Linux CLI tools,
    /// or the internal fallback register.
    pub fn get_text(&mut self) -> String {
        if !cfg!(test) {
            if let Some(sys) = &mut self.system
                && let Ok(text) = sys.get_text()
                && !text.is_empty()
            {
                let normalized = text.replace("\r\n", "\n");
                self.internal = normalized.clone();
                return normalized;
            }
            if let Some(text) = try_cli_paste()
                && !text.is_empty()
            {
                self.internal = text.clone();
                return text;
            }
        }
        self.internal.clone()
    }

    /// Takes any pending ANSI OSC 52 sequence to be queued onto `stdout` during the next frame.
    pub fn take_osc52_sequence(&mut self) -> Option<String> {
        self.pending_osc52.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_osc52_produces_valid_base64_escape_sequence() {
        assert_eq!(encode_osc52("hello"), "\x1b]52;c;aGVsbG8=\x07");
        assert_eq!(encode_osc52("omo 🦀"), "\x1b]52;c;b21vIPCfpoA=\x07");
    }

    #[test]
    fn clipboard_manager_roundtrips_text_and_queues_osc52() {
        let mut cb = ClipboardManager::new();
        assert_eq!(cb.get_text(), "");
        assert_eq!(cb.take_osc52_sequence(), None);

        cb.set_text("line 1\nline 2".to_string());
        assert_eq!(cb.get_text(), "line 1\nline 2");
        assert_eq!(
            cb.take_osc52_sequence(),
            Some("\x1b]52;c;bGluZSAxCmxpbmUgMg==\x07".to_string())
        );
        assert_eq!(cb.take_osc52_sequence(), None);
    }
}
