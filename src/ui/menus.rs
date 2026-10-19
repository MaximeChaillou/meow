//! Main menu (so ⌘C/⌘V/⌘A work in the chat) and the cat's right-click menu.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{
    NSApplication, NSControlStateValueOff, NSControlStateValueOn, NSEventModifierFlags, NSMenu,
    NSMenuItem,
};
use objc2_foundation::NSString;

use meow::actions::QuickAction;
use meow::sprites::Coat;

use super::controller::Controller;

fn item(
    mtm: MainThreadMarker,
    title: &str,
    action: Sel,
    key: &str,
    target: Option<&AnyObject>,
) -> Retained<NSMenuItem> {
    // SAFETY: plain initializer; `action` is a selector understood by the target or responder chain.
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            Some(action),
            &NSString::from_str(key),
        )
    };
    // SAFETY: targets are app-lifetime objects (or nil for the responder chain).
    unsafe { item.setTarget(target) };
    item
}

fn menu(mtm: MainThreadMarker, title: &str, items: &[Retained<NSMenuItem>]) -> Retained<NSMenu> {
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title));
    for entry in items {
        menu.addItem(entry);
    }
    menu
}

fn submenu_item(mtm: MainThreadMarker, submenu: &NSMenu) -> Retained<NSMenuItem> {
    let entry = NSMenuItem::new(mtm);
    entry.setSubmenu(Some(submenu));
    entry
}

/// Esc as a menu key equivalent works whatever has focus in the chat (input or transcript).
fn escape_item(mtm: MainThreadMarker, target: &AnyObject) -> Retained<NSMenuItem> {
    let entry = item(mtm, "Cancel / Close", sel!(escape:), "\u{1b}", Some(target));
    entry.setKeyEquivalentModifierMask(NSEventModifierFlags::empty());
    entry
}

pub fn install_main_menu(mtm: MainThreadMarker, controller: &Controller) {
    let target: &AnyObject = controller;
    let app_menu = menu(
        mtm,
        "Meow",
        &[item(mtm, "Quit Meow", sel!(quit:), "q", Some(target))],
    );
    let edit_menu = menu(
        mtm,
        "Edit",
        &[
            item(mtm, "Undo", sel!(undo:), "z", None),
            item(mtm, "Cut", sel!(cut:), "x", None),
            item(mtm, "Copy", sel!(copy:), "c", None),
            item(mtm, "Paste", sel!(paste:), "v", None),
            item(mtm, "Select All", sel!(selectAll:), "a", None),
            item(mtm, "Close", sel!(toggleChat:), "w", Some(target)),
            escape_item(mtm, target),
            item(mtm, "Tasks", sel!(showTasks:), "t", Some(target)),
            item(mtm, "Slack", sel!(showSlack:), "l", Some(target)),
            item(
                mtm,
                "New Conversation",
                sel!(newConversation:),
                "n",
                Some(target),
            ),
        ],
    );
    let bar = NSMenu::new(mtm);
    bar.addItem(&submenu_item(mtm, &app_menu));
    bar.addItem(&submenu_item(mtm, &edit_menu));
    NSApplication::sharedApplication(mtm).setMainMenu(Some(&bar));
}

/// One item per coat, the current one checked.
fn coat_menu(mtm: MainThreadMarker, target: &AnyObject, coat: Coat) -> Retained<NSMenu> {
    let items: Vec<_> = Coat::ALL
        .iter()
        .enumerate()
        .map(|(index, choice)| {
            let entry = item(mtm, choice.name(), sel!(setCoat:), "", Some(target));
            entry.setTag(index as isize);
            entry
        })
        .collect();
    let menu = menu(mtm, "Theme", &items);
    mark_coat(&menu, coat);
    menu
}

/// Checks `coat` in the theme menu.
pub fn mark_coat(menu: &NSMenu, coat: Coat) {
    for (index, choice) in Coat::ALL.iter().enumerate() {
        let Some(entry) = menu.itemWithTag(index as isize) else {
            continue;
        };
        let state = if *choice == coat {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        };
        entry.setState(state);
    }
}

/// The cat's right-click menu, and its theme submenu to check the coat in later.
pub fn cat_menu(
    mtm: MainThreadMarker,
    controller: &Controller,
    actions: &[QuickAction],
    coat: Coat,
) -> (Retained<NSMenu>, Retained<NSMenu>) {
    let target: &AnyObject = controller;
    let mut items: Vec<_> = actions
        .iter()
        .enumerate()
        .map(|(index, action)| {
            let entry = item(mtm, &action.title, sel!(quickAction:), "", Some(target));
            entry.setTag(index as isize);
            entry
        })
        .collect();
    if !items.is_empty() {
        items.push(NSMenuItem::separatorItem(mtm));
    }
    items.extend([
        item(mtm, "Open Chat", sel!(toggleChat:), "", Some(target)),
        item(mtm, "Tasks", sel!(showTasks:), "", Some(target)),
        item(mtm, "Slack", sel!(showSlack:), "", Some(target)),
        item(
            mtm,
            "New Conversation",
            sel!(newConversation:),
            "",
            Some(target),
        ),
        NSMenuItem::separatorItem(mtm),
    ]);
    items.extend([
        item(mtm, "Feed", sel!(feed:), "", Some(target)),
        item(mtm, "Let a Mouse In", sel!(releaseMouse:), "", Some(target)),
    ]);
    let coats = coat_menu(mtm, target, coat);
    let theme = submenu_item(mtm, &coats);
    theme.setTitle(&NSString::from_str("Theme"));
    items.extend([
        theme,
        NSMenuItem::separatorItem(mtm),
        item(mtm, "Quit Meow", sel!(quit:), "", Some(target)),
    ]);
    (menu(mtm, "Meow", &items), coats)
}
