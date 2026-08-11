use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use objc2::rc::Retained;
use objc2_app_kit::NSPasteboard;
use objc2_foundation::{NSData, NSString};
use std::thread;
use std::time::Duration;

const KEYCODE_C: u16 = 8;
const PLAIN_TEXT_FLAVOR: &str = "public.utf8-plain-text";
const HTML_FLAVOR: &str = "public.html";

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

pub fn accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

#[derive(serde::Serialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureSource {
    Selection,
    Clipboard,
}

#[derive(serde::Serialize, Clone)]
pub struct CaptureFlavor {
    pub flavor: String,
    pub content: String,
}

#[derive(serde::Serialize, Clone)]
pub struct CaptureResult {
    pub source: CaptureSource,
    pub plain_text: Option<String>,
    pub html: Option<String>,
    pub flavors: Vec<CaptureFlavor>,
}

fn pasteboard() -> Retained<NSPasteboard> {
    NSPasteboard::generalPasteboard()
}

struct SavedPasteboardItem {
    flavor_data: Vec<(String, Vec<u8>)>,
}

fn snapshot_pasteboard() -> Vec<SavedPasteboardItem> {
    let pb = pasteboard();
    let mut saved = Vec::new();

    if let Some(items) = pb.pasteboardItems() {
        for item in items.iter() {
            let mut flavor_data = Vec::new();
            for t in item.types().iter() {
                if let Some(data) = item.dataForType(&t) {
                    flavor_data.push((t.to_string(), data.to_vec()));
                }
            }
            saved.push(SavedPasteboardItem { flavor_data });
        }
    }

    saved
}

fn restore_pasteboard(saved: Vec<SavedPasteboardItem>) {
    let pb = pasteboard();
    pb.clearContents();
    for item in saved {
        for (flavor, bytes) in item.flavor_data {
            let ns_flavor = NSString::from_str(&flavor);
            let ns_data = NSData::with_bytes(&bytes);
            pb.setData_forType(Some(&ns_data), &ns_flavor);
        }
    }
}

fn read_all_flavors() -> Vec<CaptureFlavor> {
    let pb = pasteboard();
    let mut out = Vec::new();

    if let Some(items) = pb.pasteboardItems() {
        for item in items.iter() {
            for t in item.types().iter() {
                if let Some(data) = item.dataForType(&t) {
                    let bytes = data.to_vec();
                    out.push(CaptureFlavor {
                        flavor: t.to_string(),
                        content: String::from_utf8_lossy(&bytes).to_string(),
                    });
                }
            }
        }
    }

    out
}

fn simulate_cmd_c() -> Result<(), String> {
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|_| "failed to create CGEventSource".to_string())?;

    let key_down = CGEvent::new_keyboard_event(source.clone(), KEYCODE_C, true)
        .map_err(|_| "failed to create keydown event".to_string())?;
    key_down.set_flags(CGEventFlags::CGEventFlagCommand);

    let key_up = CGEvent::new_keyboard_event(source, KEYCODE_C, false)
        .map_err(|_| "failed to create keyup event".to_string())?;
    key_up.set_flags(CGEventFlags::CGEventFlagCommand);

    key_down.post(CGEventTapLocation::HID);
    key_up.post(CGEventTapLocation::HID);

    Ok(())
}

fn build_result(flavors: Vec<CaptureFlavor>, source: CaptureSource) -> CaptureResult {
    let plain_text = flavors
        .iter()
        .find(|f| f.flavor == PLAIN_TEXT_FLAVOR)
        .map(|f| f.content.clone());
    let html = flavors
        .iter()
        .find(|f| f.flavor == HTML_FLAVOR)
        .map(|f| f.content.clone());

    CaptureResult {
        source,
        plain_text,
        html,
        flavors,
    }
}

fn capture_via_emulated_copy() -> Result<CaptureResult, String> {
    let saved = snapshot_pasteboard();
    let pb = pasteboard();
    let change_count_before = pb.changeCount();

    simulate_cmd_c()?;
    thread::sleep(Duration::from_millis(150));

    let change_count_after = pb.changeCount();
    let new_selection_copied = change_count_after != change_count_before;

    let flavors = read_all_flavors();
    restore_pasteboard(saved);

    let source = if new_selection_copied {
        CaptureSource::Selection
    } else {
        CaptureSource::Clipboard
    };

    Ok(build_result(flavors, source))
}

fn capture_clipboard_only() -> CaptureResult {
    build_result(read_all_flavors(), CaptureSource::Clipboard)
}

pub fn capture_selection() -> Result<CaptureResult, String> {
    let result = if accessibility_trusted() {
        capture_via_emulated_copy()?
    } else {
        capture_clipboard_only()
    };

    if result.plain_text.as_deref().unwrap_or("").is_empty() {
        return Err("clipboard_empty".to_string());
    }

    Ok(result)
}
