use super::*;

fn payload(text: &str) -> Incoming {
    Incoming::Data(Ok(Some(Payload {
        text: text.into(),
        region: "cn".into(),
        uid: None,
        apply: false,
    })))
}

#[test]
fn cancellation_discards_completed_and_late_payloads() {
    let mut state = PlayerDataImport::default();
    let old_ticket = state.generation;
    deliver(&state.inbox, old_ticket, payload("old"));
    state.cancel();
    assert!(state.inbox.lock().unwrap().message.is_none());
    deliver(&state.inbox, old_ticket, payload("late"));
    state.receive();
    assert!(state.pending.is_none());
    assert!(!state.busy);
    assert!(state.preview.is_none());
    deliver(&state.inbox, state.generation, payload("current"));
    state.receive();
    assert_eq!(state.pending.as_ref().unwrap().text, "current");
}

#[test]
fn failed_reads_do_not_create_an_import_preview() {
    let mut state = PlayerDataImport::default();
    state.busy = true;
    deliver(
        &state.inbox,
        state.generation,
        Incoming::Data(Err("body exceeds limit".into())),
    );
    state.receive();
    assert!(!state.busy);
    assert!(state.pending.is_none());
    assert!(!state.has_preview());
    assert_eq!(state.status(), "body exceeds limit");
}
