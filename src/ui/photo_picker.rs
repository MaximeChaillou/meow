//! The open panel for picking a photo of the user's cat.

use std::path::PathBuf;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSModalResponseOK, NSOpenPanel};
use objc2_foundation::{NSArray, NSString};
use objc2_uniform_type_identifiers::UTType;

/// Any image macOS can open (JPEG, PNG, HEIC…).
const IMAGE_TYPE: &str = "public.image";

/// Asks for one image file; `None` when the user cancels.
pub fn pick(mtm: MainThreadMarker) -> Option<PathBuf> {
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(false);
    panel.setMessage(Some(&NSString::from_str(
        "Pick a photo of your cat: Meow will dress the cat in its colors.",
    )));
    if let Some(image) = UTType::typeWithIdentifier(&NSString::from_str(IMAGE_TYPE)) {
        panel.setAllowedContentTypes(&NSArray::from_retained_slice(&[image]));
    }
    // Meow has no Dock icon: bring it forward so the panel shows on top.
    NSApplication::sharedApplication(mtm).activate();
    if panel.runModal() != NSModalResponseOK {
        return None;
    }
    let path = panel.URL()?.path()?;
    Some(PathBuf::from(path.to_string()))
}
