#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use chrono::Local;
use std::rc::Rc;
use std::time::Instant;

use gpui_kit::base::StyledExt;
use gpui_kit::component::button::Button;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::clipboard::Clipboard;
use gpui_kit::component::dialog::{DialogDescription, DialogFooter, DialogHeader, DialogTitle};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::sidebar::{
    Sidebar, SidebarCollapsible, SidebarFooter, SidebarMenu, SidebarMenuItem,
};
use gpui_kit::component::table::{Table, TableBody, TableCell, TableHead, TableHeader, TableRow};
use gpui_kit::component::{
    ActiveTheme, Icon, IconName, Root, Sizable as _, TitleBar, WindowExt, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

mod theme;

use duifene_core::engine::{Engine, EngineConfig, Event as CoreEvent};
use duifene_core::live::LiveClient;
use duifene_core::runner::Runner;
use duifene_core::{api::CheckInResult, api::Client as _, storage};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

const WECHAT_AUTH_URL: &str = "https://open.weixin.qq.com/connect/oauth2/authorize?appid=wx1b5650884f657981&redirect_uri=https://www.duifene.com/_FileManage/PdfView.aspx?file=https%3A%2F%2Ffs.duifene.com%2Fres%2Fr2%2Fu6106199%2F%E5%AF%B9%E5%88%86%E6%98%93%E7%99%BB%E5%BD%95_876c9d439ca68ead389c.pdf&response_type=code&scope=snsapi_userinfo&connect_redirect=1#wechat_redirect";

static LOGIN_TX: std::sync::Mutex<Option<std::sync::mpsc::Sender<LoginMessage>>> =
    std::sync::Mutex::new(None);
static MAIN_WINDOW: std::sync::Mutex<Option<gpui_kit::WindowHandle<gpui_kit::component::Root>>> =
    std::sync::Mutex::new(None);

pub struct AppAssets;

impl gpui_kit::AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<std::borrow::Cow<'static, [u8]>>> {
        if path == "icons/pen.svg" {
            return Ok(Some(std::borrow::Cow::Borrowed(include_bytes!(
                "../assets/pen.svg"
            ))));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        gpui_kit::assets::Assets.list(path)
    }
}

actions!(duifene, [Quit]);

#[derive(Clone, Copy, PartialEq, Eq)]
enum EventKind {
    Ok,
    Warn,
    Info,
}

struct EventLine {
    id: u64,
    time: String,
    kind: EventKind,
    course: String,
    text: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ActivityStatus {
    Detected,
    Success,
    Failed,
    Gone,
}

struct ActivityRow {
    id: String,
    course: String,
    kind: String,
    time: String,
    detail: String,
    status: ActivityStatus,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Home,
    Courses,
    Stats,
}

enum GuiMessage {
    Core(CoreEvent),
    Courses(Vec<duifene_core::models::Course>),
    MonitorStopped(String),
    SessionCheck(bool),
    LoginName(String),
}

enum LoginMessage {
    Ok,
    Fail(String),
}

type LoginClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

struct MonitorApp {
    monitoring: bool,
    started_at: Option<Instant>,
    sidebar_collapsed: bool,
    events: Vec<EventLine>,
    event_rx: Option<mpsc::Receiver<GuiMessage>>,
    event_tx: Option<mpsc::Sender<GuiMessage>>,
    login_rx: Option<mpsc::Receiver<LoginMessage>>,
    stop_flag: Option<Arc<AtomicBool>>,
    courses_count: usize,
    courses: Vec<duifene_core::models::Course>,
    active_page: Page,
    activities: Vec<ActivityRow>,
    found_count: u32,
    signed_count: u32,
    has_cookie: bool,
    session_valid: bool,
    login_name: String,
    next_event_id: u64,
}

impl EventKind {
    fn icon(self) -> IconName {
        match self {
            EventKind::Ok => IconName::CircleCheck,
            EventKind::Warn => IconName::TriangleAlert,
            EventKind::Info => IconName::Info,
        }
    }

    fn label(self) -> &'static str {
        match self {
            EventKind::Ok => "成功",
            EventKind::Warn => "警告",
            EventKind::Info => "信息",
        }
    }

    fn color(self, cx: &App) -> Hsla {
        match self {
            EventKind::Ok => cx.theme().success,
            EventKind::Warn => cx.theme().warning,
            EventKind::Info => cx.theme().muted_foreground,
        }
    }
}

fn stat_card(
    value: String,
    unit: &'static str,
    label: &'static str,
    cx: &Context<MonitorApp>,
) -> impl IntoElement {
    v_flex()
        .flex_1()
        .px_4()
        .py_4()
        .gap_1()
        .rounded_lg()
        .bg(cx.theme().popover)
        .border_1()
        .border_color(cx.theme().border)
        .child(
            h_flex()
                .items_baseline()
                .gap_1()
                .child(div().text_2xl().font_weight(FontWeight::BOLD).child(value))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(unit),
                ),
        )
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
}

impl MonitorApp {
    fn spawn_monitor(&mut self, cx: &Context<Self>) {
        let Some(tx) = self.event_tx.clone() else {
            return;
        };
        let stop_flag = Arc::new(AtomicBool::new(false));
        let thread_stop = stop_flag.clone();
        self.stop_flag = Some(stop_flag);
        std::thread::spawn(move || {
            let send = |message: GuiMessage| {
                let _ = tx.send(message);
            };
            let stopped = |reason: &str| {
                send(GuiMessage::MonitorStopped(reason.to_string()));
            };
            let config = storage::load_config();
            if config.cookie.is_empty() {
                stopped("未登录");
                return;
            }
            let mut client = LiveClient::from_cookie_string(&config.cookie);
            if let Err(error) = client.check_login() {
                stopped(&format!("登录状态失效: {error}"));
                return;
            }
            let courses = match client.fetch_courses() {
                Ok(courses) => courses,
                Err(error) => {
                    stopped(&format!("获取课程失败: {error}"));
                    return;
                }
            };
            if thread_stop.load(Ordering::Relaxed) {
                return;
            }
            send(GuiMessage::Courses(courses.clone()));
            let engine = Engine::new(
                Box::new(client),
                courses,
                EngineConfig {
                    delay_seconds: 0,
                    coords: storage::course_coordinates(&config),
                    refresh_every: 300,
                },
            );
            let mut runner = Runner::new(engine, std::time::Duration::from_secs(2));
            runner.run_with_stop(
                &mut |event| {
                    let _ = tx.send(GuiMessage::Core(event));
                },
                &thread_stop,
            );
            if !thread_stop.load(Ordering::Relaxed) {
                let _ = tx.send(GuiMessage::MonitorStopped(
                    "监控任务已结束，请检查网络或登录状态".to_string(),
                ));
            }
        });
        let _ = cx;
    }

    fn stop_monitor(&mut self) {
        if let Some(flag) = &self.stop_flag {
            flag.store(true, Ordering::Relaxed);
        }
        self.monitoring = false;
        self.started_at = None;
        self.event_rx = None;
        self.stop_flag = None;
        self.courses_count = 0;
    }

    fn handle_session_lost(&mut self, reason: String) {
        self.session_valid = false;
        self.stop_monitor();
        self.push_row(EventKind::Warn, "会话", format!("登录状态失效: {reason}"));
    }

    fn apply_core_event(&mut self, event: CoreEvent) {
        match event {
            CoreEvent::Info(message) => self.push_row(EventKind::Info, "系统", message),
            CoreEvent::Warn(message) => {
                let lost = message.contains("登录失效");
                self.push_row(EventKind::Warn, "系统", message.clone());
                if lost {
                    self.handle_session_lost(message);
                }
            }
            CoreEvent::Found {
                course_name,
                activity_id,
                kind,
                code,
                ..
            } => {
                self.found_count += 1;
                let detail = match &code {
                    Some(code) => format!("{kind} · {code}"),
                    None => kind.clone(),
                };
                self.activities.insert(
                    0,
                    ActivityRow {
                        id: activity_id.clone(),
                        course: course_name.clone(),
                        kind: kind.clone(),
                        time: Local::now().format("%H:%M:%S").to_string(),
                        detail,
                        status: ActivityStatus::Detected,
                    },
                );
                if self.activities.len() > 10 {
                    self.activities.truncate(10);
                }
            }
            CoreEvent::Signed {
                course_name,
                activity_id,
                result,
                ..
            } => {
                let (status, detail) = match result {
                    CheckInResult::Ok(message) => {
                        self.signed_count += 1;
                        (ActivityStatus::Success, message)
                    }
                    CheckInResult::Gone(message) => (ActivityStatus::Gone, message),
                    CheckInResult::Failed(message) => (ActivityStatus::Failed, message),
                };
                if let Some(row) = self.activities.iter_mut().find(|row| row.id == activity_id) {
                    row.status = status;
                    row.detail = detail;
                } else {
                    self.activities.insert(
                        0,
                        ActivityRow {
                            id: activity_id.clone(),
                            course: course_name.clone(),
                            kind: String::new(),
                            time: Local::now().format("%H:%M:%S").to_string(),
                            detail,
                            status,
                        },
                    );
                }
            }
        }
    }

    fn drain_events(&mut self) {
        while let Some(rx) = self.login_rx.as_ref()
            && let Ok(message) = rx.try_recv()
        {
            match message {
                LoginMessage::Ok => {
                    self.has_cookie = true;
                    self.session_valid = true;
                    self.push_row(EventKind::Ok, "会话", "微信登录成功".to_string());
                    self.login_rx = None;
                    self.fetch_courses_async();
                }
                LoginMessage::Fail(reason) => {
                    self.push_row(EventKind::Warn, "会话", format!("登录失败: {reason}"));
                    self.login_rx = None;
                }
            }
        }
        loop {
            let message = match self.event_rx.as_ref() {
                Some(rx) => match rx.try_recv() {
                    Ok(message) => message,
                    Err(_) => return,
                },
                None => return,
            };
            match message {
                GuiMessage::Core(event) => self.apply_core_event(event),
                GuiMessage::SessionCheck(valid) => {
                    self.session_valid = valid;
                    if valid {
                        self.push_row(EventKind::Ok, "会话", "已恢复登录状态".to_string());
                        self.fetch_courses_async();
                    } else {
                        self.push_row(
                            EventKind::Warn,
                            "会话",
                            "保存的会话已失效,请重新登录".to_string(),
                        );
                    }
                }
                GuiMessage::LoginName(name) => {
                    self.login_name = name;
                }
                GuiMessage::Courses(courses) => {
                    self.courses = courses.clone();
                    self.courses_count = courses.len();
                }
                GuiMessage::MonitorStopped(reason) => {
                    if self.monitoring {
                        self.monitoring = false;
                        self.started_at = None;
                        self.stop_flag = None;
                        self.courses_count = 0;
                        self.push_row(EventKind::Warn, "系统", reason);
                    }
                    return;
                }
            }
        }
    }

    fn fetch_courses_async(&self) {
        let Some(tx) = self.event_tx.clone() else {
            return;
        };
        std::thread::spawn(move || {
            let config = storage::load_config();
            let mut client = LiveClient::from_cookie_string(&config.cookie);
            if let Ok(name) = client.login_name() {
                let _ = tx.send(GuiMessage::LoginName(name));
            }
            match client.fetch_courses() {
                Ok(courses) => {
                    let _ = tx.send(GuiMessage::Courses(courses));
                }
                Err(error) => {
                    let _ = tx.send(GuiMessage::Core(CoreEvent::Warn(format!(
                        "获取课程失败: {error}"
                    ))));
                }
            }
        });
    }

    fn push_row(&mut self, kind: EventKind, course: &str, text: String) {
        let time = Local::now().format("%H:%M:%S").to_string();
        self.events.insert(
            0,
            EventLine {
                id: self.next_event_id,
                time,
                kind,
                course: course.to_string(),
                text,
            },
        );
        self.next_event_id += 1;
        if self.events.len() > 200 {
            self.events.truncate(200);
        }
    }

    fn render_activity_table(&self, cx: &Context<Self>) -> AnyElement {
        let status_label = |row: &ActivityRow| -> (String, Hsla) {
            match row.status {
                ActivityStatus::Detected => ("检测中".to_string(), cx.theme().info),
                ActivityStatus::Success => ("已签".to_string(), cx.theme().success),
                ActivityStatus::Failed => (format!("失败: {}", row.detail), cx.theme().warning),
                ActivityStatus::Gone => ("已结束".to_string(), cx.theme().muted_foreground),
            }
        };
        let head_cell = |label: &'static str, width: Option<f32>, border: bool| {
            let cell = TableHead::new().text_center().when(border, |cell| {
                cell.border_r_1().border_color(cx.theme().border)
            });
            let cell = match width {
                Some(width) => cell.w(px(width)),
                None => cell,
            };
            cell.child(
                div()
                    .text_sm()
                    .font_semibold()
                    .text_color(cx.theme().muted_foreground)
                    .child(label),
            )
        };
        let body_cell = |content: String, width: Option<f32>, border: bool, color: Hsla| {
            let cell = TableCell::new().text_center().when(border, |cell| {
                cell.border_r_1().border_color(cx.theme().border)
            });
            let cell = match width {
                Some(width) => cell.w(px(width)),
                None => cell,
            };
            cell.child(div().text_sm().text_color(color).child(content))
        };
        let body: TableBody = if self.activities.is_empty() {
            TableBody::new().child(
                TableRow::new().child(
                    TableCell::new().col_span(4).child(
                        v_flex()
                            .w_full()
                            .py_8()
                            .items_center()
                            .justify_center()
                            .gap_1p5()
                            .child(
                                Icon::new(IconName::Inbox).text_color(cx.theme().muted_foreground),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("暂无活动记录"),
                            ),
                    ),
                ),
            )
        } else {
            TableBody::new().children(self.activities.iter().map(|row| {
                let (status_text, status_color) = status_label(row);
                TableRow::new()
                    .child(body_cell(
                        row.time.clone(),
                        Some(100.),
                        true,
                        cx.theme().muted_foreground,
                    ))
                    .child(body_cell(
                        if row.kind.is_empty() {
                            "未知".to_string()
                        } else {
                            row.kind.clone()
                        },
                        Some(100.),
                        true,
                        cx.theme().foreground,
                    ))
                    .child(body_cell(
                        row.course.clone(),
                        None,
                        true,
                        cx.theme().foreground,
                    ))
                    .child(body_cell(status_text, Some(100.), false, status_color))
            }))
        };
        Table::new()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .overflow_hidden()
            .child(
                TableHeader::new().child(
                    TableRow::new()
                        .child(head_cell("时间", Some(100.), true))
                        .child(head_cell("类型", Some(100.), true))
                        .child(head_cell("课程", None, true))
                        .child(head_cell("状态", Some(100.), false)),
                ),
            )
            .child(body)
            .into_any_element()
    }

    fn render_event_row(
        &self,
        event: &EventLine,
        is_last: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let kind_color = event.kind.color(cx);
        ListItem::new(ElementId::NamedInteger("event".into(), event.id)).child(
            h_flex()
                .w_full()
                .px_4()
                .py_2p5()
                .gap_3()
                .hover(|style| style.bg(cx.theme().muted))
                .when(!is_last, |row| {
                    row.border_b_1().border_color(cx.theme().border)
                })
                .child(Icon::new(event.kind.icon()).small().text_color(kind_color))
                .child(
                    div()
                        .w(px(72.))
                        .flex_shrink_0()
                        .whitespace_nowrap()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(event.time.clone()),
                )
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(kind_color)
                        .child(event.kind.label()),
                )
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(cx.theme().foreground)
                        .child(event.course.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(event.text.clone()),
                ),
        )
    }

    fn status_label(&self) -> &'static str {
        if !self.has_cookie {
            "未登录"
        } else if !self.session_valid {
            "会话失效"
        } else {
            "已登录"
        }
    }

    fn status_color(&self, cx: &Context<Self>) -> Hsla {
        if !self.has_cookie {
            cx.theme().muted_foreground
        } else if !self.session_valid {
            cx.theme().danger
        } else {
            cx.theme().success
        }
    }

    fn render_sidebar(&self, cx: &Context<Self>) -> impl IntoElement {
        let collapsed = self.sidebar_collapsed;
        let app_toggle = cx.listener(|this, _event: &ClickEvent, _window, cx| {
            this.sidebar_collapsed = !this.sidebar_collapsed;
            cx.notify();
        });

        let page_listener = |target: Page| {
            cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                this.active_page = target;
                cx.notify();
            })
        };
        let menu = SidebarMenu::new()
            .child(
                SidebarMenuItem::new("首页")
                    .icon(Icon::new(IconName::LayoutDashboard))
                    .active(self.active_page == Page::Home)
                    .collapsed(collapsed)
                    .on_click(page_listener(Page::Home)),
            )
            .child(
                SidebarMenuItem::new("课程")
                    .icon(Icon::new(IconName::BookOpen))
                    .active(self.active_page == Page::Courses)
                    .collapsed(collapsed)
                    .on_click(page_listener(Page::Courses)),
            )
            .child(
                SidebarMenuItem::new("统计")
                    .icon(Icon::new(IconName::ChartPie))
                    .active(self.active_page == Page::Stats)
                    .collapsed(collapsed)
                    .on_click(page_listener(Page::Stats)),
            );

        let logout = {
            let app_weak = cx.weak_entity();
            move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                let _ = storage::save_cookie("");
                app_weak
                    .update(cx, |app, cx| {
                        app.has_cookie = false;
                        app.session_valid = false;
                        app.login_name = String::new();
                        app.courses.clear();
                        app.courses_count = 0;
                        app.activities.clear();
                        app.events.clear();
                        app.found_count = 0;
                        app.signed_count = 0;
                        app.push_row(EventKind::Info, "会话", "已退出登录".to_string());
                        cx.notify();
                    })
                    .ok();
            }
        };

        let open_login: LoginClickHandler = Rc::new({
            let app_weak = cx.weak_entity();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                let (tx, rx) = mpsc::channel::<LoginMessage>();
                app_weak
                    .update(cx, |app, cx| {
                        app.login_rx = Some(rx);
                        cx.notify();
                    })
                    .ok();
                *LOGIN_TX.lock().unwrap() = Some(tx);
                let input =
                    cx.new(|cx| InputState::new(window, cx).placeholder("粘贴微信授权链接"));
                window.open_dialog(cx, move |dialog, _, _| {
                    let input = input.clone();
                    dialog.w(px(420.)).content(move |content, _, cx| {
                        let input_footer = input.clone();
                        content
                            .child(
                                DialogHeader::new()
                                    .child(DialogTitle::new().child("微信登录"))
                                    .child(DialogDescription::new().child(
                                        "在微信中打开授权链接,复制完整认证链接粘贴至下方输入框中",
                                    )),
                            )
                            .child(
                                v_flex()
                                    .px_4()
                                    .pb_4()
                                    .gap_3()
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .px_3()
                                                    .py_2()
                                                    .rounded_md()
                                                    .bg(cx.theme().muted)
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(
                                                        div()
                                                            .whitespace_nowrap()
                                                            .text_ellipsis()
                                                            .child(WECHAT_AUTH_URL),
                                                    ),
                                            )
                                            .child(
                                                Clipboard::new("copy-auth-link")
                                                    .value(WECHAT_AUTH_URL)
                                                    .tooltip("复制授权链接"),
                                            ),
                                    )
                                    .child(Input::new(&input).h(px(36.)))
                                    .child(
                                        DialogFooter::new().justify_end().child(
                                            Button::new("login-submit")
                                                .primary()
                                                .small()
                                                .label("登录")
                                                .on_click({
                                                    let input_footer = input_footer.clone();
                                                    move |_, window, cx| {
                                                        let link = input_footer
                                                            .read(cx)
                                                            .value()
                                                            .to_string();
                                                        window.close_dialog(cx);
                                                        let sender =
                                                            LOGIN_TX.lock().unwrap().clone();
                                                        std::thread::spawn(move || {
                                                            let mut client = LiveClient::new();
                                                            let result = client
                                                                .login_wechat(&link)
                                                                .map_err(|error| error.to_string())
                                                                .and_then(|_| {
                                                                    storage::save_cookie(
                                                                        &client.cookie_string(),
                                                                    )
                                                                    .map_err(|error| {
                                                                        error.to_string()
                                                                    })
                                                                });
                                                            match result {
                                                                Ok(()) => {
                                                                    if let Some(sender) = sender {
                                                                        let _ = sender
                                                                            .send(LoginMessage::Ok);
                                                                    }
                                                                }
                                                                Err(error) => {
                                                                    if let Some(sender) = sender {
                                                                        let _ = sender.send(
                                                                            LoginMessage::Fail(
                                                                                error,
                                                                            ),
                                                                        );
                                                                    }
                                                                }
                                                            }
                                                        });
                                                    }
                                                }),
                                        ),
                                    ),
                            )
                    })
                });
            }
        });

        let header = h_flex()
            .w_full()
            .py_2()
            .map(|header| {
                if collapsed {
                    header.justify_center()
                } else {
                    header.px_2().gap_2p5()
                }
            })
            .child(
                div()
                    .id("app-logo")
                    .size_7()
                    .rounded_lg()
                    .bg(cx.theme().primary)
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|style| style.bg(cx.theme().primary_hover))
                    .on_click(app_toggle)
                    .flex()
                    .child(
                        svg()
                            .path("icons/pen.svg")
                            .size(px(14.))
                            .text_color(cx.theme().primary_foreground),
                    ),
            )
            .when(!collapsed, |header| {
                header.child(
                    div()
                        .text_base()
                        .font_weight(FontWeight::BOLD)
                        .child("duifene-auto"),
                )
            });

        let footer = SidebarFooter::new().child(
            v_flex()
                .w_full()
                .gap_1()
                .when(!collapsed, |footer| {
                    footer
                        .child(
                            h_flex()
                                .w_full()
                                .px_3()
                                .py_2p5()
                                .gap_2p5()
                                .rounded_lg()
                                .border_1()
                                .border_color(cx.theme().border)
                                .bg(cx.theme().popover)
                                .child({
                                    let open_login = open_login.clone();
                                    div()
                                        .id("account-avatar")
                                        .size_7()
                                        .flex_shrink_0()
                                        .rounded_full()
                                        .bg(cx.theme().primary)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .cursor_pointer()
                                        .hover(|style| style.bg(cx.theme().primary_hover))
                                        .on_click(move |event, window, cx| {
                                            open_login(event, window, cx);
                                        })
                                        .child(
                                            Icon::new(IconName::CircleUser)
                                                .small()
                                                .text_color(cx.theme().primary_foreground),
                                        )
                                })
                                .child(
                                    v_flex()
                                        .child(
                                            div().text_sm().font_weight(FontWeight::MEDIUM).child(
                                                if self.login_name.is_empty() {
                                                    "账号".to_string()
                                                } else {
                                                    self.login_name.clone()
                                                },
                                            ),
                                        )
                                        .child(
                                            h_flex()
                                                .items_center()
                                                .gap_1()
                                                .child(
                                                    div()
                                                        .size_1p5()
                                                        .rounded_full()
                                                        .bg(self.status_color(cx)),
                                                )
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(cx.theme().muted_foreground)
                                                        .child(self.status_label()),
                                                ),
                                        ),
                                ),
                        )
                        .child(match self.has_cookie {
                            true => Button::new("logout-btn")
                                .outline()
                                .small()
                                .w_full()
                                .label("退出")
                                .on_click(logout)
                                .into_any_element(),
                            false => Button::new("open-login")
                                .outline()
                                .small()
                                .w_full()
                                .label("登录")
                                .on_click({
                                    let open_login = open_login.clone();
                                    move |event, window, cx| {
                                        open_login(event, window, cx);
                                    }
                                })
                                .into_any_element(),
                        })
                })
                .when(collapsed, |footer| {
                    footer.child(h_flex().w_full().justify_center().child({
                        let open_login = open_login.clone();
                        div()
                            .id("account-avatar")
                            .size_7()
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(cx.theme().primary)
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|style| style.bg(cx.theme().primary_hover))
                            .on_click(move |event, window, cx| {
                                open_login(event, window, cx);
                            })
                            .child(
                                Icon::new(IconName::CircleUser)
                                    .small()
                                    .text_color(cx.theme().primary_foreground),
                            )
                    }))
                }),
        );

        Sidebar::new("main-sidebar")
            .collapsible(SidebarCollapsible::Icon)
            .collapsed(collapsed)
            .header(header)
            .child(menu)
            .footer(footer)
    }
}

impl Render for MonitorApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dialog_layer = Root::render_dialog_layer(window, cx);
        div()
            .size_full()
            .child(
                h_flex()
                    .size_full()
                    .bg(cx.theme().background)
                    .text_color(cx.theme().foreground)
                    .child(self.render_sidebar(cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .relative()
                            .child(TitleBar::new().child(div()))
                            .child(
                                div()
                                    .id("main-scroll")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .px_8()
                                    .pb_24()
                                    .child(v_flex().gap_4().child(match self.active_page {
                                        Page::Home => self.render_home(cx).into_any_element(),
                                        Page::Courses => self.render_courses(cx).into_any_element(),
                                        Page::Stats => self.render_stats(cx).into_any_element(),
                                    })),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .right_0()
                                    .bottom_8()
                                    .flex()
                                    .justify_center()
                                    .child(self.render_monitor_button(cx)),
                            ),
                    ),
            )
            .children(dialog_layer)
    }
}

impl MonitorApp {
    fn render_home(&self, cx: &Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_4()
            .child(
                h_flex()
                    .w_full()
                    .gap_4()
                    .child(stat_card(
                        self.courses_count.to_string(),
                        "门",
                        "监控课程",
                        cx,
                    ))
                    .child(stat_card(
                        self.signed_count.to_string(),
                        "次",
                        "今日签到成功",
                        cx,
                    ))
                    .child(stat_card(
                        self.found_count.to_string(),
                        "次",
                        "今日发现签到",
                        cx,
                    )),
            )
            .child(
                div()
                    .pt_2()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(cx.theme().muted_foreground)
                    .child("活动状态"),
            )
            .child(self.render_activity_table(cx))
            .child(
                div()
                    .pt_2()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(cx.theme().muted_foreground)
                    .child("系统消息"),
            )
            .child(
                v_flex()
                    .rounded_lg()
                    .bg(cx.theme().popover)
                    .border_1()
                    .border_color(cx.theme().border)
                    .overflow_hidden()
                    .when(self.events.is_empty(), |card| {
                        card.py_8().flex().items_center().justify_center().child(
                            v_flex()
                                .items_center()
                                .gap_1p5()
                                .child(
                                    Icon::new(IconName::Inbox)
                                        .text_color(cx.theme().muted_foreground),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("暂无系统消息"),
                                ),
                        )
                    })
                    .children(self.events.iter().enumerate().map(|(index, event)| {
                        self.render_event_row(event, index + 1 == self.events.len(), cx)
                    })),
            )
    }

    fn render_courses(&self, cx: &Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_4()
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("本学期课程 · {} 门", self.courses.len())),
            )
            .child(
                v_flex()
                    .rounded_lg()
                    .bg(cx.theme().popover)
                    .border_1()
                    .border_color(cx.theme().border)
                    .overflow_hidden()
                    .children(self.courses.iter().enumerate().map(|(index, course)| {
                        ListItem::new(ElementId::NamedInteger("course".into(), index as u64)).child(
                            h_flex()
                                .w_full()
                                .px_4()
                                .py_2p5()
                                .gap_3()
                                .child(
                                    div()
                                        .w(px(28.))
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!("{}", index + 1)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .text_sm()
                                        .text_color(cx.theme().foreground)
                                        .child(course.name.clone()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!("课堂ID: {}", course.tclass_id)),
                                ),
                        )
                    })),
            )
    }

    fn render_stats(&self, cx: &Context<Self>) -> impl IntoElement {
        use gpui_kit::component::chart::{BarChart, PieChart};
        use std::collections::BTreeMap;

        // 三个图表都只统计签到记录(activities)。
        let mut kind_counts: BTreeMap<String, (u32, Hsla)> = BTreeMap::new();
        let mut result_counts: BTreeMap<String, u32> = BTreeMap::new();
        let mut course_counts: BTreeMap<String, u32> = BTreeMap::new();
        for activity in &self.activities {
            *course_counts.entry(activity.course.clone()).or_insert(0) += 1;
            let kind_color = match activity.kind.as_str() {
                "数字码" => cx.theme().info,
                "二维码" => cx.theme().success,
                "定位" => cx.theme().warning,
                _ => cx.theme().muted_foreground,
            };
            let kind = if activity.kind.is_empty() {
                "未知".to_string()
            } else {
                activity.kind.clone()
            };
            let slot = kind_counts.entry(kind).or_insert((0, kind_color));
            slot.0 += 1;
            let result = match activity.status {
                ActivityStatus::Success => Some("成功"),
                ActivityStatus::Failed | ActivityStatus::Gone => Some("失败"),
                ActivityStatus::Detected => None,
            };
            if let Some(result) = result {
                *result_counts.entry(result.to_string()).or_insert(0) += 1;
            }
        }

        let kind_data: Vec<(String, u32, Hsla)> = kind_counts
            .into_iter()
            .map(|(kind, (count, color))| (kind, count, color))
            .collect();
        let result_data: Vec<(String, u32)> = result_counts.into_iter().collect();
        let course_colors = [
            cx.theme().chart_1,
            cx.theme().chart_2,
            cx.theme().chart_3,
            cx.theme().chart_4,
            cx.theme().chart_5,
        ];
        let course_data: Vec<(String, u32, Hsla)> = course_counts
            .into_iter()
            .enumerate()
            .map(|(index, (course, count))| {
                (course, count, course_colors[index % course_colors.len()])
            })
            .collect();

        let card = |title: &'static str, chart: AnyElement| -> AnyElement {
            v_flex()
                .flex_1()
                .min_w_0()
                .h(px(320.))
                .p_4()
                .rounded_lg()
                .bg(cx.theme().popover)
                .border_1()
                .border_color(cx.theme().border)
                .child(div().font_semibold().text_sm().child(title))
                .child(div().flex_1().min_h_0().py_4().child(chart))
                .into_any_element()
        };
        let full_card = |title: &'static str, chart: AnyElement| -> AnyElement {
            v_flex()
                .w_full()
                .h(px(320.))
                .p_4()
                .rounded_lg()
                .bg(cx.theme().popover)
                .border_1()
                .border_color(cx.theme().border)
                .child(div().font_semibold().text_sm().child(title))
                .child(div().flex_1().min_h_0().py_4().child(chart))
                .into_any_element()
        };

        v_flex()
            .gap_4()
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("统计"),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_4()
                    .child(card(
                        "事件类型分布",
                        PieChart::new(kind_data)
                            .inner_radius(60.)
                            .outer_radius(100.)
                            .value(|(_, count, _)| *count as f32)
                            .color(|(_, _, color)| *color)
                            .label(|(kind, _, _)| kind.clone().into())
                            .into_any_element(),
                    ))
                    .child(card(
                        "事件结果",
                        BarChart::new(result_data)
                            .band(|(name, _)| name.clone())
                            .value(|(_, count)| *count as f64)
                            .label(|(_, count)| count.to_string())
                            .into_any_element(),
                    )),
            )
            .child(full_card(
                "各课程事件数",
                BarChart::new(course_data)
                    .band(|(name, _, _)| name.clone())
                    .value(|(_, count, _)| *count as f64)
                    .fill(|(_, _, color), _, _, _| *color)
                    .label(|(_, count, _)| count.to_string())
                    .into_any_element(),
            ))
    }

    fn render_monitor_button(&self, cx: &Context<Self>) -> AnyElement {
        let toggle = cx.listener(|this, _event: &ClickEvent, _window, cx| {
            if this.monitoring {
                this.stop_monitor();
            } else {
                this.monitoring = true;
                this.started_at = Some(Instant::now());
                this.session_valid = this.has_cookie;
                this.spawn_monitor(cx);
            }
            cx.notify();
        });
        if self.monitoring {
            let elapsed = self
                .started_at
                .map(|started| format_elapsed(started.elapsed()))
                .unwrap_or_else(|| "00:00:00".into());
            h_flex()
                .id("toggle")
                .h(px(56.))
                .px_6()
                .gap_2p5()
                .rounded_full()
                .bg(cx.theme().primary)
                .shadow(vec![monitor_button_shadow()])
                .hover(|style| style.bg(cx.theme().primary_hover))
                .active(|style| style.bg(cx.theme().primary_active))
                .cursor_pointer()
                .on_click(toggle)
                .child(div().size_2().rounded_full().bg(cx.theme().success))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(cx.theme().primary_foreground)
                        .child(elapsed),
                )
                .child(
                    Icon::new(IconName::Pause)
                        .small()
                        .text_color(cx.theme().primary_foreground),
                )
                .into_any_element()
        } else {
            div()
                .id("toggle")
                .size(px(56.))
                .rounded_full()
                .bg(cx.theme().primary)
                .shadow(vec![monitor_button_shadow()])
                .hover(|style| style.bg(cx.theme().primary_hover))
                .active(|style| style.bg(cx.theme().primary_active))
                .cursor_pointer()
                .on_click(toggle)
                .flex()
                .items_center()
                .justify_center()
                .child(Icon::new(IconName::Play).text_color(cx.theme().primary_foreground))
                .into_any_element()
        }
    }
}

fn monitor_button_shadow() -> BoxShadow {
    BoxShadow {
        color: Hsla {
            h: 40.,
            s: 0.12,
            l: 0.2,
            a: 0.18,
        },
        offset: point(px(0.), px(4.)),
        blur_radius: px(14.),
        spread_radius: px(0.),
        inset: false,
    }
}

fn format_elapsed(elapsed: std::time::Duration) -> String {
    let total_seconds = elapsed.as_secs();
    format!(
        "{:02}:{:02}:{:02}",
        total_seconds / 3600,
        (total_seconds % 3600) / 60,
        total_seconds % 60
    )
}

fn application_icon() -> Option<Arc<image::RgbaImage>> {
    #[cfg(target_os = "linux")]
    {
        let image = image::load_from_memory(include_bytes!(env!("DUIFENE_AUTO_ICON_PNG")))
            .expect("failed to decode generated application icon")
            .to_rgba8();
        Some(Arc::new(image))
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

fn main() {
    let app = gpui_kit::application().with_assets(AppAssets);
    app.run(move |cx| {
        gpui_kit::init(cx);
        cx.set_app_identity("com.linysu.duifene-auto", "duifene-auto");
        theme::apply_paper_theme(cx);

        cx.bind_keys([KeyBinding::new("alt-q", Quit, None)]);
        cx.on_action(|_: &Quit, _cx: &mut App| {
            std::process::exit(0);
        });

        cx.spawn(async move |cx| {
            let handle = cx
                .open_window(
                    WindowOptions {
                        titlebar: Some(TitleBar::title_bar_options()),
                        window_min_size: Some(Size {
                            width: px(860.),
                            height: px(560.),
                        }),
                        icon: application_icon(),
                        ..Default::default()
                    },
                    |window, cx| {
                        let view = cx.new(|cx| {
                            let (event_tx, event_rx) = mpsc::channel::<GuiMessage>();
                            let app = MonitorApp {
                                monitoring: false,
                                started_at: None,
                                sidebar_collapsed: false,
                                events: Vec::new(),
                                event_rx: Some(event_rx),
                                event_tx: Some(event_tx.clone()),
                                login_rx: None,
                                stop_flag: None,
                                courses_count: 0,
                                courses: Vec::new(),
                                active_page: Page::Home,
                                activities: Vec::new(),
                                found_count: 0,
                                signed_count: 0,
                                has_cookie: !storage::load_config().cookie.is_empty(),
                                session_valid: false,
                                login_name: String::new(),
                                next_event_id: 1,
                            };
                            {
                                let tx = event_tx.clone();
                                std::thread::spawn(move || {
                                    let config = storage::load_config();
                                    if config.cookie.is_empty() {
                                        return;
                                    }
                                    let mut client = LiveClient::from_cookie_string(&config.cookie);
                                    let valid = client.check_login().is_ok();
                                    let _ = tx.send(GuiMessage::SessionCheck(valid));
                                });
                            }
                            cx.spawn(async move |this, cx| {
                                loop {
                                    cx.background_executor()
                                        .timer(std::time::Duration::from_millis(200))
                                        .await;
                                    if this
                                        .update(
                                            cx,
                                            |app: &mut MonitorApp, cx: &mut Context<MonitorApp>| {
                                                app.drain_events();
                                                cx.notify();
                                            },
                                        )
                                        .is_err()
                                    {
                                        break;
                                    }
                                }
                            })
                            .detach();
                            app
                        });
                        cx.new(|cx| Root::new(view, window, cx))
                    },
                )
                .expect("failed to open window");
            *MAIN_WINDOW.lock().unwrap() = Some(handle);
        })
        .detach();
    });
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::Root;
    use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext};

    use crate::{MonitorApp, Page};

    #[test]
    fn arc_fill_path_probe() {
        use std::f32::consts::PI;

        use gpui_kit::{PathBuilder, point, px};

        let center_x = 200.0_f32;
        let center_y = 100.0_f32;
        let r1 = 72.0_f32;
        let start_angle = 0.0_f32 - PI / 2.;
        let end_angle = PI - PI / 2.;
        let pad = 0.0001 * 0.5;
        let a0_outer = start_angle + pad;
        let a1_outer = end_angle - pad;
        let x01 = center_x + r1 * a0_outer.cos();
        let y01 = center_y + r1 * a0_outer.sin();
        let x11 = center_x + r1 * a1_outer.cos();
        let y11 = center_y + r1 * a1_outer.sin();

        let mut builder = PathBuilder::fill();
        builder.move_to(point(px(x01), px(y01)));
        builder.arc_to(
            point(px(r1), px(r1)),
            px(0.),
            false,
            true,
            point(px(x11), px(y11)),
        );
        builder.line_to(point(px(center_x), px(center_y)));
        match builder.build() {
            Ok(_) => {}
            Err(e) => panic!("arc fill path failed to build: {e:?}"),
        }
    }

    fn make_app(
        _window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::App,
    ) -> gpui_kit::Entity<MonitorApp> {
        cx.new(|_| MonitorApp {
            monitoring: false,
            started_at: None,
            sidebar_collapsed: false,
            events: Vec::new(),
            event_rx: None,
            event_tx: None,
            login_rx: None,
            stop_flag: None,
            courses_count: 0,
            courses: Vec::new(),
            active_page: Page::Home,
            activities: Vec::new(),
            found_count: 0,
            signed_count: 0,
            has_cookie: false,
            session_valid: false,
            login_name: String::new(),
            next_event_id: 1,
        })
    }

    #[gpui_kit::test]
    fn app_renders_in_window(cx: &mut TestAppContext) {
        cx.update(|cx| gpui_kit::init(cx));
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |window, cx| {
                let view = make_app(window, cx);
                cx.new(|cx| Root::new(view, window, cx))
            })
            .unwrap()
        });
        let mut test_cx = VisualTestContext::from_window(window.into(), cx);
        let _ = window.root(&mut test_cx).unwrap();
    }
}
