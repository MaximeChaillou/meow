//! Draws the Activity page with sample news to a PNG, to see the source
//! icons: `cargo run --example activity_preview -- out.png`

// The app's UI module, borrowed whole: most of it is unused here.
#![allow(dead_code, unused_imports)]

#[path = "../src/ui/mod.rs"]
mod ui;

use meow::activity::{ActivityLog, Kind, Link, Note};
use meow::source::Source;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSBitmapImageFileType, NSView};
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};

use ui::activity_page::ActivityPage;
use ui::controller::Controller;
use ui::{theme, widgets};

/// The size of the page in the chat window.
const SIZE: NSSize = NSSize::new(452.0, 600.0);
const NOW: u64 = 1_792_500_000;

fn note(kind: Kind, title: &str, detail: &str, sources: &[Source], key: Option<&str>) -> Note {
    Note {
        kind,
        title: title.into(),
        detail: Some(detail.into()),
        link: Some(Link::Tasks),
        sources: sources.to_vec(),
        key: key.map(String::from),
    }
}

/// Oldest first; the last three are unread.
fn sample_log() -> ActivityLog {
    use Source::*;
    let mut log = ActivityLog::default();
    let push = |log: &mut ActivityLog, minutes_ago: u64, note: Note| {
        log.push(note, NOW - minutes_ago * 60);
    };
    push(
        &mut log,
        300,
        note(
            Kind::Scheduled,
            "Unread emails summary",
            "3 emails need an answer",
            &[],
            None,
        ),
    );
    push(
        &mut log,
        240,
        note(
            Kind::Calendar,
            "Moved: Sprint review",
            "14:00 → 15:30",
            &[],
            None,
        ),
    );
    push(
        &mut log,
        180,
        note(
            Kind::Tasks,
            "5 tasks resolved at the source",
            "Review PR #812, ACC-52, Answer Cyril and 2 more",
            &[GitHub, Linear, Slack, Mail, Notion],
            None,
        ),
    );
    push(
        &mut log,
        120,
        note(
            Kind::Tasks,
            "3 new tasks waiting on you",
            "Review PR #820, Fix ACC-61, Reply in #print-ops",
            &[GitHub, Linear, Slack],
            None,
        ),
    );
    push(
        &mut log,
        60,
        note(
            Kind::Tasks,
            "2 new tasks waiting on you",
            "Review PR #823, Triage API-7",
            &[GitHub, Linear, GitHub],
            None,
        ),
    );
    log.mark_all_read();
    push(
        &mut log,
        30,
        note(
            Kind::Slack,
            "Cyril in #print-ops",
            "@axel the Kodak export is back",
            &[Slack],
            Some("slack:C1:cyril"),
        ),
    );
    // Three messages from Bo: only the last one stays.
    for (ago, text) in [
        (20, "hey"),
        (12, "are you around?"),
        (5, "never mind, found it 🙂"),
    ] {
        push(
            &mut log,
            ago,
            note(Kind::Slack, "Bo", text, &[Slack], Some("slack:D1:bo")),
        );
    }
    push(
        &mut log,
        2,
        note(
            Kind::Tasks,
            "1 new task waiting on you",
            "Answer the invoice email",
            &[Mail],
            None,
        ),
    );
    log
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "activity.png".into());
    let mtm = MainThreadMarker::new().expect("runs on the main thread");
    let _app = NSApplication::sharedApplication(mtm);
    if let Err(error) = theme::register_fonts() {
        eprintln!("{error}");
    }
    let controller = Controller::new(mtm);
    let bounds = NSRect::new(NSPoint::new(0.0, 0.0), SIZE);
    let page = ActivityPage::new(mtm, &controller, bounds);
    page.render(&sample_log(), NOW);

    let backdrop = NSView::initWithFrame(NSView::alloc(mtm), bounds);
    widgets::paint(&backdrop, Some(&theme::paper()), None);
    backdrop.addSubview(&page.view);
    let rep = backdrop
        .bitmapImageRepForCachingDisplayInRect(bounds)
        .expect("bitmap for the page");
    backdrop.cacheDisplayInRect_toBitmapImageRep(bounds, &rep);
    // SAFETY: PNG takes no required properties; an empty dictionary is valid.
    let png = unsafe {
        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }
    .expect("encode png");
    assert!(
        png.writeToFile_atomically(&NSString::from_str(&path), true),
        "write {path}"
    );
    println!("wrote {path}");
}
