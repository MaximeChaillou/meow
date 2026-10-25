//! An Objective-C target that runs a Rust closure, so a feature wires its own
//! menu items and buttons: no selector in `Controller`, no variant in
//! `UiEvent`. The closure should only send to the feature's mailbox.
//!
//! AppKit holds targets weakly: whoever builds the control keeps the action
//! alive (a menu item keeps it as its represented object, see `ui::menus`).

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};

pub struct ActionIvars {
    run: Box<dyn Fn()>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MeowAction"]
    #[ivars = ActionIvars]
    pub struct Action;

    impl Action {
        #[unsafe(method(run:))]
        fn run(&self, _sender: Option<&AnyObject>) {
            (self.ivars().run)();
        }
    }
);

impl Action {
    pub fn new(mtm: MainThreadMarker, run: impl Fn() + 'static) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ActionIvars { run: Box::new(run) });
        // SAFETY: NSObject's designated initializer, called once on a fresh allocation.
        unsafe { msg_send![super(this), init] }
    }

    /// The selector to pair with this target on a control.
    pub fn selector() -> Sel {
        sel!(run:)
    }
}
