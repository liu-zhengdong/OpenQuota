use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};

/// Keep the panel available in desktop and fullscreen spaces, preserving other behavior.
pub fn allow_fullscreen_spaces(window: &NSWindow) {
    window.setCollectionBehavior(
        window.collectionBehavior()
            | NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
}
