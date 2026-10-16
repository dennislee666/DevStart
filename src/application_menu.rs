//! Native menu actions only enqueue presentation events. Rendering and application
//! state stay on the existing egui owner, so menu clicks never control processes.
use std::sync::atomic::{AtomicBool, Ordering};
static ABOUT_REQUESTED: AtomicBool = AtomicBool::new(false);
pub fn take_about_request() -> bool {
    ABOUT_REQUESTED.swap(false, Ordering::AcqRel)
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use crate::i18n::Locale;
    use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use objc2::{rc::Retained, runtime::AnyObject};
    use objc2_app_kit::{NSApplication, NSMenu, NSMenuItem};
    use objc2_foundation::{NSObject, NSObjectProtocol, NSString, ns_string};

    define_class!(
        // SAFETY: NSObject has no additional subclassing invariants. The target
        // has no native resources; its lifetime is retained by NativeMenu below.
        #[unsafe(super = NSObject)]
        #[thread_kind = MainThreadOnly]
        #[ivars = ()]
        struct AboutTarget;
        unsafe impl NSObjectProtocol for AboutTarget {}
        impl AboutTarget {
            // SAFETY: Cocoa target/action callbacks use one nullable object sender.
            #[unsafe(method(openDevStartAbout:))]
            fn open_about(&self, _sender: Option<&AnyObject>) {
                ABOUT_REQUESTED.store(true, Ordering::Release);
            }
        }
    );
    impl AboutTarget {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let target = Self::alloc(mtm).set_ivars(());
            // SAFETY: NSObject init has the standard initializer signature.
            unsafe { msg_send![super(target), init] }
        }
    }
    pub struct NativeMenu {
        // NSMenuItem target references are not retained by AppKit.
        _target: Retained<AboutTarget>,
        bar: Retained<NSMenu>,
        menu: Retained<NSMenu>,
        top: Retained<NSMenuItem>,
        action: Retained<NSMenuItem>,
        app_action: Option<Retained<NSMenuItem>>,
        locale: Locale,
    }
    impl NativeMenu {
        pub fn install(locale: Locale) -> Result<Self, String> {
            let mtm = MainThreadMarker::new().ok_or("应用菜单必须在主线程创建")?;
            let app = NSApplication::sharedApplication(mtm);
            let bar = app.mainMenu().ok_or("系统应用菜单尚未创建")?;
            let target = AboutTarget::new(mtm);
            let menu = NSMenu::new(mtm);
            menu.setAutoenablesItems(false);
            let action = unsafe {
                // SAFETY: Selector and target use the matching signature above;
                // NativeMenu retains the target for the full application lifetime.
                let action = NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    ns_string!(""),
                    Some(sel!(openDevStartAbout:)),
                    ns_string!(""),
                );
                action.setTarget(Some(&target));
                action
            };
            action.setEnabled(true);
            menu.addItem(&action);
            let top = NSMenuItem::new(mtm);
            top.setSubmenu(Some(&menu));
            bar.insertItem_atIndex(&top, 1.min(bar.numberOfItems()));
            let app_action =
                bar.itemAtIndex(0)
                    .and_then(|item| item.submenu())
                    .and_then(|app_menu| {
                        // Replace the standard About panel entry with the same custom dialog.
                        let index = unsafe {
                            app_menu.indexOfItemWithTarget_andAction(
                                None,
                                Some(sel!(orderFrontStandardAboutPanel:)),
                            )
                        };
                        if index >= 0 {
                            app_menu.itemAtIndex(index)
                        } else {
                            None
                        }
                    });
            if let Some(item) = &app_action {
                // SAFETY: Target ownership and selector contract are identical to action.
                unsafe {
                    item.setTarget(Some(&target));
                    item.setAction(Some(sel!(openDevStartAbout:)));
                }
            }
            let mut result = Self {
                _target: target,
                bar,
                menu,
                top,
                action,
                app_action,
                locale,
            };
            result.set_titles(locale);
            Ok(result)
        }
        fn set_titles(&mut self, locale: Locale) {
            let title = NSString::from_str(locale.text("关于"));
            let action = NSString::from_str(locale.text("关于 DevStart"));
            self.top.setTitle(&title);
            self.menu.setTitle(&title);
            self.action.setTitle(&action);
            if let Some(item) = &self.app_action {
                item.setTitle(&action);
            }
            self.locale = locale;
        }
        pub fn sync_locale(&mut self, locale: Locale) {
            if self.locale != locale {
                self.set_titles(locale);
            }
        }
    }
    impl Drop for NativeMenu {
        fn drop(&mut self) {
            // SAFETY: Disconnect unretained targets before AboutTarget is released.
            // Restore the original responder-chain action in the application menu.
            unsafe {
                self.action.setTarget(None);
                self.action.setAction(None);
                if let Some(item) = &self.app_action {
                    item.setTarget(None);
                    item.setAction(Some(sel!(orderFrontStandardAboutPanel:)));
                }
            }
            if self.bar.indexOfItem(&self.top) >= 0 {
                self.bar.removeItem(&self.top);
            }
        }
    }
}
#[cfg(not(target_os = "macos"))]
mod platform {
    pub struct NativeMenu;
    impl NativeMenu {
        pub fn install(_: crate::i18n::Locale) -> Result<Self, String> {
            Ok(Self)
        }
        pub fn sync_locale(&mut self, _: crate::i18n::Locale) {}
    }
}
pub use platform::NativeMenu;
