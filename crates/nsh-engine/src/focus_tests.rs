//! A focus transfer must survive a commit after every received message.

use std::sync::{Arc, Mutex};

use nsh_core::{ApproxTextEngine, MountItem, prelude::*, tailwind::state};

use super::*;

#[derive(Clone)]
struct FocusLog(Arc<Mutex<Vec<&'static str>>>);

#[component]
fn FocusPair() -> impl IntoElement {
    let log = engine_context::<FocusLog>().unwrap();
    let note = |event: &'static str| {
        let log = log.clone();
        move || log.0.lock().unwrap().push(event)
    };
    let common = element_ref();
    common.on_focus_in(note("within.focus"));
    common.on_focus_out(note("within.blur"));
    view()
        .element_ref(&common)
        .test_id("common")
        .flex_1()
        .child(
            view()
                .test_id("first")
                .focusable(true)
                .h(px(40.0))
                .opacity(0.25)
                .hover(|s| s.opacity(0.5))
                .active(|s| s.opacity(0.75))
                .on_focus(note("first.focus"))
                .on_blur(note("first.blur")),
        )
        .child(
            view()
                .test_id("second")
                .focusable(true)
                .h(px(40.0))
                .opacity(0.25)
                .hover(|s| s.opacity(0.5))
                .active(|s| s.opacity(0.75))
                .on_focus(note("second.focus"))
                .on_blur(note("second.blur")),
        )
}

struct Sink(Arc<Mutex<Vec<MountBatch>>>);

impl MountSink for Sink {
    fn submit(&self, batch: MountBatch) {
        self.0.lock().unwrap().push(batch);
    }
}

fn id(batches: &[MountBatch], test_id: &str) -> NodeId {
    batches.iter().flat_map(|batch| &batch.items).find_map(|item| {
        match item {
            MountItem::ViewProps { id, props } if props.test_id.as_deref() == Some(test_id) => Some(*id),
            _ => None,
        }
    }).unwrap()
}

fn opacity(batches: &[MountBatch], id: NodeId) -> f32 {
    batches.iter().flat_map(|batch| &batch.items).filter_map(|item| {
        match item {
            MountItem::ViewStyle { id: found, style } if *found == id => Some(style.opacity),
            _ => None,
        }
    }).next_back().unwrap()
}

/// Deliberately commit after the first received message, without coalescing
/// any later ones. Splitting the transfer into `States` / `Event` messages
/// fails here deterministically, regardless of the sender thread's schedule.
fn commit_one_transfer(engine: &mut LocalEngine<ApproxTextEngine>) {
    let _clock = engine.driver.clock.enter();
    let _context = engine.driver.context.enter();
    let message = engine.rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(matches!(message, Message::FocusTransfer { .. }));
    assert!(engine.driver.handle(message, &engine.rx));
    engine.driver.turn();
    assert!(engine.rx.try_recv().is_err(), "one transfer is exactly one channel message");
}

#[test]
fn a_cross_thread_focus_transfer_has_no_intermediate_commit_and_keeps_other_states() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let batches = Arc::new(Mutex::new(Vec::new()));
    let mut config = EngineConfig::new(Root::of::<FocusPair>());
    config.context = EngineContext::default().with(FocusLog(log.clone()));
    let mut engine = LocalEngine::start(config, ApproxTextEngine, Sink(batches.clone())).unwrap();
    engine.pump();
    let first = id(&batches.lock().unwrap(), "first");
    let second = id(&batches.lock().unwrap(), "second");
    let other = state::HOVERED | state::PRESSED;

    let control = engine.control();
    std::thread::spawn(move || control.transfer_focus(None, Some((first, other)))).join().unwrap();
    commit_one_transfer(&mut engine);
    assert_eq!(*log.lock().unwrap(), ["first.focus", "within.focus"]);
    log.lock().unwrap().clear();

    let control = engine.control();
    std::thread::spawn(move || control.transfer_focus(
        Some((first, other | state::FOCUSED)), Some((second, other)),
    )).join().unwrap();
    commit_one_transfer(&mut engine);
    assert_eq!(*log.lock().unwrap(), ["first.blur", "second.focus"]);
    assert_eq!(opacity(&batches.lock().unwrap(), first), 0.75);
    assert_eq!(opacity(&batches.lock().unwrap(), second), 0.75);
    log.lock().unwrap().clear();

    let control = engine.control();
    std::thread::spawn(move || control.transfer_focus(Some((second, other | state::FOCUSED)), None))
        .join().unwrap();
    commit_one_transfer(&mut engine);
    assert_eq!(*log.lock().unwrap(), ["second.blur", "within.blur"]);
    assert_eq!(opacity(&batches.lock().unwrap(), second), 0.75);
    engine.control().transfer_focus(None, None);
    assert!(engine.rx.try_recv().is_err());
}
