use std::{
    collections::HashMap,
    sync::mpsc::{self, Receiver, Sender},
};

use anyhow::{Context, Result, ensure};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::{
    api::{Release, Store, StoreInfo, StoreSdk, TrySdk, try_sdk_root},
    definition::{Definition, Document, Sdk},
    theme::{Palette, Theme},
};

fn curated_sdks() -> Vec<StoreSdk> {
    sdk_list(include_str!("../data/curated-sdks.txt"))
}

fn toolchain_sdks() -> Vec<StoreSdk> {
    sdk_list(include_str!("../data/toolchain-sdks.txt"))
}

fn sdk_list(names: &str) -> Vec<StoreSdk> {
    names
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| StoreSdk {
            name: name.to_owned(),
            metadata: Default::default(),
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Tab {
    Store,
    Toolchains,
    Try,
    Preview,
}

struct Catalog {
    store_selected: usize,
    results: Vec<StoreSdk>,
    curated: bool,
    selected_view: bool,
    busy: bool,
}

impl Catalog {
    fn new(results: Vec<StoreSdk>) -> Self {
        Self {
            store_selected: 0,
            results,
            curated: true,
            selected_view: false,
            busy: false,
        }
    }
}

#[derive(Clone, Copy)]
enum Field {
    Name,
    Base,
    Runtime,
    AddSdk,
    Search,
}

struct Input {
    field: Field,
    label: &'static str,
    text: String,
}

enum Message {
    Search(Tab, Result<Vec<StoreSdk>, String>),
    Info(String, Result<StoreInfo, String>),
}

#[derive(PartialEq, Debug)]
pub enum Action {
    Continue,
    Quit,
    Editor,
}

pub struct App {
    pub document: Document,
    store: Store,
    arch: String,
    tab: Tab,
    palette: Palette,
    catalog: Catalog,
    inactive_catalogs: HashMap<Tab, Catalog>,
    try_sdks: Vec<TrySdk>,
    exit_notice: Option<String>,
    catalog_tab: Tab,
    details: HashMap<String, Result<StoreInfo, String>>,
    details_pending: Option<String>,
    channel_picker: Option<String>,
    channels: Vec<Release>,
    channel_selected: usize,
    input: Option<Input>,
    status: String,
    confirm_quit: bool,
    scroll: u16,
    tx: Sender<Message>,
    rx: Receiver<Message>,
}

impl App {
    pub fn new(document: Document, store: Store, arch: String) -> Self {
        let discovered = try_sdk_root().map_or_else(|| Ok(vec![]), |root| TrySdk::discover(&root));
        Self::with_try_sdks(document, store, arch, discovered)
    }

    fn with_try_sdks(document: Document, store: Store, arch: String, discovered: Result<Vec<TrySdk>>) -> Self {
        let (tx, rx) = mpsc::channel();
        let exit_notice = discovered.as_ref().err().map(|error| format!(
            "Local try SDK detection was disabled because the try directory is not accessible: {error:#}.\nFor snap installations, enable detection with:\n  sudo snap connect workshop-config:workshop-try\nThen restart workshop-config."
        ));
        let try_sdks = discovered.unwrap_or_default();
        let local_catalog = Catalog::new(try_sdks.iter().map(|sdk| StoreSdk {
            name: sdk.name.clone(), metadata: Default::default(),
        }).collect());
        Self {
            document,
            store,
            arch,
            tab: Tab::Store,
            palette: Palette::new(Theme::Light, true),
            catalog: Catalog::new(curated_sdks()),
            inactive_catalogs: HashMap::from([
                (Tab::Toolchains, Catalog::new(toolchain_sdks())),
                (Tab::Try, local_catalog),
            ]),
            try_sdks,
            exit_notice,
            catalog_tab: Tab::Store,
            details: HashMap::new(),
            details_pending: None,
            channel_picker: None,
            channels: vec![],
            channel_selected: 0,
            input: None,
            status: "Space/Enter toggles SDKs; c optionally changes channel. Save with s (comments are removed).".into(),
            confirm_quit: false,
            scroll: 0,
            tx,
            rx,
        }
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
    }

    pub fn exit_notice(&self) -> Option<&str> {
        self.exit_notice.as_deref()
    }

    pub fn set_error(&mut self, error: impl std::fmt::Display) {
        self.status = format!("Error: {error}");
    }

    fn switch_tab(&mut self, tab: Tab) {
        if tab != Tab::Preview && tab != self.catalog_tab {
            let catalog = self.inactive_catalogs.remove(&tab).unwrap();
            let previous = std::mem::replace(&mut self.catalog, catalog);
            self.inactive_catalogs.insert(self.catalog_tab, previous);
            self.catalog_tab = tab;
            if self.catalog.selected_view {
                self.show_selected();
            }
        }
        self.tab = tab;
        self.scroll = 0;
    }

    fn tabs(&self) -> Vec<(Tab, &'static str)> {
        let mut tabs = vec![(Tab::Store, "1 SDKs"), (Tab::Toolchains, "2 Toolchains")];
        if !self.try_sdks.is_empty() {
            tabs.push((Tab::Try, "3 Try SDKs"));
            tabs.push((Tab::Preview, "4 YAML preview"));
        } else {
            tabs.push((Tab::Preview, "3 YAML preview"));
        }
        tabs
    }

    pub fn apply_edited(&mut self, definition: Definition) {
        let previous = std::mem::replace(&mut self.document.definition, definition);
        if let Err(error) = self.document.validate() {
            self.document.definition = previous;
            self.set_error(error);
        } else {
            if self.catalog.selected_view {
                self.show_selected();
            }
            self.status = "Advanced YAML applied in memory. Press s to save.".into();
        }
    }

    fn input(&mut self, field: Field, label: &'static str, text: String) {
        self.input = Some(Input { field, label, text });
    }

    fn accept_input(&mut self, input: Input) -> Result<()> {
        let text = input.text.trim().to_owned();
        if matches!(input.field, Field::Search) {
            ensure!(!self.catalog.busy, "A request is already running");
            self.search(text);
            return Ok(());
        }
        let mut definition = self.document.definition.clone();
        match input.field {
            Field::Name => definition.name = text,
            Field::Base => definition.base = text,
            Field::Runtime => definition.runtime = if text.is_empty() { None } else { Some(text) },
            Field::AddSdk => definition.add_sdk(Sdk::new(text, None))?,
            Field::Search => unreachable!(),
        }
        definition.validate()?;
        let previous = std::mem::replace(&mut self.document.definition, definition);
        if let Err(error) = self.document.validate() {
            self.document.definition = previous;
            return Err(error);
        }
        self.status = "Definition updated in memory. Press s to save.".into();
        Ok(())
    }

    fn search(&mut self, query: String) {
        if self.catalog_tab == Tab::Try {
            self.catalog.results = self.try_sdks.iter()
                .filter(|sdk| sdk.name.contains(&query))
                .map(|sdk| StoreSdk { name: sdk.name.clone(), metadata: Default::default() })
                .collect();
            self.catalog.store_selected = 0;
            self.catalog.selected_view = false;
            self.scroll = 0;
            return;
        }
        let (store, tx) = (self.store.clone(), self.tx.clone());
        let tab = self.catalog_tab;
        self.catalog.busy = true;
        self.status = "Searching the SDK Store...".into();
        self.channel_picker = None;
        std::thread::spawn(move || {
            let result = store.find(&query).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Message::Search(tab, result));
        });
    }

    fn show_curated(&mut self) {
        if self.catalog_tab == Tab::Try {
            self.search(String::new());
            return;
        }
        self.catalog.results = if self.catalog_tab == Tab::Toolchains {
            toolchain_sdks()
        } else {
            curated_sdks()
        };
        self.catalog.curated = true;
        self.catalog.selected_view = false;
        self.catalog.store_selected = 0;
        self.scroll = 0;
        self.status = "Curated list. Space/Enter toggles SDKs; c changes channel.".into();
    }

    fn show_selected(&mut self) {
        self.catalog.results = self
            .document
            .definition
            .sdks
            .iter()
            .map(|sdk| StoreSdk {
                name: sdk.name.clone(),
                metadata: Default::default(),
            })
            .collect();
        self.catalog.curated = false;
        self.catalog.selected_view = true;
        self.catalog.store_selected = self
            .catalog
            .store_selected
            .min(self.catalog.results.len().saturating_sub(1));
        self.scroll = 0;
    }

    fn load_highlighted(&mut self) {
        if self.tab == Tab::Preview
            || self.catalog.busy
            || self.details_pending.is_some()
            || self.confirm_quit
        {
            return;
        }
        let Some(sdk) = self.catalog.results.get(self.catalog.store_selected) else {
            return;
        };
        if sdk.name.starts_with("try-") || sdk.name.starts_with("project-") || sdk.name == "system" {
            return;
        }
        if self.details.contains_key(&sdk.name) {
            return;
        }
        let (store, tx, name) = (self.store.clone(), self.tx.clone(), sdk.name.clone());
        self.details_pending = Some(name.clone());
        std::thread::spawn(move || {
            let result = store.info(&name).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Message::Info(name, result));
        });
    }

    fn highlighted_info(&self) -> Result<&StoreInfo> {
        let sdk = self
            .catalog
            .results
            .get(self.catalog.store_selected)
            .context("Select an SDK first")?;
        match self.details.get(&sdk.name) {
            Some(Ok(info)) => Ok(info),
            Some(Err(error)) => anyhow::bail!("{error}"),
            None => anyhow::bail!(
                "Details for {} are loading; select it once loading completes",
                sdk.name
            ),
        }
    }

    fn toggle_sdk(&mut self) -> Result<()> {
        let name = self
            .catalog
            .results
            .get(self.catalog.store_selected)
            .context("Select an SDK first")?
            .name
            .clone();
        if let Some(index) = self
            .document
            .definition
            .sdks
            .iter()
            .position(|sdk| sdk.name == name)
        {
            self.document.definition.remove_sdk(index)?;
            if self.catalog.selected_view {
                self.show_selected();
            }
            self.status = format!("Deselected {name}. Press s to save.");
        } else {
            if name.starts_with("try-") {
                self.try_sdks.iter().find(|sdk| sdk.name == name)
                    .context("Local SDK is no longer available")?
                    .metadata(&self.document.definition.base, &self.arch)?;
                self.document.definition.add_sdk(Sdk::new(name.clone(), None))?;
                self.status = format!("Selected {name}. Press s to save.");
                return Ok(());
            }
            let release = self
                .highlighted_info()?
                .compatible_releases(&self.document.definition.base, &self.arch)
                .into_iter()
                .next()
                .context("No compatible release is available for this base and architecture")?;
            self.document
                .definition
                .add_sdk(Sdk::new(name.clone(), Some(release.channel.name)))?;
            self.status = format!("Selected {name}. Keep selecting SDKs; s saves the combination.");
        }
        Ok(())
    }

    fn open_channels(&mut self) -> Result<()> {
        ensure!(
            !self.catalog.results.get(self.catalog.store_selected)
                .is_some_and(|sdk| sdk.name.starts_with("try-")),
            "Local try SDKs have no channels"
        );
        let info = self.highlighted_info()?;
        let channels = info.compatible_releases(&self.document.definition.base, &self.arch);
        ensure!(
            !channels.is_empty(),
            "No compatible channels are available for this base and architecture"
        );
        let name = info.name.clone();
        let current = self
            .document
            .definition
            .sdks
            .iter()
            .find(|sdk| sdk.name == name)
            .and_then(|sdk| sdk.channel.as_deref());
        self.channel_selected = channels
            .iter()
            .position(|r| Some(r.channel.name.as_str()) == current)
            .unwrap_or(0);
        self.channels = channels;
        self.channel_picker = Some(name);
        Ok(())
    }

    fn apply_channel(&mut self) -> Result<()> {
        let name = self
            .channel_picker
            .as_ref()
            .context("No SDK selected")?
            .clone();
        let channel = self
            .channels
            .get(self.channel_selected)
            .context("No compatible channel selected")?
            .channel
            .name
            .clone();
        let mut definition = self.document.definition.clone();
        if let Some(sdk) = definition.sdks.iter_mut().find(|sdk| sdk.name == name) {
            sdk.channel = Some(channel.clone());
        } else {
            definition.add_sdk(Sdk::new(name.clone(), Some(channel.clone())))?;
        }
        definition.validate()?;
        self.document.definition = definition;
        self.channel_picker = None;
        self.status = format!("Selected {name} [{channel}]. Keep selecting SDKs; s saves.");
        Ok(())
    }

    pub fn receive(&mut self) {
        while let Ok(message) = self.rx.try_recv() {
            match message {
                Message::Search(tab, result) => {
                    let catalog = if tab == self.catalog_tab {
                        &mut self.catalog
                    } else {
                        self.inactive_catalogs.get_mut(&tab).unwrap()
                    };
                    catalog.busy = false;
                    match result {
                        Ok(results) => {
                            catalog.results = results;
                            catalog.curated = false;
                            catalog.selected_view = false;
                            catalog.store_selected = 0;
                        }
                        Err(error) => {
                            self.set_error(error);
                            continue;
                        }
                    }
                    self.status = format!(
                        "{} SDKs found. Space/Enter toggles SDKs; c changes channel.",
                        catalog.results.len()
                    );
                    if tab == self.catalog_tab {
                        self.scroll = 0;
                    }
                }
                Message::Info(name, result) => {
                    self.details_pending = None;
                    if self
                        .catalog
                        .results
                        .get(self.catalog.store_selected)
                        .is_some_and(|sdk| sdk.name == name)
                        && let Err(error) = &result
                    {
                        self.set_error(error);
                    }
                    self.details.insert(name, result);
                }
            }
        }
    }

    pub fn tick(&mut self) {
        self.receive();
        self.load_highlighted();
    }

    pub fn key(&mut self, key: KeyEvent) -> Result<Action> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            if self.confirm_quit || !self.document.dirty() {
                return Ok(Action::Quit);
            }
            self.confirm_quit = true;
            return Ok(Action::Continue);
        }
        if self.confirm_quit {
            match key.code {
                KeyCode::Char('y') => return Ok(Action::Quit),
                KeyCode::Esc | KeyCode::Char('n') => self.confirm_quit = false,
                _ => {}
            }
            return Ok(Action::Continue);
        }
        if let Some(mut input) = self.input.take() {
            match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter => {
                    // Keep rejected input available for correction rather than discarding it.
                    let retry = Input {
                        field: input.field,
                        label: input.label,
                        text: input.text.clone(),
                    };
                    if let Err(error) = self.accept_input(input) {
                        self.input = Some(retry);
                        return Err(error);
                    }
                }
                KeyCode::Backspace => {
                    input.text.pop();
                    self.input = Some(input);
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    input.text.clear();
                    self.input = Some(input);
                }
                KeyCode::Char(c)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    input.text.push(c);
                    self.input = Some(input);
                }
                _ => self.input = Some(input),
            }
            return Ok(Action::Continue);
        }
        if self.channel_picker.is_some() {
            match key.code {
                KeyCode::Esc => self.channel_picker = None,
                KeyCode::Down | KeyCode::Char('j') => {
                    self.channel_selected = next(self.channel_selected, self.channels.len())
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.channel_selected = self.channel_selected.saturating_sub(1)
                }
                KeyCode::Enter => self.apply_channel()?,
                _ => {}
            }
            return Ok(Action::Continue);
        }
        match key.code {
            KeyCode::Char('q') => {
                if self.document.dirty() {
                    self.confirm_quit = true;
                } else {
                    return Ok(Action::Quit);
                }
            }
            KeyCode::Tab | KeyCode::BackTab => {
                let tabs = self.tabs();
                let index = tabs.iter().position(|(tab, _)| *tab == self.tab).unwrap_or(0);
                let offset = if key.code == KeyCode::BackTab { tabs.len() - 1 } else { 1 };
                self.switch_tab(tabs[(index + offset) % tabs.len()].0);
            }
            KeyCode::Char('s') => {
                self.document.save()?;
                self.status = format!("Saved {}", self.document.path.display());
            }
            KeyCode::Char('e') => return Ok(Action::Editor),
            KeyCode::Char('1') => {
                self.switch_tab(Tab::Store);
            }
            KeyCode::Char('2') => {
                self.switch_tab(Tab::Toolchains);
            }
            KeyCode::Char('3') => {
                self.switch_tab(self.tabs()[2].0);
            }
            KeyCode::Char('4') if !self.try_sdks.is_empty() => self.switch_tab(Tab::Preview),
            KeyCode::Char('n') => self.input(
                Field::Name,
                "Workshop name",
                self.document.definition.name.clone(),
            ),
            KeyCode::Char('b') => self.input(
                Field::Base,
                "Base: ubuntu@22.04 / ubuntu@24.04 / ubuntu@26.04",
                self.document.definition.base.clone(),
            ),
            KeyCode::Char('t') => self.input(
                Field::Runtime,
                "Runtime: lxd-container / lxd-vm (empty = default)",
                self.document.definition.runtime.clone().unwrap_or_default(),
            ),
            KeyCode::Char('a') => self.input(
                Field::AddSdk,
                "SDK name (including system, try-*, project-*)",
                String::new(),
            ),
            _ => self.tab_key(key.code)?,
        }
        Ok(Action::Continue)
    }

    fn tab_key(&mut self, code: KeyCode) -> Result<()> {
        match self.tab {
            Tab::Store | Tab::Toolchains | Tab::Try => match code {
                KeyCode::Char('/') => self.input(
                    Field::Search,
                    if self.tab == Tab::Try { "Search local try SDKs" } else { "Search SDK Store (empty = browse all)" },
                    String::new(),
                ),
                KeyCode::Char('r') if !self.catalog.busy => self.search(String::new()),
                KeyCode::Char('f') if !self.catalog.busy => self.show_curated(),
                KeyCode::Char('o') if !self.catalog.busy => {
                    self.catalog.store_selected = 0;
                    self.show_selected();
                    self.status =
                        "Selected SDKs in installation order. J/K reorders; Space deselects."
                            .into();
                }
                KeyCode::Char('J') | KeyCode::Char('K') => {
                    let name = self
                        .catalog
                        .results
                        .get(self.catalog.store_selected)
                        .context("Select an SDK first")?
                        .name
                        .clone();
                    let index = self
                        .document
                        .definition
                        .sdks
                        .iter()
                        .position(|sdk| sdk.name == name)
                        .context("Check this SDK before reordering it")?;
                    let target = if code == KeyCode::Char('J') {
                        next(index, self.document.definition.sdks.len())
                    } else {
                        index.saturating_sub(1)
                    };
                    self.document.definition.sdks.swap(index, target);
                    if self.catalog.selected_view {
                        self.catalog.store_selected = target;
                        self.show_selected();
                    }
                    self.status = format!(
                        "Moved {name} to installation position {}. Press s to save.",
                        target + 1
                    );
                }
                KeyCode::Enter | KeyCode::Char(' ') => self.toggle_sdk()?,
                KeyCode::Char('c') => self.open_channels()?,
                KeyCode::Char('i') => {
                    let sdk = self
                        .catalog
                        .results
                        .get(self.catalog.store_selected)
                        .context("Select an SDK first")?;
                    self.details.remove(&sdk.name);
                    self.status = format!("Retrying details for {}...", sdk.name);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.catalog.store_selected =
                        next(self.catalog.store_selected, self.catalog.results.len());
                    self.scroll = 0;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.catalog.store_selected = self.catalog.store_selected.saturating_sub(1);
                    self.scroll = 0;
                }
                KeyCode::PageDown => self.scroll = self.scroll.saturating_add(5),
                KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(5),
                _ => {}
            },
            Tab::Preview => match code {
                KeyCode::Down | KeyCode::Char('j') => self.scroll = self.scroll.saturating_add(1),
                KeyCode::Up | KeyCode::Char('k') => self.scroll = self.scroll.saturating_sub(1),
                _ => {}
            },
        }
        Ok(())
    }

    pub fn draw(&self, frame: &mut Frame) {
        frame.render_widget(Block::default().style(self.palette.base()), frame.area());
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(2),
                Constraint::Min(3),
                Constraint::Length(3),
                Constraint::Length(2),
            ])
            .split(frame.area());
        let tabs = self.tabs();
        let line = Line::from(
            tabs.into_iter()
                .map(|(tab, name)| {
                    Span::styled(
                        format!(" {name} "),
                        if self.tab == tab {
                            self.palette
                                .selection()
                                .fg(self.palette.focus)
                                .add_modifier(Modifier::UNDERLINED)
                        } else {
                            self.palette.base()
                        },
                    )
                })
                .collect::<Vec<_>>(),
        );
        frame.render_widget(
            Paragraph::new(line).block(self.block().title(format!(
                "Workshop TUI  {}{}",
                self.document.path.display(),
                if self.document.dirty() {
                    " [unsaved]"
                } else {
                    ""
                }
            ))),
            rows[0],
        );
        frame.render_widget(
            Paragraph::new(format!(
                "{} | {} | {} | n name | b base | t runtime | a add SDK",
                self.document.definition.name,
                self.document.definition.base,
                self.document
                    .definition
                    .runtime
                    .as_deref()
                    .unwrap_or("default runtime"),
            ))
            .style(self.palette.base().fg(self.palette.muted)),
            rows[1],
        );
        match self.tab {
            Tab::Store | Tab::Toolchains | Tab::Try => self.draw_store(frame, rows[2]),
            Tab::Preview => {
                let yaml = self
                    .document
                    .definition
                    .yaml()
                    .unwrap_or_else(|e| format!("Cannot preview: {e:#}"));
                frame.render_widget(
                    Paragraph::new(yaml)
                        .scroll((self.scroll, 0))
                        .block(self.block().title(
                            "Generated YAML | Up/Down scroll | comments normalized on save",
                        )),
                    rows[2],
                );
            }
        }
        frame.render_widget(
            Paragraph::new(self.status.as_str())
                .wrap(Wrap { trim: false })
                .style(
                    self.palette
                        .base()
                        .fg(if self.status.starts_with("Error:") {
                            self.palette.negative
                        } else {
                            self.palette.information
                        }),
                )
                .block(self.block().title(
                    if self.catalog.busy || self.details_pending.is_some() {
                        "Request in progress"
                    } else {
                        "Status"
                    },
                )),
            rows[3],
        );
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    " s Save ",
                    self.palette
                        .base()
                        .fg(self.palette.brand)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("| Tab switch | e edit YAML | q quit | o selected | J/K reorder"),
            ]))
            .style(self.palette.base()),
            rows[4],
        );
        if let Some(name) = &self.channel_picker {
            let area = popup(frame.area(), 85, 70);
            frame.render_widget(Clear, area);
            let mut state = ListState::default()
                .with_selected((!self.channels.is_empty()).then_some(self.channel_selected));
            let items: Vec<_> = self
                .channels
                .iter()
                .map(|release| ListItem::new(release_label(release)))
                .collect();
            frame.render_stateful_widget(
                List::new(items)
                    .block(
                        self.block()
                            .title(format!("{name} channels | Enter apply | Esc cancel")),
                    )
                    .highlight_style(self.palette.selection())
                    .highlight_symbol("> "),
                area,
                &mut state,
            );
        }
        if let Some(input) = &self.input {
            let area = popup(frame.area(), 90, 25);
            frame.render_widget(Clear, area);
            // Tail display keeps newly typed text visible for long values.
            let width = area.width.saturating_sub(4) as usize;
            let text: String = input
                .text
                .chars()
                .rev()
                .take(width)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            frame.render_widget(
                Paragraph::new(format!("{text}_\nEnter apply | Esc cancel | Ctrl-U clear")).block(
                    self.block()
                        .border_style(Style::default().fg(self.palette.focus))
                        .title(input.label),
                ),
                area,
            );
        }
        if self.confirm_quit {
            let area = popup(frame.area(), 75, 25);
            frame.render_widget(Clear, area);
            frame.render_widget(
                Paragraph::new(
                    "Discard unsaved changes and quit?\ny = discard | n / Esc = keep editing",
                )
                .block(self.block().title("Unsaved definition")),
                area,
            );
        }
    }

    fn block(&self) -> Block<'static> {
        Block::default()
            .borders(Borders::ALL)
            .style(self.palette.base().bg(self.palette.surface))
            .border_style(Style::default().fg(self.palette.border))
    }

    fn draw_store(&self, frame: &mut Frame, area: Rect) {
        let layout = if area.width >= 90 {
            Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)])
        } else {
            Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)])
        };
        let panes = layout.split(area);
        let items: Vec<_> = self
            .catalog
            .results
            .iter()
            .map(|sdk| {
                let selected = self
                    .document
                    .definition
                    .sdks
                    .iter()
                    .find(|s| s.name == sdk.name);
                let label = if sdk.name.starts_with("try-") {
                    match self.try_sdks.iter().find(|local| local.name == sdk.name)
                        .context("Local SDK missing")
                        .and_then(|local| local.metadata(&self.document.definition.base, &self.arch)) {
                        Ok(_) => "local".into(),
                        Err(_) => "local (unavailable)".into(),
                    }
                } else { match self.details.get(&sdk.name) {
                    Some(Ok(info)) => {
                        let releases =
                            info.compatible_releases(&self.document.definition.base, &self.arch);
                        let current = selected
                            .and_then(|s| s.channel.as_deref())
                            .unwrap_or("latest/stable");
                        let release = if selected.is_some() {
                            releases.iter().find(|r| r.channel.name == current)
                        } else {
                            releases.first()
                        };
                        release
                            .map(release_label)
                            .unwrap_or_else(|| format!("{current} (unavailable)"))
                    }
                    Some(Err(_)) => "details unavailable".into(),
                    None => selected
                        .and_then(|s| s.channel.clone())
                        .unwrap_or_else(|| "latest/stable (not checked)".into()),
                }};
                ListItem::new(format!(
                    "[{}] {}  [{label}]",
                    if selected.is_some() { "x" } else { " " },
                    sdk.name
                ))
            })
            .collect();
        let mut state = ListState::default()
            .with_selected((!items.is_empty()).then_some(self.catalog.store_selected));
        frame.render_stateful_widget(
            List::new(items)
                .block(
                    self.block()
                        .title(format!(
                            "{} | {} selected",
                            if self.catalog.selected_view {
                                "Selected SDKs"
                            } else if self.catalog_tab == Tab::Try {
                                "Local try SDKs"
                            } else if self.catalog.curated {
                                if self.catalog_tab == Tab::Toolchains {
                                    "Toolchains"
                                } else {
                                    "Curated SDKs"
                                }
                            } else {
                                "SDK Store"
                            },
                            self.document.definition.sdks.len(),
                        ))
                        .title_bottom(if self.catalog_tab == Tab::Try { "Space/Enter toggle" } else { "Space/Enter toggle | c channel" }),
                )
                .highlight_style(self.palette.selection())
                .highlight_symbol("> "),
            panes[0],
            &mut state,
        );

        let detail = match self.catalog.results.get(self.catalog.store_selected) {
            None => {
                "No SDKs to display. Press / to search, r to browse all, or f for curated SDKs."
                    .into()
            }
            Some(sdk) => {
                let mut text = format!(
                    "{}\nTarget: {} / {}\n\n",
                    sdk.name, self.document.definition.base, self.arch
                );
                if sdk.name.starts_with("try-") {
                    match self.try_sdks.iter().find(|local| local.name == sdk.name)
                        .context("Local SDK missing")
                        .and_then(|local| local.metadata(&self.document.definition.base, &self.arch)) {
                        Ok(metadata) => {
                            text.push_str(&format!("Local try SDK\n\n{}\n\n{}", metadata.summary, metadata.description));
                        }
                        Err(error) => text.push_str(&format!("Local SDK unavailable:\n{error:#}")),
                    }
                } else { match self.details.get(&sdk.name) {
                    Some(Ok(info)) => {
                        let releases =
                            info.compatible_releases(&self.document.definition.base, &self.arch);
                        let selected = self
                            .document
                            .definition
                            .sdks
                            .iter()
                            .find(|s| s.name == sdk.name);
                        let release = if let Some(selected) = selected {
                            let channel = selected.channel.as_deref().unwrap_or("latest/stable");
                            releases.iter().find(|r| r.channel.name == channel)
                        } else {
                            releases.first()
                        };
                        if let Some(release) = release {
                            text.push_str(&format!(
                                "{}: {}\nLatest published version in this channel.\n\n",
                                if selected.is_some() {
                                    "Selected channel"
                                } else {
                                    "Default channel"
                                },
                                release_label(release)
                            ));
                        } else {
                            text.push_str("No compatible release for the selected channel/target.\nPress c to choose another channel, or change base/architecture.\n\n");
                        }
                        text.push_str(&info.metadata.summary);
                        text.push_str("\n\n");
                        text.push_str(if info.metadata.description.is_empty() {
                            "No description published."
                        } else {
                            &info.metadata.description
                        });
                    }
                    Some(Err(error)) => text.push_str(&format!(
                        "Cannot load SDK details:\n{error}\n\nPress i to retry."
                    )),
                    None => {
                        text.push_str(&sdk.metadata.summary);
                        text.push_str("\n\nLoading description and latest channel version...");
                    }
                }}
                text
            }
        };
        frame.render_widget(
            Paragraph::new(detail)
                .wrap(Wrap { trim: false })
                .scroll((self.scroll, 0))
                .block(
                    self.block()
                        .title("SDK description | PgUp/PgDn scroll")
                        .title_bottom(if self.catalog_tab == Tab::Try { "/ search | r all" } else { "f curated | / search | r all | i retry" }),
                ),
            panes[1],
        );
    }
}

fn release_label(release: &Release) -> String {
    format!(
        "{} | {} | revision {}",
        release.channel.name,
        if release.revision.version.is_empty() {
            "version not published"
        } else {
            &release.revision.version
        },
        release.revision.revision
    )
}

fn next(index: usize, length: usize) -> usize {
    index.saturating_add(1).min(length.saturating_sub(1))
}

fn popup(area: Rect, width: u16, height: u16) -> Rect {
    let w = (area.width as u32 * width as u32 / 100) as u16;
    let h = ((area.height as u32 * height as u32 / 100) as u16)
        .max(4)
        .min(area.height);
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn app() -> App {
        let dir = tempfile::tempdir().unwrap();
        let mut document = Document::open(dir.path().join("workshop.yaml")).unwrap();
        document.definition.sdks.clear();
        let mut app = App::new(
            document,
            Store::new("http://127.0.0.1:1").unwrap(),
            "amd64".into(),
        );
        app.try_sdks.clear();
        app.inactive_catalogs.insert(Tab::Try, Catalog::new(vec![]));
        app
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn editing_validation_and_quit_confirmation() {
        let mut app = app();
        let initial_name = app.document.definition.name.clone();
        app.key(key(KeyCode::Char('1'))).unwrap();
        app.key(key(KeyCode::Char('n'))).unwrap();
        app.input.as_mut().unwrap().text = "Bad".into();
        assert!(app.key(key(KeyCode::Enter)).is_err());
        assert_eq!(app.document.definition.name, initial_name);
        app.input.as_mut().unwrap().text = "design-tools".into();
        app.key(key(KeyCode::Enter)).unwrap();
        assert_eq!(app.document.definition.name, "design-tools");
        assert_eq!(app.key(key(KeyCode::Char('q'))).unwrap(), Action::Continue);
        assert_eq!(app.key(key(KeyCode::Char('y'))).unwrap(), Action::Quit);
    }

    #[test]
    fn starts_with_curated_sdks_in_source_order_without_network() {
        let app = app();
        assert!(app.tab == Tab::Store);
        assert!(app.catalog.curated);
        assert!(!app.catalog.busy);
        assert!(!app.document.path.exists());
        let names: Vec<_> = app
            .catalog
            .results
            .iter()
            .map(|sdk| sdk.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "opencode",
                "pi",
                "omp",
                "hermes-agent",
                "copilot",
                "claude-code",
                "codex",
                "dsh",
                "herdr",
                "hindsight",
                "kiro",
            ]
        );
    }

    #[test]
    fn local_try_tab_selects_without_channels_or_store_requests() {
        let directory = tempfile::tempdir().unwrap();
        let artifact = directory.path().join("tools_amd64_ubuntu@24.04.sdk");
        std::fs::write(&artifact, "artifact").unwrap();
        std::fs::write(artifact.with_extension("sdk.yaml"), "name: tools\nsummary: Local tools\n").unwrap();
        std::fs::write(artifact.with_extension("sdk.sha3-384"), "digest").unwrap();
        let mut app = app();
        app.try_sdks = vec![TrySdk { name: "try-tools".into(), directory: directory.path().into() }];
        app.inactive_catalogs.insert(Tab::Try, Catalog::new(vec![StoreSdk {
            name: "try-tools".into(), metadata: Default::default(),
        }]));
        app.key(key(KeyCode::Char('3'))).unwrap();
        assert!(app.tab == Tab::Try);
        app.tick();
        assert!(app.details_pending.is_none());
        app.key(key(KeyCode::Enter)).unwrap();
        assert_eq!(app.document.definition.sdks[0].name, "try-tools");
        assert!(app.document.definition.sdks[0].channel.is_none());
        assert!(app.key(key(KeyCode::Char('c'))).is_err());
        for (width, height) in [(160, 35), (40, 12), (10, 4)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| app.draw(frame)).unwrap();
            if width == 160 {
                let text: String = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect();
                assert!(text.contains("3 Try SDKs"));
                assert!(text.contains("4 YAML preview"));
                assert!(text.contains("Local tools"));
            }
        }
        app.key(key(KeyCode::Tab)).unwrap();
        assert!(app.tab == Tab::Preview);
        app.key(key(KeyCode::BackTab)).unwrap();
        assert!(app.tab == Tab::Try);
        app.key(key(KeyCode::Char(' '))).unwrap();
        assert!(app.document.definition.sdks.is_empty());
        app.document.definition.base = "ubuntu@26.04".into();
        assert!(app.key(key(KeyCode::Enter)).is_err());
        app.search("missing".into());
        assert!(app.catalog.results.is_empty());
        app.key(key(KeyCode::Char('r'))).unwrap();
        assert_eq!(app.catalog.results[0].name, "try-tools");
        app.switch_tab(Tab::Store);
        app.catalog.busy = true;
        app.switch_tab(Tab::Try);
        app.tx.send(Message::Search(Tab::Store, Ok(vec![]))).unwrap();
        app.receive();
        assert_eq!(app.catalog.results[0].name, "try-tools");
        app.switch_tab(Tab::Store);
        assert!(!app.catalog.busy);
        assert!(app.catalog.results.is_empty());
    }

    #[test]
    fn inaccessible_try_area_hides_tab_and_defers_snap_connection_notice() {
        let fixture = app();
        let app = App::with_try_sdks(fixture.document, fixture.store, fixture.arch,
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied).into()));
        assert!(!app.tabs().iter().any(|(tab, _)| *tab == Tab::Try));
        assert!(app.tabs()[2].0 == Tab::Preview);
        assert!(!app.status.starts_with("Error:"));
        assert!(app.exit_notice().unwrap().contains("sudo snap connect workshop-config:workshop-try"));
        assert!(super::tests::app().exit_notice().is_none());
    }

    #[test]
    fn search_results_can_be_reset_to_curated_without_network() {
        let mut app = app();
        app.tx
            .send(Message::Search(
                Tab::Store,
                Ok(vec![StoreSdk {
                    name: "rust".into(),
                    metadata: Default::default(),
                }]),
            ))
            .unwrap();
        app.receive();
        assert!(!app.catalog.curated);
        assert_eq!(app.catalog.results[0].name, "rust");
        app.catalog.store_selected = 7;
        app.key(key(KeyCode::Char('f'))).unwrap();
        assert!(app.catalog.curated);
        assert_eq!(app.catalog.results[0].name, "opencode");
        assert_eq!(app.catalog.store_selected, 0);
        assert!(!app.catalog.busy);
    }

    #[test]
    fn toolchains_use_file_order_and_share_selection_details_and_channels() {
        let mut app = app();
        cache_info(&mut app, "opencode");
        app.key(key(KeyCode::Char(' '))).unwrap();
        app.key(key(KeyCode::Char('2'))).unwrap();
        let names: Vec<_> = app
            .catalog
            .results
            .iter()
            .map(|sdk| sdk.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "node",
                "uv",
                "openjdk",
                "dotnet",
                "go",
                "rust",
                "zig",
                "flutter",
                "docker-ce"
            ]
        );
        app.key(key(KeyCode::Down)).unwrap();
        cache_info(&mut app, "uv");
        let mut terminal = Terminal::new(TestBackend::new(160, 35)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Full SDK description"));
        assert!(text.contains("Default channel: latest/stable | 2.0 | revision 20"));
        app.key(key(KeyCode::Enter)).unwrap();
        app.key(key(KeyCode::Char('c'))).unwrap();
        assert_eq!(
            app.channels[app.channel_selected].channel.name,
            "latest/stable"
        );
        app.key(key(KeyCode::Down)).unwrap();
        app.key(key(KeyCode::Enter)).unwrap();
        assert_eq!(app.document.definition.sdks[1].name, "uv");
        assert_eq!(
            app.document.definition.sdks[1].channel.as_deref(),
            Some("latest/edge")
        );
        app.key(key(KeyCode::Char('1'))).unwrap();
        assert_eq!(
            app.catalog.results[app.catalog.store_selected].name,
            "opencode"
        );
        app.key(key(KeyCode::Char('2'))).unwrap();
        assert_eq!(app.catalog.results[app.catalog.store_selected].name, "uv");
        app.key(key(KeyCode::Char(' '))).unwrap();
        assert_eq!(app.document.definition.sdks.len(), 1);
        assert_eq!(app.document.definition.sdks[0].name, "opencode");
    }

    #[test]
    fn new_definitions_preselect_uv_and_respect_saved_deselection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workshop.yaml");
        let mut app = App::new(
            Document::open(path.clone()).unwrap(),
            Store::new("http://127.0.0.1:1").unwrap(),
            "amd64".into(),
        );
        assert!(!path.exists());
        assert_eq!(app.document.definition.sdks.len(), 1);
        assert_eq!(app.document.definition.sdks[0].name, "uv");
        assert_eq!(
            app.document.definition.sdks[0].channel.as_deref(),
            Some("latest/stable")
        );
        app.key(key(KeyCode::Char('2'))).unwrap();
        app.key(key(KeyCode::Down)).unwrap();
        cache_info(&mut app, "uv");
        let mut terminal = Terminal::new(TestBackend::new(160, 35)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("[x] uv"));
        app.key(key(KeyCode::Char('s'))).unwrap();
        let saved = Document::open(path.clone()).unwrap();
        assert_eq!(saved.definition.sdks, app.document.definition.sdks);
        assert!(!saved.dirty());
        app.key(key(KeyCode::Enter)).unwrap();
        app.key(key(KeyCode::Char('s'))).unwrap();
        assert!(Document::open(path).unwrap().definition.sdks.is_empty());
    }

    #[test]
    fn catalog_search_results_remain_with_the_originating_tab() {
        let mut app = app();
        app.catalog.busy = true;
        app.switch_tab(Tab::Toolchains);
        app.catalog.busy = true;
        app.tx
            .send(Message::Search(
                Tab::Store,
                Ok(vec![StoreSdk {
                    name: "copilot".into(),
                    metadata: Default::default(),
                }]),
            ))
            .unwrap();
        app.receive();
        assert!(app.catalog.busy);
        assert!(app.catalog.curated);
        assert_eq!(app.catalog.results[0].name, "node");
        app.tx
            .send(Message::Search(Tab::Toolchains, Ok(vec![])))
            .unwrap();
        app.receive();
        assert!(!app.catalog.busy);
        app.key(key(KeyCode::Char('f'))).unwrap();
        assert_eq!(app.catalog.results[0].name, "node");
        app.switch_tab(Tab::Store);
        assert!(!app.catalog.busy);
        assert!(!app.catalog.curated);
        assert_eq!(app.catalog.results[0].name, "copilot");
        app.key(key(KeyCode::Char('f'))).unwrap();
        assert_eq!(app.catalog.results[0].name, "opencode");
    }

    #[test]
    fn toolchain_tab_loads_metadata_for_the_highlighted_toolchain() {
        let mut app = app();
        app.switch_tab(Tab::Toolchains);
        app.load_highlighted();
        assert_eq!(app.details_pending.as_deref(), Some("node"));
    }

    #[test]
    fn renders_every_tab_and_small_terminals() {
        let mut app = app();
        for (w, h) in [(120, 35), (40, 12), (10, 4)] {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            for tab in [Tab::Store, Tab::Toolchains, Tab::Preview] {
                app.switch_tab(tab);
                terminal.draw(|f| app.draw(f)).unwrap();
            }
            app.input(Field::Search, "Search", String::from("rust"));
            terminal.draw(|f| app.draw(f)).unwrap();
            app.input = None;
            app.confirm_quit = true;
            terminal.draw(|f| app.draw(f)).unwrap();
            app.confirm_quit = false;
        }
    }

    #[test]
    fn navigation_has_sdk_toolchain_and_preview_views() {
        let mut app = app();
        let mut terminal = Terminal::new(TestBackend::new(120, 35)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("1 SDKs"));
        assert!(text.contains("2 Toolchains"));
        assert!(text.contains("3 YAML preview"));
        assert!(!text.contains("1 Definition") && !text.contains("4 Workshop"));
        app.key(key(KeyCode::Tab)).unwrap();
        assert!(app.tab == Tab::Toolchains);
        app.key(key(KeyCode::Tab)).unwrap();
        assert!(app.tab == Tab::Preview);
        app.key(key(KeyCode::Tab)).unwrap();
        assert!(app.tab == Tab::Store);
        app.key(key(KeyCode::BackTab)).unwrap();
        assert!(app.tab == Tab::Preview);
        app.key(key(KeyCode::BackTab)).unwrap();
        assert!(app.tab == Tab::Toolchains);
        app.key(key(KeyCode::BackTab)).unwrap();
        assert!(app.tab == Tab::Store);
        app.key(key(KeyCode::Char('2'))).unwrap();
        assert!(app.tab == Tab::Toolchains);
        app.key(key(KeyCode::Char('3'))).unwrap();
        assert!(app.tab == Tab::Preview);
        app.key(key(KeyCode::Char('4'))).unwrap();
        assert!(app.tab == Tab::Preview);
        app.key(key(KeyCode::Char('n'))).unwrap();
        app.input.as_mut().unwrap().text = "preview-edit".into();
        app.key(key(KeyCode::Enter)).unwrap();
        assert_eq!(app.document.definition.name, "preview-edit");
    }

    #[test]
    fn selected_view_reorders_and_removes_local_and_store_sdks() {
        let mut app = app();
        app.document
            .definition
            .add_sdk(Sdk::new("project-tools".into(), None))
            .unwrap();
        cache_info(&mut app, "opencode");
        app.key(key(KeyCode::Char(' '))).unwrap();
        app.key(key(KeyCode::Char('o'))).unwrap();
        assert!(app.catalog.selected_view);
        assert_eq!(app.catalog.results[0].name, "project-tools");
        app.key(key(KeyCode::Char('J'))).unwrap();
        assert_eq!(app.catalog.results[0].name, "opencode");
        assert_eq!(app.catalog.results[1].name, "project-tools");
        assert_eq!(app.catalog.store_selected, 1);
        assert_eq!(app.document.definition.sdks[1].name, "project-tools");
        app.key(key(KeyCode::Char(' '))).unwrap();
        assert_eq!(app.document.definition.sdks.len(), 1);
        assert_eq!(app.catalog.results.len(), 1);
        assert_eq!(app.catalog.store_selected, 0);
    }

    #[test]
    fn themes_cover_selection_and_overlays_and_monochrome_has_no_colors() {
        let mut app = app();
        cache_info(&mut app, "opencode");
        let mut terminal = Terminal::new(TestBackend::new(120, 35)).unwrap();
        for theme in [Theme::Light, Theme::Dark, Theme::Paper] {
            app.set_palette(Palette::new(theme, true));
            app.open_channels().unwrap();
            terminal.draw(|frame| app.draw(frame)).unwrap();
            let cells = &terminal.backend().buffer().content;
            assert!(cells.iter().any(|cell| cell.fg == app.palette.border));
            assert!(cells.iter().any(|cell| cell.bg == app.palette.active));
            app.channel_picker = None;
        }
        app.set_palette(Palette::new(Theme::Light, false));
        for overlay in 0..3 {
            match overlay {
                0 => app.open_channels().unwrap(),
                1 => {
                    app.channel_picker = None;
                    app.input(Field::Name, "Name", "test".into());
                }
                _ => {
                    app.input = None;
                    app.confirm_quit = true;
                }
            }
            terminal.draw(|frame| app.draw(frame)).unwrap();
            assert!(
                terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .all(|cell| cell.fg == ratatui::style::Color::Reset
                        && cell.bg == ratatui::style::Color::Reset)
            );
        }
    }

    #[test]
    fn store_selection_adds_only_compatible_channel() {
        let mut app = app();
        cache_info(&mut app, "opencode");
        app.document.definition.base = "ubuntu@22.04".into();
        assert!(app.key(key(KeyCode::Enter)).is_err());
        assert!(app.document.definition.sdks.is_empty());
        app.document.definition.base = "ubuntu@24.04".into();
        app.key(key(KeyCode::Enter)).unwrap();
        assert_eq!(app.document.definition.sdks[0].name, "opencode");
        assert_eq!(
            app.document.definition.sdks[0].channel.as_deref(),
            Some("latest/stable")
        );
        assert!(app.channel_picker.is_none());
    }

    fn cache_info(app: &mut App, name: &str) {
        let mut info: StoreInfo = serde_json::from_str(r#"{
            "name":"placeholder",
            "metadata":{"summary":"SDK summary","description":"Full SDK description"},
            "channel-map":[
                {"channel":{"name":"latest/edge","platform":{"name":"ubuntu","channel":"24.04","architecture":"amd64"}},
                 "revision":{"version":"3.0","revision":30}},
                {"channel":{"name":"latest/stable","platform":{"name":"ubuntu","channel":"24.04","architecture":"amd64"}},
                 "revision":{"version":"2.0","revision":20}}
            ]
        }"#).unwrap();
        info.name = name.to_owned();
        app.details.insert(name.to_owned(), Ok(info));
    }

    #[test]
    fn selects_a_combination_and_preserves_it_across_store_views() {
        let mut app = app();
        cache_info(&mut app, "opencode");
        cache_info(&mut app, "pi");
        app.key(key(KeyCode::Char(' '))).unwrap();
        app.key(key(KeyCode::Down)).unwrap();
        app.key(key(KeyCode::Enter)).unwrap();
        let names: Vec<_> = app
            .document
            .definition
            .sdks
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(names, ["opencode", "pi"]);
        assert!(
            app.document
                .definition
                .sdks
                .iter()
                .all(|s| s.channel.as_deref() == Some("latest/stable"))
        );
        app.tx
            .send(Message::Search(Tab::Store, Ok(vec![])))
            .unwrap();
        app.receive();
        app.key(key(KeyCode::Char('f'))).unwrap();
        assert_eq!(app.document.definition.sdks.len(), 2);
        app.key(key(KeyCode::Char(' '))).unwrap();
        assert_eq!(app.document.definition.sdks.len(), 1);
        assert_eq!(app.document.definition.sdks[0].name, "pi");
        assert!(app.channel_picker.is_none());
    }

    #[test]
    fn optional_channel_picker_starts_on_default_and_preserves_sdk_configuration() {
        let mut app = app();
        cache_info(&mut app, "opencode");
        app.key(key(KeyCode::Char('c'))).unwrap();
        assert_eq!(
            app.channels[app.channel_selected].channel.name,
            "latest/stable"
        );
        app.key(key(KeyCode::Esc)).unwrap();
        assert!(app.document.definition.sdks.is_empty());
        app.key(key(KeyCode::Char(' '))).unwrap();
        app.document.definition.sdks[0].plugs =
            serde_yaml::from_str("cache: {interface: mount}").unwrap();
        app.key(key(KeyCode::Char('c'))).unwrap();
        app.key(key(KeyCode::Down)).unwrap();
        app.key(key(KeyCode::Enter)).unwrap();
        assert_eq!(app.document.definition.sdks.len(), 1);
        assert_eq!(
            app.document.definition.sdks[0].channel.as_deref(),
            Some("latest/edge")
        );
        assert!(!app.document.definition.sdks[0].plugs.is_empty());
        app.key(key(KeyCode::Char('c'))).unwrap();
        assert_eq!(
            app.channels[app.channel_selected].channel.name,
            "latest/edge"
        );
    }

    #[test]
    fn late_metadata_does_not_move_selection_or_clear_search_busy_state() {
        let mut app = app();
        cache_info(&mut app, "opencode");
        let info = app.details.remove("opencode").unwrap().unwrap();
        app.details_pending = Some("opencode".into());
        app.catalog.store_selected = 1;
        app.catalog.busy = true;
        app.tx
            .send(Message::Info("opencode".into(), Ok(info)))
            .unwrap();
        app.receive();
        assert_eq!(app.catalog.store_selected, 1);
        assert!(app.catalog.busy);
        assert!(app.channel_picker.is_none());
        assert!(app.details.contains_key("opencode"));
    }

    #[test]
    fn highlighted_sdk_shows_description_default_channel_and_current_version() {
        let mut app = app();
        cache_info(&mut app, "opencode");
        let mut terminal = Terminal::new(TestBackend::new(160, 35)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        for expected in [
            "Full SDK description",
            "Default channel: latest/stable",
            "2.0",
            "20",
        ] {
            assert!(text.contains(expected), "Missing {expected}");
        }
        assert!(!text.contains("Default channel: latest/edge"));
        assert!(app.channel_picker.is_none());
    }
}
