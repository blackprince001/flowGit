//! Actions: workflow runs with a workflow filter and dispatch, and a run page with
//! its jobs, steps and logs. Running runs are polled while visible.
use super::ui::*;
use super::*;
use crate::{
    gh::{InputKind, RunAction, WorkflowInput},
    icons::Glyph,
    menu::{Entry, Item},
    util,
};
use gpui::prelude::FluentBuilder;
use gpui_component::{
    Disableable,
    IconName, Sizable,
    button::{Button, ButtonVariants, DropdownButton},
    input::{Input, InputState},
    menu::PopupMenuItem,
    resizable::*,
    switch::Switch,
};

const LOG_ROW: f32 = 20.;
const RUN_ROW: f32 = 60.;
/// How often the open run's jobs are re-read while it is running.
const JOBS_POLL: Duration = Duration::from_secs(3);

/// The inline panel that starts a workflow with inputs. It lives on the tab and
/// is not saved with the session.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum DispatchPanel {
    #[default]
    Closed,
    /// The workflow to run and the branch it runs on. Its fields are on the tab.
    Open { workflow: u64, branch: String },
}

/// One input on the open panel: its definition, and what is entered for it.
pub struct DispatchField {
    pub input: WorkflowInput,
    pub value: FieldValue,
}

pub enum FieldValue {
    Text(Entity<InputState>),
    Flag(bool),
    /// An index into the choice's options.
    Pick(usize),
}

impl FieldValue {
    fn read(&self, cx: &App) -> Value {
        match self {
            FieldValue::Text(e) => Value::Text(e.read(cx).value().to_string()),
            FieldValue::Flag(on) => Value::Flag(*on),
            FieldValue::Pick(i) => Value::Pick(*i),
        }
    }
}

/// A field's value as plain data, read out of its entity when the run starts.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Text(String),
    Flag(bool),
    Pick(usize),
}

/// The `name=value` pairs a run sends, or why it can't start yet. Optional text
/// left empty is not sent, so the workflow's own default applies.
pub fn dispatch_pairs(fields: &[(WorkflowInput, Value)]) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for (input, value) in fields {
        match (&input.kind, value) {
            (_, Value::Flag(on)) => out.push((input.name.clone(), on.to_string())),
            (InputKind::Choice(options), Value::Pick(i)) => {
                let option = options.get(*i).ok_or_else(|| format!("Pick a value for {}", input.name))?;
                out.push((input.name.clone(), option.clone()));
            }
            (kind, Value::Text(text)) => {
                let text = text.trim();
                if text.is_empty() {
                    if input.required {
                        return Err(format!("{} is required", input.name));
                    }
                    continue;
                }
                if *kind == InputKind::Number && text.parse::<f64>().is_err() {
                    return Err(format!("{} must be a number", input.name));
                }
                out.push((input.name.clone(), text.to_string()));
            }
            _ => return Err(format!("{} has no value", input.name)),
        }
    }
    Ok(out)
}

impl Workspace {
    pub(super) fn actions_view(&self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match self.current().and_then(|t| t.run) {
            Some(run) => self.run_page(run, cx),
            None => self.run_list(cx),
        }
    }

    fn confirm_dispatch(&mut self, workflow: u64, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(branch) = self.current().and_then(|t| self.branch_of(t)) else { return };
        let prompt = window.prompt(
            PromptLevel::Info,
            &format!("Run “{name}” on {branch}?"),
            Some("Starts a workflow_dispatch run. Workflows without that trigger will refuse."),
            &["Run workflow", "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if prompt.await == Ok(0) {
                let _ = this.update(cx, |s, cx| {
                    s.run_action(0, RunAction::Dispatch { workflow, reference: branch, inputs: Vec::new() }, cx)
                });
            }
        })
        .detach();
    }

    fn run_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let Some(t) = self.current() else { return div().into_any_element() };
        let data = self.data(&t.repo);
        let workflows = data.and_then(|d| d.workflows.data.clone()).unwrap_or_default();
        let current_name = t
            .workflow
            .and_then(|id| workflows.iter().find(|w| w.id == id))
            .map(|w| w.name.clone())
            .unwrap_or_else(|| "All workflows".into());
        let filter = self.popover_button(
            "workflow-filter",
            button("workflow-filter-trigger", current_name)
                .icon(icon(Glyph::Workflow).text_color(rgb(colors.muted)))
                .dropdown_caret(true),
            Some("Find a workflow…"),
            320.,
            |s, weak, _| {
                let Some(t) = s.current() else { return vec![] };
                let current = t.workflow;
                let w = weak.clone();
                let mut out = vec![Entry::Item(Item::new("All workflows").checked(current.is_none()).on_click(move |_, cx| {
                    let _ = w.update(cx, |s, cx| {
                        if let Some(t) = s.current_mut() {
                            t.workflow = None;
                        }
                        cx.notify();
                    });
                }))];
                if let Some(list) = s.data(&t.repo).and_then(|d| d.workflows.data.as_ref()) {
                    out.push(Entry::Separator);
                    for wf in list {
                        let (id, w) = (wf.id, weak.clone());
                        out.push(Entry::Item(
                            Item::new(wf.name.clone())
                                .icon(Glyph::Workflow)
                                .checked(current == Some(id))
                                .detail(if wf.state == "active" { "" } else { "disabled" })
                                .on_click(move |_, cx| {
                                    let _ = w.update(cx, |s, cx| {
                                        if let Some(t) = s.current_mut() {
                                            t.workflow = Some(id);
                                        }
                                        cx.notify();
                                    });
                                }),
                        ));
                    }
                }
                out
            },
            cx,
        );
        // Only workflows with a workflow_dispatch trigger can be started by hand.
        let runnable = data
            .and_then(|d| d.workflows.data.as_ref())
            .map(|list| list.iter().filter(|w| w.dispatchable).count());
        let dispatch = self.popover_button(
            "dispatch",
            Button::new("dispatch-trigger")
                .label("Run workflow")
                .icon(Glyph::Play)
                .outline()
                .small()
                .dropdown_caret(true)
                .loading(self.busy_with("Starting workflow"))
                .disabled(self.busy.is_some() || runnable == Some(0))
                .tooltip(match runnable {
                    Some(0) => "No workflow here has a workflow_dispatch trigger",
                    Some(_) => "Start a workflow that has a workflow_dispatch trigger",
                    None => "Checking which workflows can be run…",
                }),
            Some("Find a workflow…"),
            320.,
            |s, weak, _| {
                let Some(t) = s.current() else { return vec![] };
                let branch = s.branch_of(t).unwrap_or_default();
                let mut out = vec![Entry::Label(format!("Run on {branch}").into())];
                let Some(list) = s.data(&t.repo).and_then(|d| d.workflows.data.as_ref()) else {
                    return vec![Entry::Item(Item::new("Checking workflows…").disabled(true))];
                };
                {
                    for wf in list.iter().filter(|w| w.dispatchable) {
                        let (id, name, w) = (wf.id, wf.name.clone(), weak.clone());
                        let has_inputs = !wf.inputs.is_empty();
                        out.push(Entry::Item(Item::new(wf.name.clone()).icon(Glyph::Workflow).detail(wf.path.rsplit('/').next().unwrap_or("").to_string()).on_click(
                            move |win, cx| {
                                let name = name.clone();
                                let _ = w.update(cx, |s, cx| {
                                    // A workflow with inputs gets the panel; the others start at once.
                                    if has_inputs {
                                        s.open_dispatch(id, win, cx)
                                    } else {
                                        s.confirm_dispatch(id, name, win, cx)
                                    }
                                });
                            },
                        )));
                    }
                }
                out
            },
            cx,
        );
        let l = data.and_then(|d| d.runs.get(&t.workflow));
        let header = row()
            .h(px(BAR + 6.))
            .flex_shrink_0()
            .px_2()
            .gap_1()
            .border_b_1()
            .border_color(rgb(colors.row_border))
            .child(filter)
            .when(t.view == View::Actions, |s| s.child(divider(colors)).child(self.branch_button("actions-branch", self.branch_of(t).unwrap_or_else(|| "…".into()), cx)))
            .child(div().flex_1())
            .child(dispatch);
        let body = match state(colors, l, "Loading workflow runs…") {
            Err(p) => p.into_any_element(),
            Ok(()) => {
                let runs = l.and_then(|l| l.data.as_ref()).unwrap();
                if runs.is_empty() {
                    empty(colors, Glyph::CirclePlay, "No workflow runs", "Runs appear here once a workflow is triggered.").into_any_element()
                } else {
                    let runs = Arc::new(runs.clone());
                    uniform_list(
                        ("runs", t.id),
                        runs.len(),
                        cx.processor(move |s, range: std::ops::Range<usize>, _, cx| {
                            range.map(|i| s.run_row(i, &runs, cx)).collect()
                        }),
                    )
                    .track_scroll(t.list_scroll.clone())
                    .size_full()
                    .into_any_element()
                }
            }
        };
        col()
            .size_full()
            .bg(rgb(colors.background))
            .child(header)
            .children(self.dispatch_panel(t, cx))
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }

    /// One workflow run row; fixed height so the list can virtualize.
    /// One run on the timeline: when it started, its status on the rail, and what
    /// ran. Fixed height so the list can virtualize.
    fn run_row(&self, i: usize, runs: &[gh::Run], cx: &mut Context<Self>) -> Stateful<Div> {
        let colors = palette(cx);
        let mono = settings::mono(cx);
        let r = &runs[i];
        let id = r.database_id;
        let default = self.current().and_then(|t| self.default_branch(&t.repo));
        let tags = self
            .current()
            .and_then(|t| self.data(&t.repo)?.releases.data.as_ref());
        let color = |r: &gh::Run| run_glyph(colors, &r.status, &r.conclusion).1;
        let at = util::parse_time(&r.created_at);
        let show_day = i == 0 || !same_day(util::parse_time(&runs[i - 1].created_at), at);
        let started = r.started_at.as_deref().or(Some(r.created_at.as_str()));
        let took = util::span(started, (!r.active()).then_some(r.updated_at.as_str()));
        let dot = r
            .status
            .eq_ignore_ascii_case("in_progress")
            .then(|| spinner(colors).into_any_element());
        div()
            .id(("run", id))
            .flex()
            .w_full()
            .h(px(RUN_ROW))
            .pl_2()
            .pr_3()
            .gap_2()
            .cursor_pointer()
            .hover(|s| s.bg(rgb(colors.hover)))
            .child(date_column(colors, at, show_day, 12.))
            .child(rail(
                i.checked_sub(1).map(|p| color(&runs[p])),
                color(r),
                runs.get(i + 1).map(color),
                14.,
                dot,
                colors.background,
            ))
            .child(
                col()
                    .flex_1()
                    .min_w_0()
                    .pt(px(11.))
                    .gap(px(4.))
                    .child(
                        row()
                            .gap_2()
                            .child(div().flex_1().min_w_0().truncate().font_weight(FontWeight::MEDIUM).child(r.display_title.clone()))
                            .when_some(took, |s, d| {
                                s.child(
                                    div()
                                        .flex_shrink_0()
                                        .text_size(px(12.))
                                        .text_color(rgb(colors.muted))
                                        .child(d),
                                )
                            }),
                    )
                    .child(
                        row()
                            .gap(px(6.))
                            .overflow_hidden()
                            .text_size(px(12.))
                            .text_color(rgb(colors.muted))
                            .child(div().flex_shrink_0().child(r.workflow_name.clone()))
                            .child(div().flex_shrink_0().font_family(mono.clone()).text_size(px(11.5)).child(format!("#{}", r.number)))
                            .child(div().text_color(rgb(colors.quiet)).child("·"))
                            .child(r.event.replace('_', " "))
                            .when(!r.head_branch.is_empty(), |s| {
                                // Merged branches are often deleted, so test for "not default, not a tag".
                                let side = default.as_deref().is_some_and(|d| d != r.head_branch)
                                    && !tags.as_ref().is_some_and(|t| t.iter().any(|t| t.tag_name == r.head_branch));
                                let (fg, bg) = if side {
                                    (colors.pink, colors.pink_tint)
                                } else {
                                    (colors.blue, colors.blue_tint)
                                };
                                s.child(tinted_chip(fg, bg, mono.clone(), r.head_branch.clone()))
                            }),
                    ),
            )
            .on_click(cx.listener(move |s, _, _, cx| {
                if let Some(t) = s.current_mut() {
                    t.run = Some(id);
                    t.job = None;
                    t.log_open = false;
                }
                cx.notify();
            }))
    }

    fn run_page(&self, id: u64, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let mono = settings::mono(cx);
        let Some(t) = self.current() else { return div().into_any_element() };
        let data = self.data(&t.repo);
        let run = data
            .and_then(|d| d.runs.values().find_map(|l| l.data.as_ref()?.iter().find(|r| r.database_id == id)))
            .cloned();
        let jobs_l = data.and_then(|d| d.jobs.get(&id));
        let crumb = row()
            .h(px(BAR))
            .flex_shrink_0()
            .px_3()
            .gap_1()
            .text_size(px(13.))
            .child(
                row()
                    .id("crumb-actions")
                    .gap(px(4.))
                    .cursor_pointer()
                    .text_color(rgb(colors.blue))
                    .child(icon(IconName::ChevronLeft).size(px(13.)))
                    .child("Actions")
                    .on_click(cx.listener(|s, _, _, cx| {
                        if let Some(t) = s.current_mut() {
                            t.run = None;
                        }
                        cx.notify();
                    })),
            )
            .child(div().text_color(rgb(colors.quiet)).child("/"))
            .child(div().text_color(rgb(colors.muted)).child(match &run {
                Some(r) => format!("{} #{}", r.workflow_name, r.number),
                None => format!("Run {id}"),
            }))
;
        let busy = self.busy.is_some();
        let head = run.as_ref().map(|r| {
            let (glyph, color) = run_glyph(colors, &r.status, &r.conclusion);
            let failed = r.conclusion == "failure";
            let url = r.url.clone();
            let outcome = if r.active() { r.status.replace('_', " ") } else { r.conclusion.clone() };
            col()
                .flex_shrink_0()
                .px_4()
                .pb_3()
                .gap_2()
                .child(
                    row()
                        .gap_3()
                        .child(icon(glyph).size(px(20.)).text_color(rgb(color)))
                        .child(div().flex_1().min_w_0().truncate().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(r.display_title.clone()))
                        .when(r.active(), |s| {
                            s.child(
                                Button::new("run-cancel")
                                    .label("Cancel run")
                                    .icon(Glyph::Stop)
                                    .outline()
                                    .small()
                                    .loading(self.busy_with("Cancelling"))
                                    .disabled(busy)
                                    .on_click(cx.listener(move |s, _, _, cx| s.run_action(id, RunAction::Cancel, cx))),
                            )
                        })
                        .when(!r.active() && failed, |s| {
                            s.child(
                                Button::new("rerun-failed")
                                    .label("Re-run failed jobs")
                                    .icon(Glyph::RotateCcw)
                                    .outline()
                                    .small()
                                    .loading(self.busy_with("Re-running failed"))
                                    .disabled(busy)
                                    .on_click(cx.listener(move |s, _, _, cx| s.run_action(id, RunAction::RerunFailed, cx))),
                            )
                        })
                        .when(!r.active(), |s| {
                            s.child(
                                Button::new("rerun-all")
                                    .label("Re-run all")
                                    .icon(Glyph::RotateCcw)
                                    .primary()
                                    .small()
                                    .loading(self.busy_with("Re-running all"))
                                    .disabled(busy)
                                    .on_click(cx.listener(move |s, _, _, cx| s.run_action(id, RunAction::Rerun, cx))),
                            )
                        })
                        .child(icon_button("run-github", IconName::ExternalLink, "Open run on GitHub").on_click(move |_, _, cx| cx.open_url(&url))),
                )
                .child(
                    row()
                        .gap_2()
                        .text_size(px(12.5))
                        .text_color(rgb(colors.muted))
                        .child(pill(outcome, color, if color == colors.green { colors.green_tint } else if color == colors.red { colors.red_tint } else { colors.amber_tint }))
                        .child(r.event.replace('_', " "))
                        .child("on")
                        .child(chip(colors, mono.clone(), r.head_branch.clone()))
                        .child(div().text_color(rgb(colors.quiet)).child("·"))
                        .child(format!("started {}", self.when(util::parse_time(r.started_at.as_deref().unwrap_or(&r.created_at)), cx)))
                        .when_some(util::span(r.started_at.as_deref(), (!r.active()).then_some(r.updated_at.as_str())), |s, d| s.child(format!("· {d}"))),
                )
        });
        let body = match state(colors, jobs_l, "Loading jobs…") {
            Err(p) => p.into_any_element(),
            Ok(()) => {
                let jobs = jobs_l.and_then(|l| l.data.as_ref()).unwrap();
                let selected = t.job.or_else(|| jobs.first().map(|j| j.database_id));
                let mut list = col().p_1().gap(px(1.));
                for j in jobs {
                    let jid = j.database_id;
                    let active = selected == Some(jid);
                    list = list.child(
                        row()
                            .id(("job", jid))
                            .h(px(32.))
                            .px_2()
                            .gap_2()
                            .rounded(px(5.))
                            .cursor_pointer()
                            .when(active, |s| s.bg(rgb(colors.selected)))
                            .when(!active, |s| s.hover(|s| s.bg(rgb(colors.hover))))
                            .child(run_icon(colors, &j.status, &j.conclusion))
                            .child(div().flex_1().min_w_0().truncate().child(j.name.clone()))
                            .when_some(util::span(j.started_at.as_deref(), j.completed_at.as_deref()), |s, d| s.child(label(colors, d)))
                            .on_click(cx.listener(move |s, _, _, cx| {
                                if let Some(t) = s.current_mut() {
                                    t.job = Some(jid);
                                }
                                cx.notify();
                            })),
                    );
                }
                if jobs.is_empty() {
                    list = list.child(div().p_3().text_color(rgb(colors.muted)).child("No jobs yet"));
                }
                let job = selected.and_then(|s| jobs.iter().find(|j| j.database_id == s));
                h_resizable(SharedString::from(format!("run-jobs-{}", t.id)))
                    .child(
                        resizable_panel()
                            .size(px(260.))
                            .size_range(px(180.)..px(460.))
                            .child(
                                col()
                                    .size_full()
                                    .bg(rgb(colors.chrome))
                                    .border_r_1()
                                    .border_color(rgb(colors.border))
                                    .child(section(colors, format!("Jobs · {}", jobs.len())).px_3())
                                    .child(div().id("jobs").flex_1().min_h_0().overflow_y_scroll().child(list)),
                            ),
                    )
                    .child(resizable_panel().child(match job {
                        Some(j) => self.job_pane(t, j, cx),
                        None => empty(colors, Glyph::Workflow, "No job selected", "").into_any_element(),
                    }))
                    .into_any_element()
            }
        };
        col()
            .size_full()
            .bg(rgb(colors.background))
            .child(crumb)
            .children(head)
            .child(div().flex_1().min_h_0().border_t_1().border_color(rgb(colors.border)).child(body))
            .into_any_element()
    }

    fn job_pane(&self, t: &Tab, j: &gh::Job, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let jid = j.database_id;
        let log_open = t.log_open;
        // With the log open and loaded: the log, and the rows the filter keeps.
        let loaded = if log_open { self.data(&t.repo).and_then(|d| d.log.get(&jid)) } else { None };
        let query = t.log_search.read(cx).value().to_string();
        let shown = loaded.and_then(|l| l.data.clone()).map(|log| {
            let rows = filter_log(&log.rows, &query);
            (log, rows)
        });
        let header = row()
            .h(px(BAR + 4.))
            .flex_shrink_0()
            .px_3()
            .gap_2()
            .border_b_1()
            .border_color(rgb(colors.row_border))
            .child(run_icon(colors, &j.status, &j.conclusion))
            .child(div().font_weight(FontWeight::MEDIUM).truncate().child(j.name.clone()))
            .when_some(util::span(j.started_at.as_deref(), j.completed_at.as_deref()), |s, d| s.child(label(colors, d)))
            .child(div().flex_1())
            .when_some(shown.as_ref(), |s, (log, rows)| {
                let matches = match_count(&log.rows, rows);
                s.child(
                    row()
                        .gap_2()
                        .child(
                            div().w(px(220.)).child(
                                Input::new(&t.log_search)
                                    .small()
                                    .prefix(icon(IconName::Search).text_color(rgb(colors.muted)))
                                    .cleanable(true),
                            ),
                        )
                        .when(!query.is_empty(), |s| {
                            s.child(label(colors, if matches == 1 { "1 match".to_string() } else { format!("{matches} matches") }))
                        }),
                )
            })
            .child(crate::workspace::pulls::segmented(
                colors,
                "job-mode",
                &[(false, "Steps".to_string()), (true, "Log".to_string())],
                log_open,
                move |s, on, _| {
                    if let Some(t) = s.current_mut() {
                        t.job = Some(jid);
                        t.log_open = on;
                    }
                },
                cx,
            ));
        let body = if log_open {
            match (shown, loaded.and_then(|l| l.error.clone())) {
                (Some((log, rows)), _) => self.log_list(t, log, rows, cx),
                (None, Some(e)) => failed(colors, &e).into_any_element(),
                _ if !j.status.eq_ignore_ascii_case("completed") => {
                    empty(colors, Glyph::Terminal, "Log available when the job finishes", "gh streams complete job logs only.").into_any_element()
                }
                _ => loading(colors, "Downloading log…").into_any_element(),
            }
        } else {
            let mut steps = col().p_2();
            for (i, s) in j.steps.iter().enumerate() {
                steps = steps.child(
                    row()
                        .h(px(30.))
                        .px_2()
                        .gap_3()
                        .rounded(px(5.))
                        .when(i % 2 == 1, |d| d.bg(rgb(colors.panel)))
                        .child(run_icon(colors, &s.status, &s.conclusion))
                        .child(div().w(px(22.)).text_color(rgb(colors.quiet)).text_size(px(12.)).child(s.number.to_string()))
                        .child(div().flex_1().min_w_0().truncate().child(s.name.clone()))
                        .when_some(util::span(s.started_at.as_deref(), s.completed_at.as_deref()), |d, t| d.child(label(colors, t))),
                );
            }
            if j.steps.is_empty() {
                steps = steps.child(div().p_3().text_color(rgb(colors.muted)).child("Steps appear once the job starts"));
            }
            div().id(("steps", jid)).size_full().overflow_y_scroll().child(steps).into_any_element()
        };
        col().size_full().child(header).child(div().flex_1().min_h_0().child(body)).into_any_element()
    }

    /// Show the rows `shown` keeps from the log. Line numbers are the full log's.
    fn log_list(&self, t: &Tab, log: Arc<LogView>, shown: Vec<usize>, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let mono = settings::mono(cx);
        let size = settings::get(cx).mono_size as f32;
        let count = shown.len();
        let digits = log.rows.len().to_string().len() as f32;
        let truncated = log.truncated;
        if count == 0 && !log.rows.is_empty() {
            return div().p_3().text_color(rgb(colors.muted)).child("No lines match").into_any_element();
        }
        let list = uniform_list(
            ("log", t.id),
            count,
            cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
                let colors = palette(cx);
                range
                    .map(|i| {
                        let line = shown[i];
                        let (header, text) = &log.rows[line];
                        if *header {
                            row()
                                .h(px(LOG_ROW))
                                .w_full()
                                .px_3()
                                .gap_2()
                                .bg(rgb(colors.chrome))
                                .text_color(rgb(colors.blue))
                                .font_weight(FontWeight::MEDIUM)
                                .child(icon(IconName::ChevronRight).size(px(11.)))
                                .child(text.clone())
                        } else {
                            let lower = text.to_lowercase();
                            let tone = if lower.contains("error") || lower.starts_with("##[error]") {
                                colors.red
                            } else if lower.contains("warning") || lower.starts_with("##[warning]") {
                                colors.amber
                            } else if text.starts_with("##[group]") || text.starts_with("##[endgroup]") {
                                colors.muted
                            } else {
                                colors.text
                            };
                            row()
                                .h(px(LOG_ROW))
                                .w_full()
                                .hover(|s| s.bg(rgb(colors.chrome)))
                                .child(div().w(px(16. + digits * 8.)).flex_shrink_0().pr_2().text_right().text_color(rgb(colors.quiet)).child((line + 1).to_string()))
                                .child(div().pl_2().whitespace_nowrap().text_color(rgb(tone)).child(text.clone()))
                        }
                    })
                    .collect()
            }),
        )
        .track_scroll(t.log_scroll.clone())
        // Inside a horizontal scroller: keep sideways swipes from also scrolling rows.
        .map(|mut list| {
            list.style().restrict_scroll_to_axis = Some(true);
            list
        })
        .h_full()
        .min_w(px(1600.))
        .w_full();
        col()
            .size_full()
            .when(truncated, |s| {
                s.child(
                    row()
                        .h(px(24.))
                        .px_3()
                        .gap_2()
                        .bg(rgb(colors.amber_tint))
                        .text_size(px(12.))
                        .child(icon(IconName::Info).size(px(12.)).text_color(rgb(colors.amber)))
                        .child(format!("Showing the last {} lines", util::thousands(gh::LOG_LIMIT as u64))),
                )
            })
            .child(
                div()
                    .id(("log-x", t.id))
                    .flex_1()
                    .min_h_0()
                    .overflow_x_scroll()
                    .map(|mut d| {
                        d.style().restrict_scroll_to_axis = Some(true);
                        d
                    })
                    .font_family(mono)
                    .text_size(px(size))
                    .child(list),
            )
            .into_any_element()
    }
}

impl Workspace {
    /// Open the panel for a workflow that takes inputs, aimed at the default branch.
    /// Opening the workflow already open keeps what was typed.
    pub(super) fn open_dispatch(&mut self, workflow: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.current() else { return };
        if matches!(&t.dispatch, DispatchPanel::Open { workflow: open, .. } if *open == workflow) {
            return;
        }
        let repo = t.repo.clone();
        let Some(wf) = self
            .data(&repo)
            .and_then(|d| d.workflows.data.as_ref())
            .and_then(|list| list.iter().find(|w| w.id == workflow))
            .cloned()
        else {
            return;
        };
        let branch = self.default_branch(&repo).unwrap_or_default();
        let fields = wf
            .inputs
            .into_iter()
            .map(|spec| {
                let value = match &spec.kind {
                    InputKind::Boolean => FieldValue::Flag(spec.default == "true"),
                    InputKind::Choice(options) => FieldValue::Pick(options.iter().position(|o| *o == spec.default).unwrap_or(0)),
                    _ => FieldValue::Text(input(&spec.default, &spec.description, window, cx)),
                };
                DispatchField { input: spec, value }
            })
            .collect();
        let Some(t) = self.current_mut() else { return };
        t.dispatch = DispatchPanel::Open { workflow, branch };
        t.dispatch_fields = fields;
        cx.notify();
    }

    fn close_dispatch(&mut self, cx: &mut Context<Self>) {
        if let Some(t) = self.current_mut() {
            t.dispatch = DispatchPanel::Closed;
            t.dispatch_fields.clear();
        }
        cx.notify();
    }

    fn set_dispatch_branch(&mut self, name: String, cx: &mut Context<Self>) {
        if let Some(DispatchPanel::Open { branch, .. }) = self.current_mut().map(|t| &mut t.dispatch) {
            *branch = name;
        }
        cx.notify();
    }

    fn set_dispatch_flag(&mut self, field: usize, on: bool, cx: &mut Context<Self>) {
        if let Some(FieldValue::Flag(value)) = self.current_mut().and_then(|t| t.dispatch_fields.get_mut(field)).map(|f| &mut f.value) {
            *value = on;
        }
        cx.notify();
    }

    fn set_dispatch_pick(&mut self, field: usize, choice: usize, cx: &mut Context<Self>) {
        if let Some(f) = self.current_mut().and_then(|t| t.dispatch_fields.get_mut(field)) {
            f.value = FieldValue::Pick(choice);
        }
        cx.notify();
    }

    /// Start the run through `mutate`. The panel closes once GitHub accepts it, so a
    /// refused run keeps what was typed.
    fn submit_dispatch(&mut self, cx: &mut Context<Self>) {
        let Some(t) = self.current() else { return };
        let DispatchPanel::Open { workflow, branch } = t.dispatch.clone() else { return };
        let values = t.dispatch_fields.iter().map(|f| (f.input.clone(), f.value.read(cx))).collect::<Vec<_>>();
        let Ok(inputs) = dispatch_pairs(&values) else { return };
        let Some(repo) = self.repo(&t.repo).cloned() else { return };
        let (key, slug, tab) = (repo.key(), repo.slug(), t.id);
        let label = format!("Starting workflow on {branch}");
        self.mutate(
            label,
            move || gh::run_action(&slug, 0, &RunAction::Dispatch { workflow, reference: branch, inputs }),
            move |s, cx| {
                if let Some(t) = s.tabs.iter_mut().find(|t| t.id == tab) {
                    t.dispatch = DispatchPanel::Closed;
                    t.dispatch_fields.clear();
                }
                if let Some(d) = s.data.get_mut(&key) {
                    d.runs.clear();
                }
                s.last_poll = Instant::now();
                s.ensure(false, cx);
            },
            cx,
        );
    }

    /// The panel above the run list, for a workflow that takes inputs. It sits
    /// inline, like the release draft, not in a modal.
    fn dispatch_panel(&self, t: &Tab, cx: &mut Context<Self>) -> Option<AnyElement> {
        let DispatchPanel::Open { workflow, branch } = &t.dispatch else { return None };
        let colors = palette(cx);
        let name = self
            .data(&t.repo)
            .and_then(|d| d.workflows.data.as_ref())
            .and_then(|list| list.iter().find(|w| w.id == *workflow))
            .map(|w| w.name.clone())
            .unwrap_or_else(|| "workflow".into());
        let values = t.dispatch_fields.iter().map(|f| (f.input.clone(), f.value.read(cx))).collect::<Vec<_>>();
        let check = dispatch_pairs(&values);
        let hint = check.as_ref().err().cloned().unwrap_or_default();
        let weak = cx.weak_entity();
        let branch_picker = self.popover_button(
            "dispatch-branch",
            Button::new("dispatch-branch-trigger")
                .label(branch.clone())
                .icon(icon(Glyph::Branch).text_color(rgb(colors.muted)))
                .outline()
                .small()
                .dropdown_caret(true),
            Some("Find a branch…"),
            320.,
            |s, weak, _| {
                let Some(t) = s.current() else { return vec![] };
                let DispatchPanel::Open { branch, .. } = &t.dispatch else { return vec![] };
                let current = branch.clone();
                let Some(list) = s.data(&t.repo).and_then(|d| d.branches.data.as_ref()) else {
                    return vec![Entry::Item(Item::new("Loading branches…").disabled(true))];
                };
                let default = s.default_branch(&t.repo);
                let mut sorted = list.iter().collect::<Vec<_>>();
                // Default first, then alphabetical, as the Code view lists them.
                sorted.sort_by_key(|b| (Some(&b.name) != default.as_ref(), b.name.to_lowercase()));
                let mut out = vec![Entry::Label(format!("Branches · {}", list.len()).into())];
                for b in sorted {
                    let (name, w) = (b.name.clone(), weak.clone());
                    out.push(Entry::Item(
                        Item::new(b.name.clone())
                            .icon(if b.protected { Glyph::Lock } else { Glyph::Branch })
                            .checked(b.name == current)
                            .detail(if Some(&b.name) == default.as_ref() { "default" } else { "" })
                            .on_click(move |_, cx| {
                                let name = name.clone();
                                let _ = w.update(cx, |s, cx| s.set_dispatch_branch(name, cx));
                            }),
                    ));
                }
                out
            },
            cx,
        );
        let mut form = col().gap_3().child(field(colors, "Branch", branch_picker));
        for (i, f) in t.dispatch_fields.iter().enumerate() {
            form = form.child(self.dispatch_field(i, f, &weak, colors));
        }
        let busy = self.busy.is_some();
        let footer = row()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .text_color(rgb(colors.muted))
                    .child(hint),
            )
            .child(button("dispatch-cancel", "Cancel").on_click(cx.listener(|s, _, _, cx| s.close_dispatch(cx))))
            .child(
                Button::new("dispatch-run")
                    .label("Run workflow")
                    .primary()
                    .small()
                    .loading(self.busy_with("Starting workflow"))
                    .disabled(busy || check.is_err())
                    .on_click(cx.listener(|s, _, _, cx| s.submit_dispatch(cx))),
            );
        Some(
            col()
                .px_4()
                .pt_4()
                .child(
                    card(colors)
                        .child(
                            row()
                                .h(px(BAR))
                                .px_4()
                                .border_b_1()
                                .border_color(rgb(colors.row_border))
                                .child(section(colors, format!("Run {name}"))),
                        )
                        .child(col().p_4().gap_3().child(form).child(footer)),
                )
                .into_any_element(),
        )
    }

    /// One input's control: text, a switch, or a dropdown of its options.
    fn dispatch_field(&self, i: usize, f: &DispatchField, weak: &WeakEntity<Self>, colors: Palette) -> Div {
        let control = match &f.value {
            FieldValue::Text(entity) => Input::new(entity).small().into_any_element(),
            FieldValue::Flag(on) => {
                let weak = weak.clone();
                Switch::new(("dispatch-flag", i as u64))
                    .checked(*on)
                    .on_click(move |on: &bool, _, cx| {
                        let on = *on;
                        let _ = weak.update(cx, |s, cx| s.set_dispatch_flag(i, on, cx));
                    })
                    .into_any_element()
            }
            FieldValue::Pick(sel) => {
                let options = match &f.input.kind {
                    InputKind::Choice(options) => options.clone(),
                    _ => Vec::new(),
                };
                let (weak, sel) = (weak.clone(), *sel);
                let current = options.get(sel).cloned().unwrap_or_default();
                DropdownButton::new(("dispatch-choice", i as u64))
                    .button(
                        Button::new(("dispatch-choice-trigger", i as u64))
                            .label(current)
                            .outline()
                            .small(),
                    )
                    .dropdown_menu(move |menu, _, _| {
                        options.iter().enumerate().fold(menu, |menu, (j, option)| {
                            let weak = weak.clone();
                            menu.item(PopupMenuItem::new(option.clone()).checked(j == sel).on_click(move |_, _, cx| {
                                let _ = weak.update(cx, |s, cx| s.set_dispatch_pick(i, j, cx));
                            }))
                        })
                    })
                    .into_any_element()
            }
        };
        col()
            .min_w_0()
            .gap_1()
            .child(label(colors, f.input.name.clone()))
            .child(control)
            .when(!f.input.description.is_empty(), |s| s.child(label(colors, f.input.description.clone())))
    }

    /// Re-read the open run's jobs every few seconds while it runs, and load the
    /// selected job's log once that job has finished.
    pub(super) fn poll_jobs(&mut self, cx: &mut Context<Self>) {
        let Some(t) = self.current() else { return };
        if t.view != View::Actions {
            return;
        }
        let (Some(run), key) = (t.run, t.repo.clone()) else { return };
        let Some(slug) = self.repo(&key).map(|r| r.slug()) else { return };
        if self.run_active(&key, run) && self.last_jobs_poll.elapsed() >= JOBS_POLL {
            self.last_jobs_poll = Instant::now();
            let s = slug.clone();
            self.fetch(&key, move |d| d.jobs.entry(run).or_default(), true, move || gh::jobs(&s, run), cx);
        }
        self.ensure_job_log(&key, &slug, run, false, cx);
    }

    /// Whether the run is queued or running, by its own status or by any of its jobs.
    fn run_active(&self, key: &str, run: u64) -> bool {
        let Some(d) = self.data(key) else { return false };
        let listed = d
            .runs
            .values()
            .filter_map(|l| l.data.as_ref())
            .flatten()
            .any(|r| r.database_id == run && r.active());
        listed
            || d.jobs
                .get(&run)
                .and_then(|l| l.data.as_ref())
                .is_some_and(|jobs| jobs.iter().any(gh::Job::active))
    }

    /// Load the log of the selected job, once it has finished. GitHub serves no log
    /// for a job that is still running.
    pub(super) fn ensure_job_log(&mut self, key: &str, slug: &str, run: u64, force: bool, cx: &mut Context<Self>) {
        let Some(t) = self.current() else { return };
        if !t.log_open {
            return;
        }
        let jobs = self.data(key).and_then(|d| d.jobs.get(&run)).and_then(|l| l.data.as_ref());
        let Some(job) = t.job.or_else(|| jobs?.first().map(|j| j.database_id)) else { return };
        let finished = jobs.is_some_and(|jobs| jobs.iter().any(|j| j.database_id == job && !j.active()));
        if !finished {
            return;
        }
        let (key, s) = (key.to_string(), slug.to_string());
        self.fetch(
            &key,
            move |d| d.log.entry(job).or_default(),
            force,
            move || Ok(Arc::new(load_log(gh::job_log(&s, job)?))),
            cx,
        );
    }
}

/// The rows of a log that `query` keeps, as indices into `rows`. A line is kept
/// when it contains the query, ignoring case. A step header is kept when some
/// line under it is. An empty query keeps everything.
pub fn filter_log<S: AsRef<str>>(rows: &[(bool, S)], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return (0..rows.len()).collect();
    }
    let mut out = Vec::new();
    let mut step = None;
    let mut hits = Vec::new();
    for (i, (header, text)) in rows.iter().enumerate() {
        if *header {
            if !hits.is_empty() {
                out.extend(step);
                out.append(&mut hits);
            }
            step = Some(i);
        } else if text.as_ref().to_lowercase().contains(&query) {
            hits.push(i);
        }
    }
    if !hits.is_empty() {
        out.extend(step);
        out.append(&mut hits);
    }
    out
}

/// How many of the `shown` rows are log lines (step headers don't count).
pub fn match_count<S: AsRef<str>>(rows: &[(bool, S)], shown: &[usize]) -> usize {
    shown.iter().filter(|&&i| !rows[i].0).count()
}

#[cfg(test)]
mod tests {
    // Not `super::*`: gpui's glob brings in its own `test` macro.
    use super::{dispatch_pairs, filter_log, match_count, Value};
    use crate::gh::{InputKind, WorkflowInput};

    fn spec(name: &str, required: bool, kind: InputKind) -> WorkflowInput {
        WorkflowInput {
            name: name.into(),
            description: String::new(),
            required,
            default: String::new(),
            kind,
        }
    }
    fn text(s: &str) -> Value {
        Value::Text(s.into())
    }

    #[test]
    fn a_required_input_must_be_filled() {
        let fields = [(spec("service", true, InputKind::Text), text("   "))];
        assert_eq!(dispatch_pairs(&fields), Err("service is required".to_string()));
        let fields = [(spec("service", true, InputKind::Text), text(" api "))];
        assert_eq!(dispatch_pairs(&fields), Ok(vec![("service".to_string(), "api".to_string())]));
    }

    #[test]
    fn an_empty_optional_input_is_left_out() {
        let fields = [
            (spec("note", false, InputKind::Text), text("")),
            (spec("tag", false, InputKind::Environment), text("prod")),
        ];
        assert_eq!(dispatch_pairs(&fields), Ok(vec![("tag".to_string(), "prod".to_string())]));
    }

    #[test]
    fn numbers_must_parse() {
        let number = spec("replicas", false, InputKind::Number);
        assert_eq!(dispatch_pairs(&[(number.clone(), text("3.5"))]), Ok(vec![("replicas".to_string(), "3.5".to_string())]));
        assert_eq!(dispatch_pairs(&[(number, text("three"))]), Err("replicas must be a number".to_string()));
    }

    #[test]
    fn flags_and_choices_send_their_value() {
        let options = InputKind::Choice(vec!["patch".into(), "minor".into()]);
        let fields = [
            (spec("dry_run", false, InputKind::Boolean), Value::Flag(false)),
            (spec("bump", true, options.clone()), Value::Pick(1)),
        ];
        assert_eq!(
            dispatch_pairs(&fields),
            Ok(vec![("dry_run".to_string(), "false".to_string()), ("bump".to_string(), "minor".to_string())])
        );
        assert!(dispatch_pairs(&[(spec("bump", true, options), Value::Pick(9))]).is_err());
    }

    fn log(lines: &[(bool, &'static str)]) -> Vec<(bool, &'static str)> {
        lines.to_vec()
    }

    #[test]
    fn empty_query_keeps_every_row() {
        let rows = log(&[(true, "Build"), (false, "compile"), (true, "Test"), (false, "run")]);
        assert_eq!(filter_log(&rows, ""), vec![0, 1, 2, 3]);
        assert_eq!(filter_log(&rows, "   "), vec![0, 1, 2, 3]);
    }

    #[test]
    fn keeps_matching_lines_ignoring_case() {
        let rows = log(&[(true, "Build"), (false, "Compiling"), (false, "error: E0308"), (false, "Done")]);
        assert_eq!(filter_log(&rows, "ERROR"), vec![0, 2]);
        assert_eq!(filter_log(&rows, "compil"), vec![0, 1]);
    }

    #[test]
    fn drops_steps_without_a_match() {
        let rows = log(&[
            (true, "Build"),
            (false, "ok"),
            (true, "Test"),
            (false, "failed: boom"),
            (false, "ok"),
            (true, "Upload"),
            (false, "done"),
        ]);
        assert_eq!(filter_log(&rows, "boom"), vec![2, 3]);
        assert_eq!(filter_log(&rows, "nothing here"), Vec::<usize>::new());
    }

    #[test]
    fn keeps_each_step_that_matches() {
        let rows = log(&[(true, "A"), (false, "x"), (true, "B"), (false, "x"), (false, "y")]);
        assert_eq!(filter_log(&rows, "x"), vec![0, 1, 2, 3]);
    }

    #[test]
    fn counts_lines_not_headers() {
        let rows = log(&[(true, "A"), (false, "err one"), (false, "fine"), (true, "B"), (false, "err two")]);
        let shown = filter_log(&rows, "err");
        assert_eq!(match_count(&rows, &shown), 2);
    }
}
