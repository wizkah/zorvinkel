//! Vertically centres the native macOS window buttons in the tab bar.
//!
//! winit offers no position for them, so this adjusts the AppKit views directly. AppKit
//! restores its own layout on resize, so the placement is applied again on every frame.
#![allow(unsafe_code)]

use objc2_app_kit::{NSView, NSWindowButton};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// Leading space between the window edge and the close button.
const LEADING: f64 = 16.0;
/// Space between neighbouring buttons.
const SPACING: f64 = 8.0;

pub fn center_in_bar(window: &impl HasWindowHandle, bar_height: f64) {
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };
    // SAFETY: the handle comes from the live window and is only used on the main thread
    // during this call.
    let view: &NSView = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let Some(window) = view.window() else {
        return;
    };
    let kinds = [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ];
    let buttons: Vec<_> = kinds
        .into_iter()
        .filter_map(|kind| window.standardWindowButton(kind))
        .collect();
    let Some(first) = buttons.first() else {
        return;
    };
    // SAFETY: the views belong to the window that was just obtained.
    let (titlebar, container) = unsafe {
        let Some(titlebar) = first.superview() else {
            return;
        };
        let Some(container) = titlebar.superview() else {
            return;
        };
        (titlebar, container)
    };
    let window_frame = window.frame();
    let width = window_frame.size.width;
    let container_frame = NSRect::new(
        NSPoint::new(0.0, window_frame.size.height - bar_height),
        NSSize::new(width, bar_height),
    );
    if container.frame() != container_frame {
        container.setFrame(container_frame);
    }
    let titlebar_frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, bar_height));
    if titlebar.frame() != titlebar_frame {
        titlebar.setFrame(titlebar_frame);
    }
    let mut x = LEADING;
    for button in &buttons {
        let size = button.frame().size;
        let origin = NSPoint::new(x, (bar_height - size.height) / 2.0);
        if button.frame().origin != origin {
            button.setFrameOrigin(origin);
        }
        x += size.width + SPACING;
    }
}
