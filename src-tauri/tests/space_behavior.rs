#[cfg(target_os = "macos")]
#[path = "../src/space_behavior.rs"]
mod space_behavior;

#[cfg(target_os = "macos")]
fn main() {
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior as Behavior};

    let mtm = MainThreadMarker::new().expect("AppKit test must run on the main thread");
    // SAFETY: allocation and initialization occur on the process main thread.
    // Never order the window front or activate the application.
    let window = unsafe { NSWindow::init(NSWindow::alloc(mtm)) };
    window.setCollectionBehavior(Behavior::IgnoresCycle);
    assert!(!window
        .collectionBehavior()
        .contains(Behavior::CanJoinAllSpaces));
    assert!(!window
        .collectionBehavior()
        .contains(Behavior::FullScreenAuxiliary));

    space_behavior::allow_fullscreen_spaces(&window);
    let behavior = window.collectionBehavior();
    assert!(behavior.contains(Behavior::CanJoinAllSpaces));
    assert!(behavior.contains(Behavior::FullScreenAuxiliary));
    assert!(behavior.contains(Behavior::IgnoresCycle));
    assert!(!window.isVisible());

    space_behavior::allow_fullscreen_spaces(&window);
    assert_eq!(window.collectionBehavior(), behavior);
    assert!(!window.isVisible());
    println!(
        "space_behavior: both flags set, existing flags preserved, idempotent, window invisible"
    );
}

#[cfg(not(target_os = "macos"))]
fn main() {}
