//! GTK4 + libadwaita configurator UI.
//!
//! The business logic (loading, mutating, validating, persisting the
//! config and talking to the daemon over D-Bus) lives in the
//! `xxkb-config-state` crate. This file is just the GTK widget layer:
//! every widget's "value-changed" signal calls a setter on
//! [`ConfigEditor`]; failures bubble up as toasts in the
//! [`adw::ToastOverlay`].
//!
//! Pages (driven by `AdwViewSwitcher`):
//! 1. **General** — two-state, base/alt group, cycle modifier, ignore_reverse.
//! 2. **Main indicator** — enable, mode, size, border, per-display positions.
//! 3. **Per-window indicator** — enable, size, border, offset.
//! 4. **Icons** — prefer SVG, search paths, group→icon mapping.
//! 5. **Sound** — mode + file picker.
//! 6. **App rules** — table editor with "Capture from active window".

use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use anyhow::Result;
use gtk4::{Button, Orientation};
use xxkb_config::{BorderConfig, Config, MainIndicatorMode, ModifierName, SoundMode};
use xxkb_config_state::{ConfigEditor, ValidationError};
use xxkb_core::{
    rules::{AppMatch, AppRule, RuleAction},
    Offset,
};

const APP_ID: &str = "org.xxkb.Configurator";

/// Editor shared between widgets. Single-threaded — every signal
/// handler runs on the GTK main loop, so `Rc<RefCell<…>>` is enough.
type SharedEditor = Rc<RefCell<ConfigEditor>>;

/// Run the GTK4 app.
pub fn run() -> Result<()> {
    let app = adw::Application::builder().application_id(APP_ID).build();

    app.connect_activate(|app| {
        let editor = match ConfigEditor::load_default() {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(error = %e, "failed to load config; starting with defaults");
                let path = xxkb_config::config_path()
                    .unwrap_or_else(|_| std::path::PathBuf::from("/tmp/xxkb-config.toml"));
                ConfigEditor::from_parts(Config::default(), path)
            }
        };
        build_window(app, Rc::new(RefCell::new(editor)));
    });

    app.run();
    Ok(())
}

fn build_window(app: &adw::Application, editor: SharedEditor) {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("xxkb settings")
        .default_width(900)
        .default_height(640)
        .build();

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar.add_top_bar(&header);

    let toast_overlay = adw::ToastOverlay::new();

    let stack = adw::ViewStack::new();
    stack.add_titled_with_icon(
        &general_page(&editor, &toast_overlay),
        Some("general"),
        "General",
        "preferences-system-symbolic",
    );
    stack.add_titled_with_icon(
        &main_indicator_page(&editor, &toast_overlay),
        Some("main"),
        "Main indicator",
        "video-display-symbolic",
    );
    stack.add_titled_with_icon(
        &per_window_page(&editor, &toast_overlay),
        Some("perwin"),
        "Window indicator",
        "window-symbolic",
    );
    stack.add_titled_with_icon(
        &icons_page(&editor, &toast_overlay),
        Some("icons"),
        "Icons",
        "image-symbolic",
    );
    stack.add_titled_with_icon(
        &sound_page(&editor, &toast_overlay),
        Some("sound"),
        "Sound",
        "audio-speakers-symbolic",
    );
    stack.add_titled_with_icon(
        &rules_page(&editor, &toast_overlay),
        Some("rules"),
        "Rules",
        "format-justify-left-symbolic",
    );

    let switcher_bar = adw::ViewSwitcherBar::new();
    switcher_bar.set_stack(Some(&stack));
    switcher_bar.set_reveal(true);

    let switcher = adw::ViewSwitcher::new();
    switcher.set_stack(Some(&stack));
    switcher.set_policy(adw::ViewSwitcherPolicy::Wide);
    header.set_title_widget(Some(&switcher));

    let layout_box = gtk4::Box::new(Orientation::Vertical, 0);
    layout_box.append(&stack);
    layout_box.append(&switcher_bar);

    toast_overlay.set_child(Some(&layout_box));
    toolbar.set_content(Some(&toast_overlay));

    let save_button = Button::with_label("Save");
    save_button.add_css_class("suggested-action");
    {
        let editor = editor.clone();
        let toast = toast_overlay.clone();
        save_button.connect_clicked(move |_| {
            match editor.borrow_mut().save() {
                Ok(()) => {
                    toast.add_toast(adw::Toast::new("Saved."));
                    // Best-effort kick the running daemon. We do this on
                    // a dedicated worker thread so the GTK loop stays
                    // responsive even if D-Bus is slow to answer.
                    std::thread::spawn(|| {
                        if let Err(e) = xxkb_config_state::dbus_client::blocking::ping_reload() {
                            tracing::warn!(error = %e, "daemon ping failed");
                        }
                    });
                }
                Err(e) => {
                    toast.add_toast(adw::Toast::new(&format!("Save failed: {e}")));
                }
            }
        });
    }
    header.pack_end(&save_button);

    {
        let (tx, rx) = std::sync::mpsc::channel::<u32>();
        let toast_poll = toast_overlay.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(120), move || {
            for count in rx.try_iter() {
                let msg = if count == 1 {
                    "Daemon saved a main-indicator position (Ctrl+drag or D-Bus).".to_string()
                } else {
                    format!("Daemon saved {count} main-indicator positions.")
                };
                toast_poll.add_toast(adw::Toast::new(&msg));
            }
            glib::ControlFlow::Continue
        });
        xxkb_config_state::dbus_client::spawn_positions_saved_listener(move |count| {
            let _ = tx.send(count);
        });
    }

    window.set_content(Some(&toolbar));
    window.present();
}

// ---------------------------------------------------------------------
// Page builders
// ---------------------------------------------------------------------

fn general_page(editor: &SharedEditor, toast: &adw::ToastOverlay) -> adw::PreferencesPage {
    let cfg = editor.borrow().config().clone();
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    group.set_title("Cycle behaviour");

    group.add(&switch_row(
        "Two-state mode",
        Some("Cycle only between base and alt groups"),
        cfg.general.two_state,
        {
            let editor = editor.clone();
            move |b| editor.borrow_mut().set_two_state(b)
        },
    ));

    group.add(&spin_row_validated(
        "Base group",
        (1.0, 4.0, 1.0),
        f64::from(cfg.general.base_group),
        toast,
        {
            let editor = editor.clone();
            move |v| editor.borrow_mut().set_base_group(v as u8)
        },
    ));

    group.add(&spin_row_validated(
        "Alt group",
        (1.0, 4.0, 1.0),
        f64::from(cfg.general.alt_group),
        toast,
        {
            let editor = editor.clone();
            move |v| editor.borrow_mut().set_alt_group(v as u8)
        },
    ));

    let modifier_labels = [
        "None", "Shift", "Lock", "Ctrl", "Alt", "Mod1", "Mod2", "Mod3", "Mod4", "Mod5",
    ];
    let modifier_idx = match cfg.general.cycle_modifier {
        ModifierName::None => 0,
        ModifierName::Shift => 1,
        ModifierName::Lock => 2,
        ModifierName::Ctrl => 3,
        ModifierName::Alt => 4,
        ModifierName::Mod1 => 5,
        ModifierName::Mod2 => 6,
        ModifierName::Mod3 => 7,
        ModifierName::Mod4 => 8,
        ModifierName::Mod5 => 9,
    };
    group.add(&combo_row(
        "Cycle modifier",
        &modifier_labels,
        modifier_idx,
        {
            let editor = editor.clone();
            move |i| {
                let m = match i {
                    1 => ModifierName::Shift,
                    2 => ModifierName::Lock,
                    3 => ModifierName::Ctrl,
                    4 => ModifierName::Alt,
                    5 => ModifierName::Mod1,
                    6 => ModifierName::Mod2,
                    7 => ModifierName::Mod3,
                    8 => ModifierName::Mod4,
                    9 => ModifierName::Mod5,
                    _ => ModifierName::None,
                };
                editor.borrow_mut().set_cycle_modifier(m);
            }
        },
    ));

    group.add(&switch_row(
        "Invert ignore_reverse semantics",
        Some("If on, only matching windows are managed"),
        cfg.general.ignore_reverse,
        {
            let editor = editor.clone();
            move |b| editor.borrow_mut().set_ignore_reverse(b)
        },
    ));

    page.add(&group);
    page
}

fn main_indicator_page(editor: &SharedEditor, toast: &adw::ToastOverlay) -> adw::PreferencesPage {
    let cfg = editor.borrow().config().clone();
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    group.set_title("Display indicator");

    group.add(&switch_row(
        "Show indicator",
        None,
        cfg.main_indicator.enable,
        {
            let editor = editor.clone();
            move |b| editor.borrow_mut().set_main_enable(b)
        },
    ));

    group.add(&combo_row(
        "Display mode",
        &["Primary monitor only", "All displays"],
        match cfg.main_indicator.mode {
            MainIndicatorMode::PrimaryOnly => 0,
            MainIndicatorMode::AllDisplays => 1,
        },
        {
            let editor = editor.clone();
            move |i| {
                let mode = if i == 0 {
                    MainIndicatorMode::PrimaryOnly
                } else {
                    MainIndicatorMode::AllDisplays
                };
                editor.borrow_mut().set_main_mode(mode);
            }
        },
    ));

    group.add(&switch_row(
        "Confirm before saving drag",
        Some(
            "Opens a short dialog (zenity or kdialog) after Ctrl-drag; declining restores the previous position",
        ),
        cfg.main_indicator.confirm_drag_save,
        {
            let editor = editor.clone();
            move |b| editor.borrow_mut().set_main_confirm_drag_save(b)
        },
    ));

    group.add(&spin_row_validated(
        "Size (px)",
        (8.0, 256.0, 1.0),
        f64::from(cfg.main_indicator.size_px),
        toast,
        {
            let editor = editor.clone();
            move |v| editor.borrow_mut().set_main_size(v as u32)
        },
    ));

    group.add(&border_block(
        "Border",
        cfg.main_indicator.border.clone(),
        toast,
        {
            let editor = editor.clone();
            move |b| editor.borrow_mut().set_main_border(b)
        },
    ));

    page.add(&group);

    let positions_group = adw::PreferencesGroup::new();
    positions_group.set_title("Saved positions");
    positions_group.set_description(Some(
        "Ctrl-drag updates the live indicator; the daemon writes ~/.config/xxkb/config.toml \
         (optional confirmation dialog if enabled below). If xxkb-config is open, a toast fires \
         when the daemon emits PositionsSaved.",
    ));
    if cfg.main_indicator.positions.is_empty() {
        let row = adw::ActionRow::new();
        row.set_title("No saved positions yet");
        row.set_subtitle("Move the indicator to record one");
        positions_group.add(&row);
    } else {
        for (output, point) in &cfg.main_indicator.positions {
            let row = adw::ActionRow::new();
            row.set_title(&output.0);
            row.set_subtitle(&format!("({}, {})", point.x, point.y));
            let forget = Button::from_icon_name("user-trash-symbolic");
            forget.set_tooltip_text(Some("Forget this position"));
            forget.add_css_class("flat");
            {
                let editor = editor.clone();
                let toast = toast.clone();
                let output = output.clone();
                let row_widget = row.clone();
                let group_widget = positions_group.clone();
                forget.connect_clicked(move |_| {
                    if editor.borrow_mut().forget_main_position(&output) {
                        group_widget.remove(&row_widget);
                        toast.add_toast(adw::Toast::new(&format!(
                            "Forgot position for {}",
                            output.0
                        )));
                    }
                });
            }
            row.add_suffix(&forget);
            positions_group.add(&row);
        }
    }
    page.add(&positions_group);

    page
}

fn per_window_page(editor: &SharedEditor, toast: &adw::ToastOverlay) -> adw::PreferencesPage {
    let cfg = editor.borrow().config().clone();
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    group.set_title("Window indicator");

    group.add(&switch_row(
        "Draw on title bars",
        None,
        cfg.per_window_indicator.enable,
        {
            let editor = editor.clone();
            move |b| editor.borrow_mut().set_per_window_enable(b)
        },
    ));

    group.add(&spin_row_validated(
        "Size (px)",
        (6.0, 64.0, 1.0),
        f64::from(cfg.per_window_indicator.size_px),
        toast,
        {
            let editor = editor.clone();
            move |v| editor.borrow_mut().set_per_window_size(v as u32)
        },
    ));

    let offset_x = adw::SpinRow::with_range(-512.0, 512.0, 1.0);
    offset_x.set_title("Offset X");
    offset_x.set_subtitle("Negative = from right edge of title bar");
    offset_x.set_value(f64::from(cfg.per_window_indicator.offset.x));
    let offset_y = adw::SpinRow::with_range(-128.0, 128.0, 1.0);
    offset_y.set_title("Offset Y");
    offset_y.set_subtitle("From top of title bar");
    offset_y.set_value(f64::from(cfg.per_window_indicator.offset.y));
    {
        let editor = editor.clone();
        let oy = offset_y.clone();
        offset_x.connect_value_notify(move |row| {
            let off = Offset {
                x: row.value() as i32,
                y: oy.value() as i32,
            };
            editor.borrow_mut().set_per_window_offset(off);
        });
    }
    {
        let editor = editor.clone();
        let ox = offset_x.clone();
        offset_y.connect_value_notify(move |row| {
            let off = Offset {
                x: ox.value() as i32,
                y: row.value() as i32,
            };
            editor.borrow_mut().set_per_window_offset(off);
        });
    }
    group.add(&offset_x);
    group.add(&offset_y);

    group.add(&border_block(
        "Border",
        cfg.per_window_indicator.border.clone(),
        toast,
        {
            let editor = editor.clone();
            move |b| editor.borrow_mut().set_per_window_border(b)
        },
    ));

    page.add(&group);
    page
}

fn icons_page(editor: &SharedEditor, _toast: &adw::ToastOverlay) -> adw::PreferencesPage {
    let cfg = editor.borrow().config().clone();
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    group.set_title("Icons");

    group.add(&switch_row(
        "Prefer SVG over raster",
        None,
        cfg.icons.prefer_svg,
        {
            let editor = editor.clone();
            move |b| editor.borrow_mut().set_prefer_svg(b)
        },
    ));

    let paths_row = adw::EntryRow::new();
    paths_row.set_title("Search paths (comma-separated)");
    paths_row.set_text(&cfg.icons.search_paths.join(", "));
    {
        let editor = editor.clone();
        paths_row.connect_changed(move |row| {
            let paths: Vec<String> = row
                .text()
                .as_str()
                .split(',')
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect();
            editor.borrow_mut().set_search_paths(paths);
        });
    }
    group.add(&paths_row);

    page.add(&group);

    let mapping_group = adw::PreferencesGroup::new();
    mapping_group.set_title("Group → icon");
    for group_idx in 1u8..=4 {
        let row = adw::EntryRow::new();
        row.set_title(&format!("Group {group_idx}"));
        if let Some(name) = cfg.icons.icon_for(group_idx) {
            row.set_text(name);
        }
        {
            let editor = editor.clone();
            row.connect_changed(move |r| {
                let name = r.text().as_str().to_owned();
                if let Err(e) = editor.borrow_mut().set_icon_for_group(group_idx, name) {
                    tracing::warn!(error = %e, "icon mapping rejected");
                }
            });
        }
        mapping_group.add(&row);
    }
    page.add(&mapping_group);

    page
}

fn sound_page(editor: &SharedEditor, _toast: &adw::ToastOverlay) -> adw::PreferencesPage {
    let cfg = editor.borrow().config().clone();
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    group.set_title("Sound");

    group.add(&combo_row(
        "Mode",
        &["Off", "Manual only", "Auto only", "Both"],
        match cfg.sound.mode {
            SoundMode::Off => 0,
            SoundMode::ManualOnly => 1,
            SoundMode::AutoOnly => 2,
            SoundMode::Both => 3,
        },
        {
            let editor = editor.clone();
            move |i| {
                let mode = match i {
                    1 => SoundMode::ManualOnly,
                    2 => SoundMode::AutoOnly,
                    3 => SoundMode::Both,
                    _ => SoundMode::Off,
                };
                editor.borrow_mut().set_sound_mode(mode);
            }
        },
    ));

    let file_row = adw::EntryRow::new();
    file_row.set_title("Sound file (empty = built-in click)");
    file_row.set_text(&cfg.sound.file);
    {
        let editor = editor.clone();
        file_row.connect_changed(move |row| {
            editor
                .borrow_mut()
                .set_sound_file(row.text().as_str().to_owned());
        });
    }
    group.add(&file_row);

    page.add(&group);
    page
}

fn rules_page(editor: &SharedEditor, toast: &adw::ToastOverlay) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    group.set_title("Per-application rules");
    group.set_description(Some(
        "Each rule matches a window property against a glob pattern. \
         The first matching rule wins.",
    ));

    let add_button = Button::from_icon_name("list-add-symbolic");
    add_button.set_tooltip_text(Some("Add rule"));
    add_button.add_css_class("flat");
    group.set_header_suffix(Some(&add_button));

    let cfg = editor.borrow().config().clone();
    for (idx, rule) in cfg.app_rules.iter().enumerate() {
        group.add(&rule_action_row(idx, rule, editor, toast, &group));
    }
    {
        let editor = editor.clone();
        let toast = toast.clone();
        let group_widget = group.clone();
        add_button.connect_clicked(move |_| {
            // Add a sane default rule and let the user edit it.
            let new_rule = AppRule {
                match_: AppMatch::WmClassClass("MyApp*".into()),
                action: RuleAction::Ignore,
            };
            match editor.borrow_mut().add_app_rule(new_rule.clone()) {
                Ok(()) => {
                    let new_idx = editor.borrow().config().app_rules.len() - 1;
                    let row = rule_action_row(new_idx, &new_rule, &editor, &toast, &group_widget);
                    group_widget.add(&row);
                }
                Err(e) => toast.add_toast(adw::Toast::new(&e.to_string())),
            }
        });
    }

    page.add(&group);
    page
}

// ---------------------------------------------------------------------
// Reusable row builders
// ---------------------------------------------------------------------

fn switch_row<F>(title: &str, subtitle: Option<&str>, initial: bool, on_change: F) -> adw::SwitchRow
where
    F: Fn(bool) + 'static,
{
    let row = adw::SwitchRow::new();
    row.set_title(title);
    if let Some(s) = subtitle {
        row.set_subtitle(s);
    }
    row.set_active(initial);
    row.connect_active_notify(move |r| on_change(r.is_active()));
    row
}

fn spin_row_validated<F>(
    title: &str,
    range: (f64, f64, f64),
    initial: f64,
    toast: &adw::ToastOverlay,
    on_change: F,
) -> adw::SpinRow
where
    F: Fn(f64) -> Result<(), ValidationError> + 'static,
{
    let row = adw::SpinRow::with_range(range.0, range.1, range.2);
    row.set_title(title);
    row.set_value(initial);
    let toast = toast.clone();
    row.connect_value_notify(move |r| {
        if let Err(e) = on_change(r.value()) {
            toast.add_toast(adw::Toast::new(&e.to_string()));
        }
    });
    row
}

fn combo_row<F>(title: &str, options: &[&str], initial: usize, on_change: F) -> adw::ComboRow
where
    F: Fn(usize) + 'static,
{
    let row = adw::ComboRow::new();
    row.set_title(title);
    let model = gtk4::StringList::new(options);
    row.set_model(Some(&model));
    row.set_selected(initial as u32);
    row.connect_selected_notify(move |r| on_change(r.selected() as usize));
    row
}

fn border_block<F>(
    title: &str,
    initial: BorderConfig,
    toast: &adw::ToastOverlay,
    apply: F,
) -> adw::ExpanderRow
where
    F: Fn(BorderConfig) -> Result<(), ValidationError> + 'static,
{
    let expander = adw::ExpanderRow::new();
    expander.set_title(title);
    expander.set_show_enable_switch(true);
    expander.set_enable_expansion(initial.enabled);

    let state = Rc::new(RefCell::new(initial.clone()));
    let apply = Rc::new(apply);

    let push = {
        let state = state.clone();
        let apply = apply.clone();
        let toast = toast.clone();
        Rc::new(move || {
            let snapshot = state.borrow().clone();
            if let Err(e) = apply(snapshot) {
                toast.add_toast(adw::Toast::new(&e.to_string()));
            }
        })
    };

    {
        let state = state.clone();
        let push = push.clone();
        expander.connect_enable_expansion_notify(move |row| {
            state.borrow_mut().enabled = row.enables_expansion();
            push();
        });
    }

    let color_row = adw::EntryRow::new();
    color_row.set_title("Color (#RRGGBB or #RRGGBBAA)");
    color_row.set_text(&initial.color);
    {
        let state = state.clone();
        let push = push.clone();
        color_row.connect_changed(move |r| {
            state.borrow_mut().color = r.text().as_str().to_owned();
            push();
        });
    }

    let width_row = adw::SpinRow::with_range(0.0, 32.0, 1.0);
    width_row.set_title("Width (px)");
    width_row.set_value(f64::from(initial.width));
    {
        let state = state.clone();
        let push = push.clone();
        width_row.connect_value_notify(move |r| {
            state.borrow_mut().width = r.value() as u32;
            push();
        });
    }

    expander.add_row(&color_row);
    expander.add_row(&width_row);
    expander
}

fn rule_action_row(
    initial_idx: usize,
    rule: &AppRule,
    editor: &SharedEditor,
    toast: &adw::ToastOverlay,
    group_widget: &adw::PreferencesGroup,
) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    let (kind, pattern) = match &rule.match_ {
        AppMatch::WmClassClass(p) => ("WM_CLASS.class", p.clone()),
        AppMatch::WmClassName(p) => ("WM_CLASS.name", p.clone()),
        AppMatch::WmName(p) => ("WM_NAME", p.clone()),
    };
    row.set_title(&pattern);
    row.set_subtitle(&format!("{kind} → {}", action_label(rule.action)));

    let delete = Button::from_icon_name("user-trash-symbolic");
    delete.set_tooltip_text(Some("Delete rule"));
    delete.add_css_class("flat");
    {
        let editor = editor.clone();
        let toast = toast.clone();
        let row_widget = row.clone();
        let group_widget = group_widget.clone();
        // We capture initial_idx but the *real* current index can shift
        // as other rules are added or removed. Look it up by pattern at
        // the moment the user clicks.
        let pattern = pattern.clone();
        delete.connect_clicked(move |_| {
            let current_idx =
                editor
                    .borrow()
                    .config()
                    .app_rules
                    .iter()
                    .position(|r| match &r.match_ {
                        AppMatch::WmClassClass(p)
                        | AppMatch::WmClassName(p)
                        | AppMatch::WmName(p) => p == &pattern,
                    });
            let idx = current_idx.unwrap_or(initial_idx);
            match editor.borrow_mut().remove_app_rule(idx) {
                Ok(()) => {
                    group_widget.remove(&row_widget);
                }
                Err(e) => toast.add_toast(adw::Toast::new(&e.to_string())),
            }
        });
    }
    row.add_suffix(&delete);
    row
}

fn action_label(a: RuleAction) -> String {
    match a {
        RuleAction::Ignore => "ignore".into(),
        RuleAction::StartAlt => "start in alt group".into(),
        RuleAction::AltGroup(g) => format!("alt group {}", g.as_one_based()),
    }
}
