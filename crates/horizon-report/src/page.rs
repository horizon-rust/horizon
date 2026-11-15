//! The Report Bugs screen: a page at `Place::ReportBugs`, which the engine draws as a form over the
//! world (`game/report_form.rs`). Three steps share it: Write (kind, area, the player's sentence
//! and what they expected), Preview (the whole issue, a few lines at a time, with Send), and the
//! answer (the issue's link, or why it failed). The engine keeps the fields' text while the form
//! is open and tells the page each change; the page keeps the rest here, in memory only.
use crate::body::{self, AREAS, Draft, Kind, Problem, Technical};
use crate::send::{self, Sent};
use horizon_platform::pages::{Act, Page, Place, Row};
use serde_json::Value;
use std::sync::{Mutex, MutexGuard};

const TITLE: &str = "horizon.report.title";
const HAPPENED: &str = "horizon.report.happened";
const EXPECTED: &str = "horizon.report.expected";
const KIND_BUG: &str = "horizon.report.kind_bug";
const KIND_PERFORMANCE: &str = "horizon.report.kind_performance";
const NEXT_AREA: &str = "horizon.report.next_area";
const SHOW_ISSUE: &str = "horizon.report.show_issue";
const PREVIOUS: &str = "horizon.report.previous";
const NEXT: &str = "horizon.report.next";
const SEND: &str = "horizon.report.send";
const EDIT: &str = "horizon.report.edit";
const RETRY: &str = "horizon.report.retry";

const AREA_KEYS: [&str; 10] = [
    "horizon.report.area_blocks",
    "horizon.report.area_mobs",
    "horizon.report.area_rendering",
    "horizon.report.area_audio",
    "horizon.report.area_ui",
    "horizon.report.area_multiplayer",
    "horizon.report.area_saves",
    "horizon.report.area_worldgen",
    "horizon.report.area_chunks",
    "horizon.report.area_commands",
];

/// Lines of the issue on one page of the preview.
const PAGE_LINES: usize = 8;
/// The characters a preview line holds: the form wraps at 300 pixels.
const LINE_CHARS: usize = 44;

#[derive(Clone, Debug, PartialEq)]
enum Step {
    Write,
    /// The issue's page `.0` (from 0).
    Preview(usize),
    Sending,
    Failed(String),
    Sent(Sent),
}

/// What the screen keeps while the game runs.
struct State {
    step: Step,
    draft: Draft,
    /// The request the preview shows, and its lines: what Send posts.
    request: Option<Value>,
    lines: Vec<String>,
}

impl State {
    const fn new() -> Self {
        Self {
            step: Step::Write,
            draft: Draft {
                kind: Kind::Bug,
                area: 0,
                happened: String::new(),
                expected: String::new(),
            },
            request: None,
            lines: Vec::new(),
        }
    }

    fn pages(&self) -> usize {
        self.lines.len().div_ceil(PAGE_LINES).max(1)
    }
}

static STATE: Mutex<State> = Mutex::new(State::new());

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// What a press asks the page to do outside its state.
#[derive(Debug, PartialEq)]
enum Effect {
    None,
    /// Post this request.
    Post(Value),
}

/// The rows of the current step. Their number and order never depend on the text typed, so the
/// engine's row indexes stay right while it tells the page of several edits at once.
fn build_rows(state: &State, reports_on: bool, rows: &mut Vec<Row>) {
    match &state.step {
        Step::Write => {
            let draft = &state.draft;
            let bug = draft.kind == Kind::Bug;
            rows.push(Row::Heading(if bug {
                AREA_KEYS[draft.area % AREA_KEYS.len()]
            } else {
                "horizon.report.performance_report"
            }));
            rows.push(Row::Heading(match body::check(draft, None) {
                _ if !reports_on => "horizon.report.problem_off",
                Err(problem) => problem.key(),
                Ok(()) => "horizon.report.ready",
            }));
            rows.push(Row::field(HAPPENED, draft.happened.clone()).max(*body::HAPPENED_BUG.end()));
            rows.push(Row::field(EXPECTED, draft.expected.clone()).max(body::EXPECTED_MAX));
            rows.push(Row::button(KIND_BUG).half().active(!bug));
            rows.push(Row::button(KIND_PERFORMANCE).half().active(bug));
            rows.push(Row::button(NEXT_AREA).half().active(bug));
            rows.push(
                Row::button(SHOW_ISSUE)
                    .half()
                    .active(reports_on && body::check(draft, None).is_ok()),
            );
        }
        Step::Preview(page) => {
            let pages = state.pages();
            rows.push(Row::Heading("horizon.report.preview"));
            rows.push(Row::text(format!("{} / {pages}", page + 1)).quiet());
            let from = page * PAGE_LINES;
            for line in state.lines.iter().skip(from).take(PAGE_LINES) {
                rows.push(Row::text(line.clone()));
            }
            rows.push(Row::button(PREVIOUS).half().active(*page > 0));
            rows.push(Row::button(NEXT).half().active(page + 1 < pages));
            rows.push(Row::button(SEND).half().active(reports_on));
            rows.push(Row::button(EDIT).half());
        }
        Step::Sending => rows.push(Row::Heading("horizon.report.sending")),
        Step::Failed(why) => {
            rows.push(Row::Heading("horizon.report.failed"));
            rows.push(Row::text(why.clone()));
            rows.push(Row::button(RETRY));
        }
        Step::Sent(Sent::Opened(link)) => {
            rows.push(Row::Heading("horizon.report.sent"));
            rows.push(Row::text(link.clone()));
        }
        Step::Sent(Sent::Queued) => rows.push(Row::Heading("horizon.report.queued")),
    }
}

/// A press on the button labelled `label` (`back`: with Shift held).
fn press(state: &mut State, label: &str, back: bool, technical: &dyn Fn() -> Technical) -> Effect {
    match (label, state.step.clone()) {
        (KIND_BUG, Step::Write) => state.draft.kind = Kind::Bug,
        (KIND_PERFORMANCE, Step::Write) => state.draft.kind = Kind::Performance,
        (NEXT_AREA, Step::Write) if state.draft.kind == Kind::Bug => {
            let n = AREAS.len();
            state.draft.area = if back {
                (state.draft.area + n - 1) % n
            } else {
                (state.draft.area + 1) % n
            };
        }
        (SHOW_ISSUE, Step::Write) => match body::request(&state.draft, &technical()) {
            Ok(request) => {
                state.lines = body::wrap(&body::preview(&request), LINE_CHARS);
                state.request = Some(request);
                state.step = Step::Preview(0);
            }
            Err(Problem::Happened | Problem::Measurement | Problem::Version) => {}
        },
        (PREVIOUS, Step::Preview(page)) => state.step = Step::Preview(page.saturating_sub(1)),
        (NEXT, Step::Preview(page)) => {
            state.step = Step::Preview((page + 1).min(state.pages() - 1));
        }
        (EDIT, Step::Preview(_)) => state.step = Step::Write,
        (SEND, Step::Preview(_)) | (RETRY, Step::Failed(_)) => {
            if let Some(request) = state.request.clone() {
                state.step = Step::Sending;
                return Effect::Post(request);
            }
        }
        _ => {}
    }
    Effect::None
}

/// An edit of the field labelled `label`.
fn edit(state: &mut State, label: &str, text: String) {
    if state.step != Step::Write {
        return;
    }
    match label {
        HAPPENED => state.draft.happened = text,
        EXPECTED => state.draft.expected = text,
        _ => {}
    }
}

/// Posts the request on a thread of its own, so the game keeps drawing; the answer lands in the
/// state, where the next frame's rows show it.
fn post(request: Value) {
    let Some(endpoint) = horizon_telemetry::endpoint() else {
        state().step = Step::Failed("reports are turned off".to_owned());
        return;
    };
    let spawned = std::thread::Builder::new()
        .name("Report".into())
        .spawn(move || {
            let result = send::post(&endpoint, &request);
            let mut state = state();
            state.step = match result {
                Ok(sent) => Step::Sent(sent),
                Err(why) => {
                    log::info!("Horizon: the report was not sent: {why}");
                    Step::Failed(why)
                }
            };
        });
    if let Err(e) = spawned {
        state().step = Step::Failed(e.to_string());
    }
}

/// The page, for `horizon_platform::pages::register`.
pub static PAGE: Screen = Screen;

pub struct Screen;

impl Page for Screen {
    fn id(&self) -> &'static str {
        "horizon-report.screen"
    }
    fn title(&self) -> &'static str {
        TITLE
    }
    fn place(&self) -> Place {
        Place::ReportBugs
    }
    fn rows(&self, rows: &mut Vec<Row>) {
        build_rows(&state(), horizon_telemetry::endpoint().is_some(), rows);
    }
    fn act(&self, row: usize, act: Act) {
        let reports_on = horizon_telemetry::endpoint().is_some();
        let effect = {
            let mut state = state();
            let mut rows = Vec::new();
            build_rows(&state, reports_on, &mut rows);
            match (rows.get(row), act) {
                (Some(Row::Field { label, .. }), Act::Edit(text)) => {
                    edit(&mut state, label, text);
                    Effect::None
                }
                (
                    Some(Row::Button {
                        label,
                        active: true,
                        ..
                    }),
                    act @ (Act::Press | Act::PressBack),
                ) => press(
                    &mut state,
                    label,
                    act == Act::PressBack,
                    &crate::technical_now,
                ),
                _ => Effect::None,
            }
        };
        if let Effect::Post(request) = effect {
            post(request);
        }
    }
    /// A finished or failed report starts over; one still on its way keeps its screen.
    fn opened(&self) {
        let mut state = state();
        if state.step != Step::Sending {
            *state = State::new();
        }
    }
    fn back(&self) -> &'static str {
        match state().step {
            Step::Sent(_) => "gui.done",
            _ => "gui.cancel",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn technical() -> Technical {
        Technical {
            version: "v0.2.31".into(),
            system: vec![("CPU".into(), "Some CPU".into())],
            settings: vec![],
            measurement: None,
        }
    }

    fn typed(state: &mut State, happened: &str) {
        edit(state, HAPPENED, happened.to_owned());
    }

    fn labels(state: &State) -> Vec<&'static str> {
        let mut rows = Vec::new();
        build_rows(state, true, &mut rows);
        rows.iter()
            .filter_map(|row| match row {
                Row::Button {
                    label,
                    active: true,
                    ..
                } => Some(*label),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_write_step_keeps_its_rows_whatever_is_typed() {
        let mut state = State::new();
        let count = |state: &State| {
            let mut rows = Vec::new();
            build_rows(state, true, &mut rows);
            rows.len()
        };
        let empty = count(&state);
        typed(&mut state, "The sky stays black after I sleep");
        assert_eq!(count(&state), empty);
        state.draft.kind = Kind::Performance;
        assert_eq!(count(&state), empty);
    }

    #[test]
    fn show_the_issue_waits_for_a_sentence() {
        let mut state = State::new();
        assert!(!labels(&state).contains(&SHOW_ISSUE));
        typed(&mut state, "The sky stays black after I sleep");
        assert!(labels(&state).contains(&SHOW_ISSUE));
        // A performance report needs a measurement, which is not taken yet.
        press(&mut state, KIND_PERFORMANCE, false, &technical);
        assert!(!labels(&state).contains(&SHOW_ISSUE));
    }

    #[test]
    fn the_area_steps_forward_and_back_round_the_list() {
        let mut state = State::new();
        press(&mut state, NEXT_AREA, false, &technical);
        assert_eq!(state.draft.area, 1);
        press(&mut state, NEXT_AREA, true, &technical);
        press(&mut state, NEXT_AREA, true, &technical);
        assert_eq!(state.draft.area, AREAS.len() - 1);
        press(&mut state, KIND_PERFORMANCE, false, &technical);
        press(&mut state, NEXT_AREA, false, &technical);
        assert_eq!(state.draft.area, AREAS.len() - 1);
    }

    #[test]
    fn the_preview_is_paged_and_send_posts_what_it_showed() {
        let mut state = State::new();
        typed(&mut state, "The sky stays black after I sleep");
        edit(&mut state, EXPECTED, "Daylight".to_owned());
        press(&mut state, SHOW_ISSUE, false, &technical);
        assert_eq!(state.step, Step::Preview(0));
        let shown = state.request.clone().unwrap();
        assert_eq!(shown["happened"], "The sky stays black after I sleep");
        assert!(labels(&state).contains(&SEND) && !labels(&state).contains(&PREVIOUS));
        // Typing in the preview changes nothing: the fields are not there.
        typed(&mut state, "something else entirely");
        assert_eq!(state.request.clone().unwrap(), shown);
        assert_eq!(press(&mut state, EDIT, false, &technical), Effect::None);
        assert_eq!(state.step, Step::Write);
        press(&mut state, SHOW_ISSUE, false, &technical);
        assert_eq!(
            press(&mut state, SEND, false, &technical),
            Effect::Post(shown)
        );
        assert_eq!(state.step, Step::Sending);
        // A second press while it is on its way posts nothing.
        assert_eq!(press(&mut state, SEND, false, &technical), Effect::None);
    }

    #[test]
    fn a_long_issue_spans_pages_and_next_stops_at_the_last() {
        let mut state = State::new();
        typed(&mut state, &"The sky stays black. ".repeat(20));
        press(&mut state, SHOW_ISSUE, false, &technical);
        let pages = state.pages();
        assert!(pages > 1);
        for _ in 0..pages + 3 {
            press(&mut state, NEXT, false, &technical);
        }
        assert_eq!(state.step, Step::Preview(pages - 1));
        assert!(!labels(&state).contains(&NEXT));
        press(&mut state, PREVIOUS, false, &technical);
        assert_eq!(state.step, Step::Preview(pages - 2));
    }

    #[test]
    fn a_failed_report_can_be_sent_again() {
        let mut state = State::new();
        typed(&mut state, "The sky stays black after I sleep");
        press(&mut state, SHOW_ISSUE, false, &technical);
        press(&mut state, SEND, false, &technical);
        state.step = Step::Failed("429: too many reports".to_owned());
        assert!(matches!(
            press(&mut state, RETRY, false, &technical),
            Effect::Post(_)
        ));
        assert_eq!(state.step, Step::Sending);
    }
}
